use std::time::{Duration, Instant};

use google_tables::google::{FakeBook, GoogleError};
use google_tables::limiter::{Kind, Limiter, PROJECT_PER_MINUTE, READS_PER_MINUTE};

fn admit(book: &mut FakeBook, limiter: &Limiter, account: &str, now: Instant) -> bool {
    if limiter.try_acquire(account, Kind::Read, now).is_err() {
        return false;
    }
    book.tokens = book.tokens.saturating_add(1);
    book.version("sheet").expect("read");
    limiter.succeed(account);
    true
}

#[test]
fn limiter_caps_reads() {
    let t = Instant::now();
    let limiter = Limiter::at(t);
    let mut book = FakeBook::new("sheet");
    for _ in 0..READS_PER_MINUTE {
        assert!(admit(&mut book, &limiter, "acct", t));
    }
    assert!(!admit(&mut book, &limiter, "acct", t));
    assert_eq!(
        limiter.try_acquire("acct", Kind::Read, t).unwrap_err(),
        Duration::from_secs(1)
    );
    assert_eq!(book.tokens, READS_PER_MINUTE);
    assert_eq!(limiter.spent("acct").reads, READS_PER_MINUTE);
    limiter.try_acquire("acct", Kind::Write, t).unwrap();
    assert_eq!(limiter.spent("acct").writes, 1);
    assert!(admit(
        &mut book,
        &limiter,
        "acct",
        t + Duration::from_secs(1)
    ));
    assert_eq!(book.tokens, READS_PER_MINUTE + 1);

    let project = Limiter::at(t);
    let accounts = PROJECT_PER_MINUTE / READS_PER_MINUTE;
    for index in 0..accounts {
        for _ in 0..READS_PER_MINUTE {
            project
                .try_acquire(&format!("acct-{index}"), Kind::Read, t)
                .unwrap();
        }
    }
    assert_eq!(
        project
            .try_acquire("acct-extra", Kind::Read, t)
            .unwrap_err(),
        Duration::from_millis(200)
    );
}

#[test]
fn limiter_shares_backoff() {
    let t = Instant::now();
    let limiter = Limiter::at(t);
    let mut book = FakeBook::new("sheet");
    book.rate_limit.push_back(true);

    limiter.try_acquire("acct", Kind::Read, t).unwrap();
    book.tokens = book.tokens.saturating_add(1);
    let err = book.version("sheet").unwrap_err();
    assert!(matches!(err, GoogleError::RateLimited));
    assert_eq!(limiter.penalize("acct", t), Duration::from_secs(1));
    assert_eq!(book.tokens, 1);

    assert_eq!(
        limiter.try_acquire("acct", Kind::Read, t).unwrap_err(),
        Duration::from_secs(1)
    );
    limiter.try_acquire("other", Kind::Read, t).unwrap();
    assert_eq!(book.tokens, 1);

    let later = t + Duration::from_secs(1);
    assert_eq!(limiter.penalize("acct", later), Duration::from_secs(2));
    limiter.succeed("acct");
    assert_eq!(
        limiter.penalize("acct", later + Duration::from_secs(2)),
        Duration::from_secs(1)
    );
}
