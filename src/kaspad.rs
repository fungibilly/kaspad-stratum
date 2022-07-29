use anyhow::Result;
use log::{debug, info, warn};
use proto::kaspad_message::Payload;
use proto::submit_block_response_message::RejectReason;
pub use proto::RpcBlock;
use proto::*;
use rpc_client::RpcClient;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

pub type Send<T> = mpsc::UnboundedSender<T>;
type Recv<T> = mpsc::UnboundedReceiver<T>;

pub struct U256([u64; 4]);

impl U256 {
    pub fn as_slice(&self) -> &[u64] {
        &self.0
    }
}

impl From<[u64; 4]> for U256 {
    fn from(v: [u64; 4]) -> Self {
        U256(v)
    }
}

#[derive(Clone)]
pub struct KaspadHandle(Send<Payload>);

impl KaspadHandle {
    pub fn new() -> (Self, Recv<Payload>) {
        let (send, recv) = mpsc::unbounded_channel();
        (KaspadHandle(send), recv)
    }

    pub fn submit_block(&self, block: RpcBlock) {
        info!("Submitting block");
        let _ = self.0.send(Payload::submit_block(block, false));
    }
}

#[derive(Debug)]
pub enum Message {
    Info { version: String, synced: bool },
    Template(RpcBlock),
    NewTemplate,
}

struct ClientTask {
    url: String,
    send_msg: Send<Message>,
    recv_cmd: Recv<Payload>,
    synced: bool,
}

impl ClientTask {
    async fn run(mut self) -> Result<()> {
        let mut client = RpcClient::connect(self.url).await?;
        let mut stream = client
            .message_stream(
                UnboundedReceiverStream::new(self.recv_cmd)
                    .map(|p| KaspadMessage { payload: Some(p) }),
            )
            .await?
            .into_inner();

        while let Some(KaspadMessage { payload }) = stream.message().await? {
            let msg = match payload {
                Some(Payload::GetInfoResponse(info)) => {
                    self.synced = info.is_synced;
                    if !self.synced {
                        warn!("Not yet synced");
                    }
                    Message::Info {
                        version: info.server_version,
                        synced: info.is_synced,
                    }
                }
                Some(Payload::SubmitBlockResponse(res)) => {
                    match (RejectReason::from_i32(res.reject_reason), res.error) {
                        (Some(RejectReason::None), None) => {
                            info!("Submitted block successfully");
                        }
                        (_, Some(e)) => {
                            warn!("Unable to submit block: {}", e.message);
                        }
                        _ => {
                            warn!("Unable to submit block");
                        }
                    }
                    continue;
                }
                Some(Payload::GetBlockTemplateResponse(res)) => {
                    if let Some(e) = res.error {
                        warn!("Error: {}", e.message);
                        continue;
                    }
                    if let Some(block) = res.block {
                        if !self.synced && res.is_synced {
                            info!("Node synced");
                        }
                        self.synced = res.is_synced;

                        if block.header.is_none() {
                            warn!("Template block is missing a header");
                            continue;
                        }
                        Message::Template(block)
                    } else {
                        continue;
                    }
                }
                Some(Payload::NewBlockTemplateNotification(_)) => Message::NewTemplate,
                Some(Payload::NotifyNewBlockTemplateResponse(res)) => match res.error {
                    Some(e) => {
                        warn!("Unable to subscribe to new templates: {}", e.message);
                        break;
                    }
                    None => {
                        debug!("Subscribed to new templates");
                        continue;
                    }
                },
                _ => {
                    debug!("Received unknown message");
                    continue;
                }
            };
            self.send_msg.send(msg)?;
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct Client {
    pay_address: String,
    extra_data: String,
    send_cmd: Send<Payload>,
}

impl Client {
    pub fn new(
        url: &str,
        pay_address: &str,
        extra_data: &str,
        handle: KaspadHandle,
        recv_cmd: Recv<Payload>,
    ) -> (Self, Recv<Message>) {
        let (send_msg, recv_msg) = mpsc::unbounded_channel();
        let task = ClientTask {
            url: url.into(),
            send_msg,
            recv_cmd,
            synced: false,
        };

        tokio::spawn(async move {
            match task.run().await {
                Ok(_) => warn!("Kaspad connection closed"),
                Err(e) => warn!("Kaspad connection closed: {e}"),
            }
        });

        let send_cmd = handle.0;
        send_cmd.send(Payload::get_info()).unwrap();
        send_cmd.send(Payload::notify_new_block_template()).unwrap();

        let client = Client {
            pay_address: pay_address.into(),
            extra_data: extra_data.into(),
            send_cmd,
        };
        client.request_template();
        (client, recv_msg)
    }

    pub fn request_template(&self) -> bool {
        self.send_cmd
            .send(Payload::get_block_template(
                &self.pay_address,
                &self.extra_data,
            ))
            .is_ok()
    }
}

mod proto {
    use crate::U256;
    use anyhow::Result;
    use blake2b_simd::Hash;
    use kaspad_message::Payload;

    include!(concat!(env!("OUT_DIR"), "/protowire.rs"));

    impl Payload {
        pub fn get_info() -> Self {
            Payload::GetInfoRequest(GetInfoRequestMessage {})
        }

        pub fn submit_block(block: RpcBlock, allow_non_daa_blocks: bool) -> Self {
            Payload::SubmitBlockRequest(SubmitBlockRequestMessage {
                block: Some(block),
                allow_non_daa_blocks,
            })
        }

        pub fn get_block_template(pay_address: &str, extra_data: &str) -> Self {
            Payload::GetBlockTemplateRequest(GetBlockTemplateRequestMessage {
                pay_address: pay_address.into(),
                extra_data: extra_data.into(),
            })
        }

        pub fn notify_new_block_template() -> Self {
            Payload::NotifyNewBlockTemplateRequest(super::NotifyNewBlockTemplateRequestMessage {})
        }
    }

    impl RpcBlockHeader {
        pub fn pre_pow(&self) -> Result<U256> {
            let hash = self.hash(true)?;
            let mut out = [0; 4];
            for (o, c) in out.iter_mut().zip(hash.as_bytes().chunks_exact(8)) {
                *o = u64::from_le_bytes(c.try_into().unwrap());
            }
            Ok(out.into())
        }

        pub fn hash(&self, pre_pow: bool) -> Result<Hash> {
            let mut state = blake2b_simd::Params::new()
                .hash_length(32)
                .key(b"BlockHash")
                .to_state();

            let version = self.version as u16;
            state.update(&version.to_le_bytes());
            let mut parents = self.parents.len() as u64;
            state.update(&parents.to_le_bytes());

            let mut hash = [0u8; 32];
            for parent in &self.parents {
                parents = parent.parent_hashes.len() as u64;
                state.update(&parents.to_le_bytes());
                for h in &parent.parent_hashes {
                    hex::decode_to_slice(h, &mut hash)?;
                    state.update(&hash);
                }
            }
            hex::decode_to_slice(&self.hash_merkle_root, &mut hash)?;
            state.update(&hash);
            hex::decode_to_slice(&self.accepted_id_merkle_root, &mut hash)?;
            state.update(&hash);
            hex::decode_to_slice(&self.utxo_commitment, &mut hash)?;
            state.update(&hash);

            let (timestamp, nonce) = if pre_pow {
                (0, 0)
            } else {
                (self.timestamp, self.nonce)
            };

            state
                .update(&timestamp.to_le_bytes())
                .update(&self.bits.to_le_bytes())
                .update(&nonce.to_le_bytes())
                .update(&self.daa_score.to_le_bytes())
                .update(&self.blue_score.to_le_bytes());

            let len = (self.blue_work.len() + 1) / 2;
            if self.blue_work.len() % 2 == 0 {
                hex::decode_to_slice(&self.blue_work, &mut hash[..len])?;
            } else {
                hex::decode_to_slice(format!("0{}", self.blue_work), &mut hash[..len])?;
            }
            state
                .update(&(len as u64).to_le_bytes())
                .update(&hash[..len]);

            hex::decode_to_slice(&self.pruning_point, &mut hash)?;
            state.update(&hash);

            Ok(state.finalize())
        }
    }
}

#[cfg(test)]
mod test {
    use super::{RpcBlockHeader, RpcBlockLevelParents};
    use blake2b_simd::Hash;

    #[test]
    fn header_hash() {
        // From kaspa-miner
        let header = RpcBlockHeader {
            version: 24565,
            parents: vec![
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "62a5eee82abdf44a2d0b75fb180daf48a79ee0b10d394651850fd4a178892ee2".into(),
                        "85ece1511455780875d64ee2d3d0d0de6bf8f9b44ce85ff044c6b1f83b8e883b".into(),
                        "bf857aab99c5b252c7429c32f3a8aeb79ef856f659c18f0dcecc77c75e7a81bf".into(),
                        "de275f67cfe242cf3cc354f3ede2d6becc4ea3ae5e88526a9f4a578bcb9ef2d4".into(),
                        "a65314768d6d299761ea9e4f5aa6aec3fc78c6aae081ac8120c720efcd6cea84".into(),
                        "b6925e607be063716f96ddcdd01d75045c3f000f8a796bce6c512c3801aacaee".into(),
                    ],
                },
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "dfad5b50ece0b8b7c1965d9181251b7c9c9ca5205afc16a236a2efcdd2d12d2a".into(),
                        "79d074a8280ae9439eb0d6aeca0823ae02d67d866ac2c4fe4a725053da119b9d".into(),
                        "4f515140a2d7239c40b45ac3950d941fc4fe1c0cb96ad322d62282295fbfe11e".into(),
                        "26a433076db5c1444c3a34d32a5c4a7ffbe8d181f7ed3b8cfe904f93f8f06d29".into(),
                        "bcd9ed847b182e046410f44bc4b0f3f03a0d06820a30f257f8114130678ac045".into(),
                        "86c1e3c9342c8b8055c466d886441d259906d69acd894b968ae9f0eb9d965ce6".into(),
                        "a4693c4ebe881501b7d9846b66eb02b57e5cda7b6cba6891d616bd686c37b834".into(),
                    ],
                },
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "613ac8ba52734ae4e3f1217acd5f83270814301867b5d06711b238001c7957b2".into(),
                        "7719ce3f3188dfe57deebf6f82595a10f7bb562ca04d5c3d27942958c6db3262".into(),
                        "670649f3bc97d9a2316735ede682a5dfe6f1a011fbc98ad0fbe790003c01e8e9".into(),
                        "967703af665e9f72407f4b03d4fdb474aafe8a0d3e0515dd4650cf51172b8124".into(),
                        "8bcb7f969e400b6c5b127768b1c412fae98cf57631cf37033b4b4aba7d7ed319".into(),
                        "ba147249c908ac70d1c406dade0e828eb6ba0dcaa88285543e10213c643fc860".into(),
                        "3b5860236670babcad0bd7f4c4190e323623a868d1eae1769f40a26631431b3b".into(),
                        "d5215605d2086fead499ac63a4653d12283d56019c3795a98a126d09cfcbe36c".into(),
                        "dcc93788a5409f8b6e42c2dd83aa46611852ad0b5028775c771690b6854e05b3".into(),
                    ],
                },
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "77241e302c6da8665c42341dda4adaea595ab1895f9652489dd2ceb49c247430".into(),
                        "3cbb44c2b94303db662c9c66b8782905190f1e1635b63e34878d3f246fadfce3".into(),
                        "44e74ef813090f8030bcd525ac10653ff182e00120f7e1f796fa0fc16ba7bb90".into(),
                        "be2a33e87c3d60ab628471a420834383661801bb0bfd8e6c140071db1eb2f7a1".into(),
                        "8194f1a045a94c078835c75dff2f3e836180baad9e955da840dc74c4dc2498f8".into(),
                        "c201aec254a0e36476b2eeb124fdc6afc1b7d809c5e08b5e0e845aaf9b6c3957".into(),
                        "e95ab4aa8e107cdb873f2dac527f16c4d5ac8760768a715e4669cb840c25317f".into(),
                        "9a368774e506341afb46503e28e92e51bd7f7d4b53b9023d56f9b9ec991ac2a9".into(),
                        "d9bc45ff64bb2bf14d4051a7604b28bad44d98bfe30e54ebc07fa45f62aabe39".into(),
                    ],
                },
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "5cc98b2e3f6deb2990187058e4bfd2d1640653fc38a30b0f83231a965b413b0f".into(),
                        "26927e0d032e830b732bdeb3094cb1a5fa6dec9f06375ea25fe57c2853ea0932".into(),
                        "0ac8803976eacaa095c02f869fd7dc31072475940c3751d56283c49e2fefd41d".into(),
                        "f676bdcb5855a0470efd2dab7a72cc5e5f39ff7eea0f433a9fe7b6a675bc2ac5".into(),
                        "0cd218c009e21f910f9ddb09a0d059c4cd7d2ca65a2349df7a867dbedd81e9d4".into(),
                        "891619c83c42895ce1b671cb7a4bcaed9130ab1dd4cc2d8147a1595056b55f92".into(),
                        "a355db765adc8d3df88eb93d527f7f7ec869a75703ba86d4b36110e9a044593c".into(),
                        "966815d153665387dc38e507e7458df3e6b0f04035ef9419883e03c08e2d753b".into(),
                        "08c9090aabf175fdb63e8cf9a5f0783704c741c195157626401d949eaa6dbd04".into(),
                    ],
                },
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "d7bf5e9c18cc79dda4e12efe564ecb8a4019e1c41f2d8217c0c3a43712ae226f".into(),
                        "ce776631ae19b326a411a284741be01fb4f3aefc5def968eb6cceb8604864b4b".into(),
                        "9ad373cbac10ea7e665b294a8a790691aa5246e6ff8fd0b7fb9b9a6a958ebf28".into(),
                    ],
                },
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "ec5e8fc0bc0c637004cee262cef12e7cf6d9cd7772513dbd466176a07ab7c4f4".into(),
                        "65fe09747779c31495e689b65f557b0a4af6535880b82553d126ff7213542905".into(),
                        "5a64749599333e9655b43aa36728bb63bd286427441baa9f305d5c25e05229bb".into(),
                        "332f7e8375b7c45e1ea0461d333c3c725f7467b441b7d0f5e80242b7a4a18eda".into(),
                    ],
                },
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "e80d7d9a0a4634f07bea5c5a00212fbc591bddfebb94334f4a2d928673d262ad".into(),
                    ],
                },
                RpcBlockLevelParents {
                    parent_hashes: vec![
                        "abaa82988c683f4742c12099b732bd03639c1979752d837518243b74d6730124".into(),
                        "5efe5661eaa0428917f55a58cc33db284d1f2caa05f1fd7b6602980f06d10723".into(),
                        "0bf310b48cf62942017dd6680eb3ab13310eca1581afb3c5b619e5ce0682d0df".into(),
                        "c1fade3928179a9dc28cd170b5b5544e7f9b63b83da374afa28e1478dc5c2997".into(),
                    ],
                },
            ],
            hash_merkle_root: "a98347ec1e71514eb26822162dc7c3992fd41f0b2ccc26e55e7bd8f3fa37215f"
                .into(),
            accepted_id_merkle_root:
                "774b5216b5b872b6c2388dd950160e3ffa3bf0623c438655bb5c8c768ab33ae2".into(),
            utxo_commitment: "ee39218674008665e20a3acdf84abef35cabcc489158c0853fd5bfa954226139"
                .into(),
            timestamp: -1426594953012613626,
            bits: 684408190,
            nonce: 8230160685758639177,
            daa_score: 15448880227546599629,
            blue_work: "ce5639b8ed46571e20eeaa7a62a078f8c55aef6edd6a35ed37a3d6cf98736abd".into(),
            pruning_point: "fc44c4f57cf8f7a2ba410a70d0ad49060355b9deb97012345603d9d0d1dcb0de"
                .into(),
            blue_score: 29372123613087746,
        };

        let expected_hash = [
            85, 146, 211, 217, 138, 239, 47, 85, 152, 59, 58, 16, 4, 149, 129, 179, 172, 226, 174,
            233, 160, 96, 202, 54, 6, 225, 64, 142, 106, 0, 110, 137,
        ];

        assert_eq!(header.hash(true).unwrap().as_bytes(), &expected_hash);
    }
}
