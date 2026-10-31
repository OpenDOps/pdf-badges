mod smoke;

use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_log();
    match std::env::args().nth(1).as_deref() {
        Some("smoke") => smoke::run().await,
        Some("serve") | None => {
            let addr: SocketAddr = google_tables::server::DEFAULT_ADDR.parse()?;
            google_tables::server::serve(addr).await
        }
        Some(other) => {
            eprintln!("usage: google-tables [serve|smoke]");
            eprintln!("unknown command {other}");
            std::process::exit(2);
        }
    }
}

struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("google-tables {} {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static STDERR_LOG: StderrLog = StderrLog;

fn init_log() {
    let _ = log::set_logger(&STDERR_LOG);
    log::set_max_level(log::LevelFilter::Warn);
}
