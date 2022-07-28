use anyhow::Result;
use log::{debug, warn};
use proto::kaspad_message::Payload;
pub use proto::RpcBlockHeader;
use proto::*;
use rpc_client::RpcClient;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

type Send<T> = mpsc::UnboundedSender<T>;
type Recv<T> = mpsc::UnboundedReceiver<T>;

#[derive(Debug)]
pub enum Message {
    Info { version: String, synced: bool },
    Template(RpcBlockHeader),
    NewBlock,
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
                Some(Payload::GetBlockTemplateResponse(res)) => {
                    if let Some(e) = res.error {
                        warn!("Error: {}", e.message);
                        continue;
                    }
                    if let Some(block) = res.block {
                        match block.header {
                            Some(header) => Message::Template(header),
                            None => {
                                warn!("Template block is missing a header");
                                continue;
                            }
                        }
                    } else {
                        continue;
                    }
                }
                Some(Payload::BlockAddedNotification(_)) => Message::NewBlock,
                Some(Payload::NotifyBlockAddedResponse(res)) => match res.error {
                    Some(e) => {
                        warn!("Unable to subscribe to new blocks: {}", e.message);
                        break;
                    }
                    None => {
                        debug!("Subscribed to new blocks");
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
    send_cmd: Send<Payload>,
}

impl Client {
    pub fn new(url: &str, pay_address: &str) -> (Self, Recv<Message>) {
        let (send_msg, recv_msg) = mpsc::unbounded_channel();
        let (send_cmd, recv_cmd) = mpsc::unbounded_channel();
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

        send_cmd
            .send(Payload::GetInfoRequest(GetInfoRequestMessage {}))
            .unwrap();
        send_cmd
            .send(Payload::NotifyBlockAddedRequest(
                NotifyBlockAddedRequestMessage {},
            ))
            .unwrap();

        let client = Client {
            pay_address: pay_address.into(),
            send_cmd,
        };
        client.request_template();
        (client, recv_msg)
    }

    pub fn request_template(&self) -> bool {
        self.send_cmd
            .send(Payload::GetBlockTemplateRequest(
                GetBlockTemplateRequestMessage {
                    pay_address: self.pay_address.clone(),
                    extra_data: String::new(),
                },
            ))
            .is_ok()
    }
}

mod proto {
    include!(concat!(env!("OUT_DIR"), "/protowire.rs"));
}
