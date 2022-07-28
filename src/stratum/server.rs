use crate::kaspad::RpcBlockHeader;
use crate::stratum::{Id, OkResponse, Request, Response};
use anyhow::Result;
use log::{debug, info, warn};
use serde::Serialize;
// use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

const NEW_LINE: &'static str = "\n";

struct StratumTask {
    listener: TcpListener,
    recv: watch::Receiver<Option<RpcBlockHeader>>,
}

impl StratumTask {
    async fn run(self) {
        loop {
            match self.listener.accept().await {
                Ok((conn, addr)) => {
                    info!("New connection from {addr}");
                    let conn = StratumConn {
                        // addr,
                        conn,
                        recv: self.recv.clone(),
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
    send: watch::Sender<Option<RpcBlockHeader>>,
}

impl Stratum {
    pub async fn new(host: &str) -> Result<Self> {
        let (send, recv) = watch::channel(None);
        let listener = TcpListener::bind(host).await?;
        info!("Listening on {host}");
        let task = StratumTask { listener, recv };
        tokio::spawn(task.run());
        let stratum = Stratum { send };
        Ok(stratum)
    }

    pub fn broadcast(&self, template: RpcBlockHeader) {
        let _ = self.send.send(Some(template));
    }
}

struct StratumConn {
    // addr: SocketAddr,
    conn: TcpStream,
    recv: watch::Receiver<Option<RpcBlockHeader>>,
}

impl StratumConn {
    async fn run(mut self) -> Result<()> {
        let (reader, mut writer) = self.conn.split();
        let mut reader = BufReader::new(reader).lines();

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
                            if let Some(t) = template(&self.recv) {
                                debug!("Sending template");
                                write_req(&mut writer, t).await?;
                            }
                        }
                    }
                },
                res = read(&mut reader) => match res {
                    Ok(Some(msg)) => {
                        if &msg.method == "mining.subscribe" && msg.id.is_some(){
                            debug!("Miner subscribed");
                            subscribed = true;
                            write_res::<()>(&mut writer, msg.id.unwrap(), None).await?;

                            if let Some(t) = template(&self.recv) {
                                debug!("Sending template");
                                write_req(&mut writer, t).await?;
                            }
                        }
                        else {
                            debug!("Got {}", msg.method);
                        }
                    }
                    Ok(None) => return Ok(()),
                    Err(e) => return Err(e),
                },
            }
        }
        Ok(())
    }
}

fn template(recv: &watch::Receiver<Option<RpcBlockHeader>>) -> Option<Request> {
    let v = serde_json::to_value(vec![recv.borrow().as_ref()?]).ok()?;
    Some(Request {
        id: None,
        method: "mining.notify".into(),
        params: Some(v),
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
