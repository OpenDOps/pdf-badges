use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

/// Closes while a local call is in flight or a print job is running.
/// The sync thread waits on this before it reads or parses.
pub struct Gate {
    inflight: AtomicUsize,
    printing: AtomicBool,
    idle: Notify,
}

pub struct RequestGuard {
    gate: Arc<Gate>,
}

pub struct PrintGuard {
    gate: Arc<Gate>,
}

impl Gate {
    pub fn new() -> Self {
        Self {
            inflight: AtomicUsize::new(0),
            printing: AtomicBool::new(false),
            idle: Notify::new(),
        }
    }

    pub fn enter_request(self: &Arc<Self>) -> RequestGuard {
        self.inflight.fetch_add(1, Ordering::AcqRel);
        RequestGuard {
            gate: Arc::clone(self),
        }
    }

    pub fn enter_print(self: &Arc<Self>) -> PrintGuard {
        let already = self.printing.swap(true, Ordering::AcqRel);
        debug_assert!(!already, "one print job at a time");
        PrintGuard {
            gate: Arc::clone(self),
        }
    }

    pub fn is_idle(&self) -> bool {
        self.inflight.load(Ordering::Acquire) == 0 && !self.printing.load(Ordering::Acquire)
    }

    pub fn is_printing(&self) -> bool {
        self.printing.load(Ordering::Acquire)
    }

    pub async fn wait_until_idle(&self) {
        self.wait_until(false).await;
    }

    /// Idle aside from the request that is calling. A print still waits.
    pub async fn wait_until_others_idle(&self) {
        self.wait_until(true).await;
    }

    async fn wait_until(&self, allow_own_request: bool) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            let requests = self.inflight.load(Ordering::Acquire);
            let quiet = if allow_own_request {
                requests <= 1
            } else {
                requests == 0
            };
            if quiet && !self.printing.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    fn wake(&self) {
        self.idle.notify_waiters();
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.gate.inflight.fetch_sub(1, Ordering::AcqRel);
        self.gate.wake();
    }
}

impl Drop for PrintGuard {
    fn drop(&mut self) {
        self.gate.printing.store(false, Ordering::Release);
        self.gate.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_holds_the_gate_until_it_returns() {
        let gate = Arc::new(Gate::new());
        let guard = gate.enter_request();
        let waiting = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move { gate.wait_until_idle().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(guard);
        waiting.await.unwrap();
        assert!(gate.is_idle());
    }

    #[tokio::test]
    async fn print_holds_the_gate_until_the_job_ends() {
        let gate = Arc::new(Gate::new());
        let guard = gate.enter_print();
        let waiting = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move { gate.wait_until_idle().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(guard);
        waiting.await.unwrap();
    }

    #[tokio::test]
    async fn the_calling_request_does_not_block_itself() {
        let gate = Arc::new(Gate::new());
        let own = gate.enter_request();
        gate.wait_until_others_idle().await;

        let other = gate.enter_request();
        let waiting = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move { gate.wait_until_others_idle().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(other);
        waiting.await.unwrap();

        let printing = gate.enter_print();
        let waiting = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move { gate.wait_until_others_idle().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(printing);
        waiting.await.unwrap();
        drop(own);
    }
}
