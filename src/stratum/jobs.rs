use super::{Id, Response};
use crate::kaspad::{KaspadHandle, RpcBlock};
use crate::U256;
use anyhow::Result;
use log::debug;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};

#[derive(Clone)]
pub struct Jobs {
    inner: Arc<RwLock<JobsInner>>,
    pending: Arc<Mutex<VecDeque<Pending>>>,
}

impl Jobs {
    pub fn new(handle: KaspadHandle) -> Self {
        Self {
            inner: Arc::new(RwLock::new(JobsInner {
                next: 0,
                jobs: Vec::with_capacity(256),
                handle,
            })),
            pending: Arc::new(Mutex::new(VecDeque::with_capacity(64))),
        }
    }

    pub async fn insert(&self, template: RpcBlock) -> Option<JobParams> {
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

    pub async fn submit(
        &self,
        rpc_id: Id,
        job_id: u8,
        nonce: u64,
        send: mpsc::UnboundedSender<PendingResult>,
    ) -> bool {
        let (mut block, handle) = {
            let r = self.inner.read().await;
            let block = match r.jobs.get(job_id as usize) {
                Some(b) => b.clone(),
                None => return false,
            };
            (block, r.handle.clone())
        };
        if let Some(header) = &mut block.header {
            {
                // Keep the lock on the pending jobs while we submit the block
                // to guarantee that the ordering matches up
                let mut pending = self.pending.lock().await;
                pending.push_back(Pending { id: rpc_id, send });

                header.nonce = nonce;
                handle.submit_block(block);
            }

            true
        } else {
            false
        }
    }

    pub async fn resolve_pending(&self, error: Option<Box<str>>) {
        if let Some(pending) = self.pending.lock().await.pop_front() {
            pending.resolve(error);
        } else {
            debug!("Resolve: nothing is pending");
        }
    }
}

struct JobsInner {
    next: u8,
    handle: KaspadHandle,
    jobs: Vec<RpcBlock>,
}

pub struct JobParams {
    id: u8,
    pre_pow: U256,
    difficulty: u64,
    timestamp: u64,
}

impl JobParams {
    pub fn difficulty(&self) -> u64 {
        self.difficulty
    }

    pub fn to_value(&self) -> serde_json::Value {
        json!([
            hex::encode([self.id]),
            self.pre_pow.as_slice(),
            self.timestamp
        ])
    }
}

pub struct Pending {
    id: Id,
    send: mpsc::UnboundedSender<PendingResult>,
}

impl Pending {
    pub fn resolve(self, error: Option<Box<str>>) {
        let result = PendingResult { id: self.id, error };
        let _ = self.send.send(result);
    }
}

pub struct PendingResult {
    id: Id,
    error: Option<Box<str>>,
}

impl PendingResult {
    pub fn into_response(self) -> Result<Response> {
        match self.error {
            Some(e) => Response::err(self.id, 20, e),
            None => Response::ok(self.id, true),
        }
    }
}
