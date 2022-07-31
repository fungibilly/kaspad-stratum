use super::jobs::{JobParams, Jobs, PendingResult};
use super::{Id, Request, Response};
use crate::kaspad::{KaspadHandle, RpcBlock};
use anyhow::Result;
use log::{debug, info, warn};
use serde::Serialize;
use serde_json::json;
use std::num::Wrapping;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};

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
            worker += &1;
            if worker.0 == 0 {
                worker += &1;
            }

            match self.listener.accept().await {
                Ok((mut conn, addr)) => {
                    info!("New connection from {addr}");
                    let recv = self.recv.clone();
                    let jobs = self.jobs.clone();
                    let worker = worker.0.to_be_bytes();
                    let (pending_send, pending_recv) = mpsc::unbounded_channel();

                    tokio::spawn(async move {
                        let (reader, writer) = conn.split();
                        let conn = StratumConn {
                            // addr,
                            reader: BufReader::new(reader).lines(),
                            writer,
                            recv,
                            jobs,
                            pending_send,
                            pending_recv,
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

    pub async fn resolve_pending_job(&self, error: Option<Box<str>>) {
        self.jobs.resolve_pending(error).await
    }
}

struct StratumConn<'a> {
    // addr: SocketAddr,
    reader: Lines<BufReader<ReadHalf<'a>>>,
    writer: WriteHalf<'a>,
    recv: watch::Receiver<Option<JobParams>>,
    jobs: Jobs,
    pending_send: mpsc::UnboundedSender<PendingResult>,
    pending_recv: mpsc::UnboundedReceiver<PendingResult>,
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
                Some(j) => (j.difficulty(), j.to_value()),
                None => return Ok(()),
            }
        };
        self.write_request("mining.notify", Some(params)).await?;

        if self.difficulty != difficulty {
            self.difficulty = difficulty;
            let difficulty = (difficulty as f64) / ((1u64 << 32) as f64);
            self.write_request("mining.set_difficulty", Some(json!([difficulty])))
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

    async fn write_error_response(&mut self, id: Id, code: u64, message: Box<str>) -> Result<()> {
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
                item = self.pending_recv.recv() => {
                    let res = item.expect("channel is always open").into_response()?;
                    self.write(&res).await?;
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
                                if self.jobs.submit(i.clone(), id, nonce, self.pending_send.clone()).await {
                                    debug!("Submit new block");
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
