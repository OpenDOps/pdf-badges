use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use crate::registration_server::gate::Gate;
use crate::registration_server::ServeError;

/// Jobs waiting on the server thread. One worker drains the queue.
pub const PRINT_QUEUE_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrintJob {
    pub id: String,
    pub pages: u32,
}

#[derive(Clone)]
pub struct PrintQueue {
    tx: mpsc::Sender<PrintJob>,
}

pub struct PrintWorker {
    rx: mpsc::Receiver<PrintJob>,
    gate: Arc<Gate>,
}

impl PrintWorker {
    pub(crate) fn try_recv(&mut self) -> Option<PrintJob> {
        self.rx.try_recv().ok()
    }
}

#[derive(Debug)]
pub enum EnqueueError {
    EmptyId,
    NoPages,
    Full,
    Closed,
}

impl PrintQueue {
    pub fn pair(gate: Arc<Gate>) -> (Self, PrintWorker) {
        let (tx, rx) = mpsc::channel(PRINT_QUEUE_DEPTH);
        (Self { tx }, PrintWorker { rx, gate })
    }

    pub fn enqueue(&self, job: PrintJob) -> Result<(), EnqueueError> {
        if job.id.is_empty() {
            return Err(EnqueueError::EmptyId);
        }
        if job.pages == 0 {
            return Err(EnqueueError::NoPages);
        }
        self.tx.try_send(job).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => EnqueueError::Full,
            mpsc::error::TrySendError::Closed(_) => EnqueueError::Closed,
        })
    }
}

pub async fn run(
    mut worker: PrintWorker,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), ServeError> {
    loop {
        tokio::select! {
            job = worker.rx.recv() => {
                let Some(job) = job else {
                    return Ok(());
                };
                let _hold = worker.gate.enter_print();
                let mut page = 0u32;
                while page < job.pages {
                    page += 1;
                    // Page rendering replaces this yield. The await is the
                    // point where the accept loop can take a local request.
                    tokio::task::yield_now().await;
                }
            }
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
        }
    }
}
