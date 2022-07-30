use crate::kaspad::{KaspadHandle, RpcBlock};
use crate::stratum::{Id, Request, Response};
use crate::U256;
use anyhow::Result;
use log::{debug, info, warn};
use serde::Serialize;
use serde_json::json;
use std::num::Wrapping;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpListener;
use tokio::sync::{watch, RwLock};

const NEW_LINE: &'static str = "\n";

struct StratumTask {
    listener: TcpListener,
    recv: watch::Receiver<Option<JobParams>>,
    jobs: Jobs,
}

impl StratumTask {
    async fn run(self) {
        let mut worker = Wrapping(0u16);
        loop {
            worker += 1u16;
            if worker.0 == 0 {
                worker += 1u16;
            }

            match self.listener.accept().await {
                Ok((mut conn, addr)) => {
                    info!("New connection from {addr}");
                    let recv = self.recv.clone();
                    let jobs = self.jobs.clone();
                    let worker = worker.0.to_be_bytes();

                    tokio::spawn(async move {
                        let (reader, writer) = conn.split();
                        let conn = StratumConn {
                            // addr,
                            reader: BufReader::new(reader).lines(),
                            writer,
                            recv,
                            jobs,
                            worker,
                            id: 0,
                            subscribed: false,
                            difficulty: 0,
                        };

                        match conn.run().await {
                            Ok(_) => info!("Connection {addr} closed"),
                            Err(e) => warn!("Connection {addr} closed: {e}"),
                        }
                    });
                }
                Err(e) => {
                    warn!("Error: {e}");
                }
            }
        }
    }
}

pub struct Stratum {
    send: watch::Sender<Option<JobParams>>,
    jobs: Jobs,
}

impl Stratum {
    pub async fn new(host: &str, handle: KaspadHandle) -> Result<Self> {
        let (send, recv) = watch::channel(None);
        let listener = TcpListener::bind(host).await?;
        info!("Listening on {host}");

        let jobs = Jobs::new(handle);
        let task = StratumTask {
            listener,
            recv,
            jobs: jobs.clone(),
        };
        tokio::spawn(task.run());
        Ok(Stratum { send, jobs })
    }

    pub async fn broadcast(&self, template: RpcBlock) {
        if let Some(job) = self.jobs.insert(template).await {
            let _ = self.send.send(Some(job));
        }
    }
}

struct StratumConn<'a> {
    // addr: SocketAddr,
    reader: Lines<BufReader<ReadHalf<'a>>>,
    writer: WriteHalf<'a>,
    recv: watch::Receiver<Option<JobParams>>,
    jobs: Jobs,
    worker: [u8; 2],
    id: u64,
    subscribed: bool,
    difficulty: u64,
}

impl<'a> StratumConn<'a> {
    async fn write_template(&mut self) -> Result<()> {
        debug!("Sending template");
        let (difficulty, params) = {
            let borrow = self.recv.borrow();
            match borrow.as_ref() {
                Some(j) => (j.difficulty, j.to_value()),
                None => return Ok(()),
            }
        };
        self.write_request("mining.notify", Some(params)).await?;

        if self.difficulty == 0 {
            self.difficulty = difficulty >> 32;
            self.write_request("mining.set_difficulty", Some(json!([self.difficulty])))
                .await?;
        }

        Ok(())
    }

    async fn write_request(
        &mut self,
        method: &'static str,
        params: Option<serde_json::Value>,
    ) -> Result<()> {
        self.id += 1;
        let req = Request {
            id: Some(self.id.into()),
            method: method.into(),
            params,
        };
        self.write(&req).await
    }

    async fn write_response<T: Serialize>(&mut self, id: Id, result: Option<T>) -> Result<()> {
        let res = Response::ok(id, result)?;
        self.write(&res).await
    }

    #[allow(dead_code)]
    async fn write_error_response(&mut self, id: Id, code: u64, message: String) -> Result<()> {
        let res = Response::err(id, code, message)?;
        self.write(&res).await
    }

    async fn write<T: Serialize>(&mut self, data: &T) -> Result<()> {
        let data = serde_json::to_vec(data)?;
        self.writer.write_all(&data).await?;
        self.writer.write_all(NEW_LINE.as_ref()).await?;
        Ok(())
    }

    async fn run(mut self) -> Result<()> {
        loop {
            tokio::select! {
                res = self.recv.changed() => match res {
                    Err(_) => {
                        // Shutdown
                        break;
                    }
                    Ok(_) => {
                        if self.subscribed {
                            self.write_template().await?;
                        }
                    }
                },
                res = read(&mut self.reader) => match res {
                    Ok(Some(msg)) => {
                        match (msg.id, &*msg.method, msg.params) {
                            (Some(id), "mining.subscribe", _) => {
                                debug!("Worker subscribed");
                                self.subscribed = true;
                                self.write_response(id, Some(true)).await?;

                                self.write_request(
                                    "set_extranonce",
                                    Some(json!([hex::encode(&self.worker), 6u64]))
                                ).await?;
                                self.write_template().await?;
                            }
                            (Some(i), "mining.submit", Some(p)) => {
                                let (_, id, nonce): (String, String, String) = serde_json::from_value(p)?;
                                let id = u8::from_str_radix(&id, 16)?;
                                let nonce = u64::from_str_radix(nonce.trim_start_matches("0x"), 16)?;
                                if self.jobs.submit(id, nonce).await {
                                    debug!("Submit new block");
                                    self.write_response(i, Some(true)).await?;
                                }
                                else {
                                    debug!("Unable to submit new block");
                                    self.write_error_response(i, 20, "Unable to submit block".into()).await?;
                                }
                            }
                            _ => {
                                debug!("Got unknown {}", msg.method);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => return Err(e),
                },
            }
        }
        Ok(())
    }
}

async fn read(r: &mut Lines<BufReader<ReadHalf<'_>>>) -> Result<Option<Request>> {
    let line = match r.next_line().await? {
        Some(l) => l,
        None => return Ok(None),
    };
    Ok(Some(serde_json::from_str(&line)?))
}

#[derive(Clone)]
struct Jobs {
    inner: Arc<RwLock<JobsInner>>,
}

impl Jobs {
    fn new(handle: KaspadHandle) -> Self {
        Self {
            inner: Arc::new(RwLock::new(JobsInner {
                next: 0,
                jobs: Vec::with_capacity(256),
                handle,
            })),
        }
    }

    async fn insert(&self, template: RpcBlock) -> Option<JobParams> {
        let header = template.header.as_ref()?;
        let pre_pow = header.pre_pow().ok()?;
        let difficulty = header.difficulty();
        let timestamp = header.timestamp as u64;

        let mut w = self.inner.write().await;
        let len = w.jobs.len();
        let id = if len < 256 {
            w.jobs.push(template);
            len as u8
        } else {
            w.next
        };
        w.next = id.wrapping_add(1);

        Some(JobParams {
            id,
            pre_pow,
            difficulty,
            timestamp,
        })
    }

    async fn submit(&self, id: u8, nonce: u64) -> bool {
        let (mut block, handle) = {
            let r = self.inner.read().await;
            let block = match r.jobs.get(id as usize) {
                Some(b) => b.clone(),
                None => return false,
            };
            (block, r.handle.clone())
        };
        if let Some(header) = &mut block.header {
            header.nonce = nonce;
            handle.submit_block(block);
            return true;
        }
        false
    }
}

struct JobsInner {
    next: u8,
    handle: KaspadHandle,
    jobs: Vec<RpcBlock>,
}

struct JobParams {
    id: u8,
    pre_pow: U256,
    difficulty: u64,
    timestamp: u64,
}

impl JobParams {
    fn to_value(&self) -> serde_json::Value {
        json!([
            hex::encode([self.id]),
            self.pre_pow.as_slice(),
            self.timestamp
        ])
    }
}
