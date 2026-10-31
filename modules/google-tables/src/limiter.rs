//! One quota for every Sheets and Drive call.
//!
//! Each `credentials_ref` has 60 reads and 60 writes per minute. The process has 300 calls
//! per minute. A 429 backs that account off; the exponent is shared by every watch on it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::google::backoff_delay;

pub const READS_PER_MINUTE: u32 = 60;
pub const WRITES_PER_MINUTE: u32 = 60;
pub const PROJECT_PER_MINUTE: u32 = 300;

const MINUTE_NS: u128 = 60_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spent {
    pub reads: u32,
    pub writes: u32,
}

struct Bucket {
    tokens: u32,
    capacity: u32,
    per_minute: u32,
    last: Instant,
}

struct Account {
    reads: Bucket,
    writes: Bucket,
    failures: u32,
    backoff_until: Option<Instant>,
    spent: Spent,
}

struct Inner {
    accounts: HashMap<String, Account>,
    project: Bucket,
}

pub struct Limiter {
    inner: Mutex<Inner>,
}

impl Limiter {
    pub fn new() -> Self {
        Self::at(Instant::now())
    }

    pub fn at(now: Instant) -> Self {
        Self {
            inner: Mutex::new(Inner {
                accounts: HashMap::new(),
                project: Bucket::full(PROJECT_PER_MINUTE, now),
            }),
        }
    }

    /// Take one token. `Err` is how long to wait before trying again, and spends nothing.
    pub fn try_acquire(&self, account: &str, kind: Kind, now: Instant) -> Result<(), Duration> {
        let mut inner = self.inner.lock().expect("limiter");
        inner.project.refill(now);
        let project_tokens = inner.project.tokens;
        let project_wait = inner.project.until_token(now);
        let entry = inner
            .accounts
            .entry(account.to_string())
            .or_insert_with(|| Account::new(now));
        if let Some(until) = entry.backoff_until {
            if now < until {
                return Err(until.saturating_duration_since(now));
            }
        }
        let bucket = match kind {
            Kind::Read => &mut entry.reads,
            Kind::Write => &mut entry.writes,
        };
        bucket.refill(now);
        if bucket.tokens == 0 {
            return Err(bucket.until_token(now));
        }
        if project_tokens == 0 {
            return Err(project_wait);
        }
        bucket.tokens -= 1;
        match kind {
            Kind::Read => entry.spent.reads = entry.spent.reads.saturating_add(1),
            Kind::Write => entry.spent.writes = entry.spent.writes.saturating_add(1),
        }
        let _ = entry;
        inner.project.tokens -= 1;
        Ok(())
    }

    /// A 429 on this account. The returned delay is the shared backoff, and `n` advances.
    pub fn penalize(&self, account: &str, now: Instant) -> Duration {
        let mut inner = self.inner.lock().expect("limiter");
        let entry = inner
            .accounts
            .entry(account.to_string())
            .or_insert_with(|| Account::new(now));
        let delay = backoff_delay(entry.failures, Duration::ZERO);
        entry.failures = entry.failures.saturating_add(1);
        entry.backoff_until = Some(now + delay);
        delay
    }

    /// The next success on this account clears the shared exponent.
    pub fn succeed(&self, account: &str) {
        let mut inner = self.inner.lock().expect("limiter");
        if let Some(entry) = inner.accounts.get_mut(account) {
            entry.failures = 0;
            entry.backoff_until = None;
        }
    }

    pub fn spent(&self, account: &str) -> Spent {
        self.inner
            .lock()
            .expect("limiter")
            .accounts
            .get(account)
            .map(|entry| entry.spent)
            .unwrap_or(Spent {
                reads: 0,
                writes: 0,
            })
    }
}

impl Account {
    fn new(now: Instant) -> Self {
        Self {
            reads: Bucket::full(READS_PER_MINUTE, now),
            writes: Bucket::full(WRITES_PER_MINUTE, now),
            failures: 0,
            backoff_until: None,
            spent: Spent {
                reads: 0,
                writes: 0,
            },
        }
    }
}

impl Bucket {
    fn full(per_minute: u32, now: Instant) -> Self {
        Self {
            tokens: per_minute,
            capacity: per_minute,
            per_minute,
            last: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        if now <= self.last || self.per_minute == 0 {
            return;
        }
        let elapsed = now.saturating_duration_since(self.last).as_nanos();
        let add = elapsed.saturating_mul(self.per_minute as u128) / MINUTE_NS;
        if add == 0 {
            return;
        }
        let room = self.capacity.saturating_sub(self.tokens) as u128;
        let grant = add.min(room);
        self.tokens = self.tokens.saturating_add(grant as u32);
        if self.tokens == self.capacity {
            self.last = now;
        } else {
            let step = grant.saturating_mul(MINUTE_NS) / self.per_minute as u128;
            self.last += Duration::from_nanos(step as u64);
        }
    }

    fn until_token(&self, now: Instant) -> Duration {
        let interval_ns = MINUTE_NS / self.per_minute as u128;
        let ready = self.last + Duration::from_nanos(interval_ns as u64);
        ready.saturating_duration_since(now)
    }
}
