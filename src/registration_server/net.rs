//! Connectivity to the remote host.
//!
//! A probe runs every five seconds on the sync thread. Two samples are enough
//! to name the link. Login and the event list use the timeout from that
//! reading. The timeout is a whole number of seconds.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::sync::watch;

pub const PROBE_EVERY: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const WINDOW: usize = 8;
const READY: usize = 2;

const TIMEOUT_OFFLINE: u64 = 2;
const TIMEOUT_GOOD: u64 = 5;
const TIMEOUT_FAIR: u64 = 8;
const TIMEOUT_POOR: u64 = 10;
const TIMEOUT_SLOW: u64 = 20;
const TIMEOUT_UNKNOWN: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Good,
    Fair,
    Poor,
}

impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Good => "good",
            Quality::Fair => "fair",
            Quality::Poor => "poor",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub samples: usize,
    pub offline: bool,
    pub quality: Option<Quality>,
    pub timeout_secs: u64,
}

/// Deadline for one call to the remote server.
///
/// `timeout_secs` is the whole seconds reported to the browser.
/// `limit` is that same budget as a [`Duration`] for the HTTP client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallBudget {
    pub timeout_secs: u64,
    pub limit: Duration,
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    ok: bool,
    rtt: Duration,
}

#[derive(Debug)]
pub struct Link {
    samples: Mutex<VecDeque<Sample>>,
}

impl Link {
    pub fn new() -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
        }
    }

    pub fn record(&self, ok: bool, rtt: Duration) {
        let mut samples = self.samples.lock().unwrap_or_else(|err| err.into_inner());
        if samples.len() == WINDOW {
            samples.pop_front();
        }
        samples.push_back(Sample { ok, rtt });
    }

    pub fn snapshot(&self) -> Snapshot {
        let samples = self.samples.lock().unwrap_or_else(|err| err.into_inner());
        judge(&samples)
    }

    /// Quality-based deadline for the next remote call.
    ///
    /// Login, the event list, bind, and later calls to the remote server
    /// use this instead of choosing their own timeout.
    pub fn budget(&self) -> CallBudget {
        let timeout_secs = self.snapshot().timeout_secs;
        CallBudget {
            timeout_secs,
            limit: Duration::from_secs(timeout_secs),
        }
    }

    pub async fn probe(
        self: &std::sync::Arc<Self>,
        base: String,
        log_each: bool,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let client = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .connect_timeout(PROBE_TIMEOUT)
            .build()
            .expect("link probe client");
        let mut ticker = tokio::time::interval(PROBE_EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let (ok, rtt, detail) = probe_once(&client, &base).await;
                    self.record(ok, rtt);
                    if log_each {
                        let snap = self.snapshot();
                        eprintln!(
                            "registration-server PING {base} {detail} {}ms quality={} offline={} samples={} timeout={}s",
                            rtt.as_millis(),
                            quality_label(&snap),
                            snap.offline,
                            snap.samples,
                            snap.timeout_secs,
                        );
                    }
                }
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() {
                        return;
                    }
                }
            }
        }
    }
}

fn judge(samples: &VecDeque<Sample>) -> Snapshot {
    if samples.len() < READY {
        return Snapshot {
            samples: samples.len(),
            offline: false,
            quality: None,
            timeout_secs: TIMEOUT_UNKNOWN,
        };
    }
    let last = samples.iter().rev().take(READY).collect::<Vec<_>>();
    let failures = last.iter().filter(|sample| !sample.ok).count();
    if failures == READY {
        return Snapshot {
            samples: samples.len(),
            offline: true,
            quality: None,
            timeout_secs: TIMEOUT_OFFLINE,
        };
    }
    if failures == 1 {
        return Snapshot {
            samples: samples.len(),
            offline: false,
            quality: Some(Quality::Poor),
            timeout_secs: TIMEOUT_POOR,
        };
    }
    let worst = last
        .iter()
        .map(|sample| sample.rtt)
        .max()
        .unwrap_or_default();
    let (quality, timeout_secs) = if worst <= Duration::from_millis(300) {
        (Quality::Good, TIMEOUT_GOOD)
    } else if worst <= Duration::from_millis(1500) {
        (Quality::Fair, TIMEOUT_FAIR)
    } else {
        (Quality::Poor, TIMEOUT_SLOW)
    };
    Snapshot {
        samples: samples.len(),
        offline: false,
        quality: Some(quality),
        timeout_secs,
    }
}

fn quality_label(snapshot: &Snapshot) -> &'static str {
    if snapshot.offline {
        "offline"
    } else {
        snapshot.quality.map(Quality::as_str).unwrap_or("hidden")
    }
}

async fn probe_once(client: &reqwest::Client, base: &str) -> (bool, Duration, String) {
    let started = Instant::now();
    match client.get(base).send().await {
        Ok(response) => (true, started.elapsed(), response.status().to_string()),
        Err(err) => (false, started.elapsed(), format!("failed: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_fast_samples_are_good() {
        let link = Link::new();
        assert_eq!(link.snapshot().timeout_secs, TIMEOUT_UNKNOWN);
        assert!(link.snapshot().quality.is_none());
        link.record(true, Duration::from_millis(80));
        link.record(true, Duration::from_millis(120));
        let snap = link.snapshot();
        assert_eq!(snap.samples, 2);
        assert!(!snap.offline);
        assert_eq!(snap.quality, Some(Quality::Good));
        assert_eq!(snap.timeout_secs, 5);
        assert_eq!(snap.timeout_secs % 1, 0);
        let budget = link.budget();
        assert_eq!(budget.timeout_secs, 5);
        assert_eq!(budget.limit, Duration::from_secs(5));
    }

    #[test]
    fn slow_samples_lengthen_the_timeout() {
        let link = Link::new();
        link.record(true, Duration::from_millis(500));
        link.record(true, Duration::from_millis(400));
        assert_eq!(link.snapshot().quality, Some(Quality::Fair));
        assert_eq!(link.snapshot().timeout_secs, 8);
        link.record(true, Duration::from_millis(1200));
        link.record(true, Duration::from_millis(1500));
        assert_eq!(link.snapshot().quality, Some(Quality::Fair));
        assert_eq!(link.snapshot().timeout_secs, 8);
        link.record(true, Duration::from_millis(1501));
        link.record(true, Duration::from_millis(900));
        assert_eq!(link.snapshot().quality, Some(Quality::Poor));
        assert_eq!(link.snapshot().timeout_secs, 20);
    }

    #[test]
    fn two_failures_are_offline() {
        let link = Link::new();
        link.record(false, Duration::from_millis(4000));
        link.record(false, Duration::from_millis(4000));
        let snap = link.snapshot();
        assert!(snap.offline);
        assert!(snap.quality.is_none());
        assert_eq!(snap.timeout_secs, 2);
    }

    #[test]
    fn one_failure_is_poor() {
        let link = Link::new();
        link.record(true, Duration::from_millis(40));
        link.record(false, Duration::from_millis(4000));
        let snap = link.snapshot();
        assert!(!snap.offline);
        assert_eq!(snap.quality, Some(Quality::Poor));
        assert_eq!(snap.timeout_secs, 10);
    }
}
