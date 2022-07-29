use crate::kaspad::RpcBlock;
use crate::stratum::{Id, OkResponse, Request, Response};
use crate::U256;
use anyhow::Result;
use log::{debug, info, warn};
use serde::Serialize;
use serde_json::json;
use std::num::Wrapping;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, RwLock};

const NEW_LINE: &'static str = "\n";

struct StratumTask {
    listener: TcpListener,
    recv: watch::Receiver<Option<JobParams>>,
}

impl StratumTask {
    async fn run(self) {
        let mut worker = Wrapping(0u16);
        loop {
            worker += 1;
            if worker.0 == 0 {
                worker += 1;
            }

            match self.listener.accept().await {
                Ok((conn, addr)) => {
                    info!("New connection from {addr}");
                    let conn = StratumConn {
                        // addr,
                        conn,
                        recv: self.recv.clone(),
                        worker: worker.0.to_be_bytes(),
                    };
                    tokio::spawn(async move {
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
    pub async fn new(host: &str) -> Result<Self> {
        let (send, recv) = watch::channel(None);
        let listener = TcpListener::bind(host).await?;
        info!("Listening on {host}");
        let task = StratumTask { listener, recv };
        tokio::spawn(task.run());
        let stratum = Stratum {
            send,
            jobs: Jobs::new(),
        };
        Ok(stratum)
    }

    pub async fn broadcast(&self, template: RpcBlock) {
        if let Some(job) = self.jobs.insert(template).await {
            let _ = self.send.send(Some(job));
        }
    }
}

struct StratumConn {
    // addr: SocketAddr,
    conn: TcpStream,
    recv: watch::Receiver<Option<JobParams>>,
    worker: [u8; 2],
}

impl StratumConn {
    async fn run(mut self) -> Result<()> {
        let (reader, mut writer) = self.conn.split();
        let mut reader = BufReader::new(reader).lines();

        let mut id = 0u64;
        let mut subscribed = false;

        loop {
            tokio::select! {
                res = self.recv.changed() => match res {
                    Err(_) => {
                        // Shutdown
                        break;
                    }
                    Ok(_) => {
                        if subscribed {
                            id += 1;
                            if let Some(t) = template(&self.recv, id) {
                                debug!("Sending template");
                                write_req(&mut writer, t).await?;
                            }
                        }
                    }
                },
                res = read(&mut reader) => match res {
                    Ok(Some(msg)) => {
                        if &msg.method == "mining.subscribe" && msg.id.is_some() {
                            debug!("Miner subscribed");
                            subscribed = true;
                            write_res::<()>(&mut writer, msg.id.unwrap(), None).await?;
                            id += 1;
                            write_req(&mut writer, Request {
                                id: Some(id.into()),
                                method: "set_extranonce".into(),
                                params: Some(json!([hex::encode(&self.worker), 4]))
                            }).await?;

                            id += 1;
                            if let Some(t) = template(&self.recv, id) {
                                debug!("Sending template");
                                write_req(&mut writer, t).await?;
                            }
                        }
                        else {
                            debug!("Got unknown {}", msg.method);
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

fn template(recv: &watch::Receiver<Option<JobParams>>, id: u64) -> Option<Request> {
    Some(Request {
        id: Some(id.into()),
        method: "mining.notify".into(),
        params: Some(recv.borrow().as_ref()?.to_value()),
    })
}

async fn read(r: &mut Lines<BufReader<ReadHalf<'_>>>) -> Result<Option<Request>> {
    let line = match r.next_line().await? {
        Some(l) => l,
        None => return Ok(None),
    };
    Ok(Some(serde_json::from_str(&line)?))
}

async fn write_req(w: &mut WriteHalf<'_>, req: Request) -> Result<()> {
    let data = serde_json::to_vec(&req)?;
    w.write_all(&data).await?;
    w.write_all(NEW_LINE.as_ref()).await?;
    Ok(())
}

async fn write_res<T: Serialize>(w: &mut WriteHalf<'_>, id: Id, result: Option<T>) -> Result<()> {
    let res = Response::Ok(OkResponse { id, result });
    let data = serde_json::to_vec(&res)?;
    w.write_all(&data).await?;
    w.write_all(NEW_LINE.as_ref()).await?;
    Ok(())
}

#[derive(Clone)]
struct Jobs {
    inner: Arc<RwLock<JobsInner>>,
}

impl Jobs {
    fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(JobsInner {
                next: 0,
                jobs: Vec::with_capacity(256),
            })),
        }
    }

    async fn insert(&self, template: RpcBlock) -> Option<JobParams> {
        let header = template.header.as_ref()?;
        let pre_pow = header.pre_pow().ok()?;
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
            timestamp,
        })
    }
}

struct JobsInner {
    next: u8,
    jobs: Vec<RpcBlock>,
}

struct JobParams {
    id: u8,
    pre_pow: U256,
    timestamp: u64,
}

impl JobParams {
    fn to_value(&self) -> serde_json::Value {
        serde_json::json!([
            hex::encode([self.id]),
            self.pre_pow.as_slice(),
            self.timestamp
        ])
    }
}
