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

    pub async fn wait_until_idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            if self.is_idle() {
                return;
            }
            notified.await;
        }
    }

    fn notify_if_idle(&self) {
        if self.is_idle() {
            self.idle.notify_waiters();
        }
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
        self.gate.notify_if_idle();
    }
}

impl Drop for PrintGuard {
    fn drop(&mut self) {
        self.gate.printing.store(false, Ordering::Release);
        self.gate.notify_if_idle();
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
}
