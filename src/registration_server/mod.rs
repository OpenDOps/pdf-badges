//! Venue process. Local HTTP, WebSocket, and printing run on one
//! current-thread runtime. Remote sync runs on a second thread at a lower
//! nice value and waits while a local call or a print job is in progress.
//!
//! The Registration gRPC service is compiled for tests only. Venue clients
//! are browsers, which decode JSON natively, so the device binary serves
//! those calls over HTTP.

mod admin;
mod credentials;
pub mod current;
pub mod event_db;
mod gate;
#[cfg(test)]
mod grpc;
mod http;
mod net;
mod print;
mod print_queue;
mod remote;
mod remote_server;
mod store;
mod sync;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::{watch, Notify};

pub use admin::{api_router, OperatorState};
pub use credentials::{
    CredentialError, CredentialFile, Credentials, FILE_NAME as CREDENTIALS_FILE_NAME,
};
pub use current::{base_dir, CurrentError, CurrentEvent, Selection};
pub use event_db::{
    EventDb, EventDbError, IndexState, ListOrder, ListRow, ScanWrite, SearchPage, VisitorWrite,
    CONFIG_FILE as BASE_CONFIG_FILE,
};
pub use gate::Gate;
pub use net::{Link, Quality, Snapshot};
pub use print_queue::{PrintJob, PrintQueue};
pub use remote::{
    remote_base_from_env, remote_base_url, Event, OperatorSession, Remote, RemoteError,
    DEFAULT_REMOTE_HOST, REMOTE_ENV,
};
pub use store::{Registration, Store};
pub use sync::{sync_loop, sync_once, SYNC_IN_FLIGHT};

#[derive(Clone)]
pub struct App {
    pub store: Arc<Store>,
    pub gate: Arc<Gate>,
    pub prints: PrintQueue,
    pub link: Arc<Link>,
    pub operator: OperatorState,
    /// Built admin files. `/` and `/events` serve `index.html` from here.
    pub dist: PathBuf,
    /// Event opened from the credential file, if one is logged in.
    pub current: Arc<Mutex<CurrentEvent>>,
    /// Set while a sync wake is inside the probe or the queue.
    sync_running: Arc<AtomicBool>,
    /// `POST /api/sync` signals the sync thread. The handler does not open a socket.
    sync_notify: Arc<Notify>,
    /// Finished wakes, including one that stopped before a socket.
    sync_passes: Arc<AtomicU64>,
}

impl App {
    /// Ask the sync thread for one wake. A wake already inside the probe or the queue is left alone.
    pub fn request_sync(&self) {
        if self.sync_running.load(Ordering::Acquire) {
            return;
        }
        self.sync_notify.notify_one();
    }

    #[cfg(test)]
    pub(crate) fn sync_passes(&self) -> u64 {
        self.sync_passes.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn sync_busy(&self) -> bool {
        self.sync_running.load(Ordering::Acquire)
    }

    pub fn new() -> (Self, print_queue::PrintWorker) {
        let gate = Arc::new(Gate::new());
        let (prints, worker) = PrintQueue::pair(Arc::clone(&gate));
        let link = Arc::new(Link::new());
        let operator = OperatorState::new(
            CredentialFile::new("credentials.yml"),
            Remote::new("http://127.0.0.1:1").expect("loopback remote url"),
        )
        .with_link(Arc::clone(&link));
        (
            Self {
                store: Arc::new(Store::new()),
                gate,
                prints,
                link,
                operator,
                dist: PathBuf::from("registration-admin/dist"),
                current: Arc::new(Mutex::new(CurrentEvent::new("."))),
                sync_running: Arc::new(AtomicBool::new(false)),
                sync_notify: Arc::new(Notify::new()),
                sync_passes: Arc::new(AtomicU64::new(0)),
            },
            worker,
        )
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub http_listen: SocketAddr,
    pub endpoints: Vec<String>,
    pub sync_interval: Duration,
    /// Remote calls that may wait on a response at once. The compiled [`SYNC_IN_FLIGHT`], unless `--sync-in-flight` replaced it.
    pub sync_in_flight: usize,
    /// Normalized remote base, from `REGISTRATION_REMOTE` at process start.
    pub remote_base: String,
    /// `registration-admin/dist` from `npm run build`.
    pub dist: PathBuf,
    /// Credential document. A missing file loads as an empty document.
    pub credentials_path: PathBuf,
    /// Debug lines for HTTP, login, the event list, select, sync, and print.
    pub debug: bool,
    /// One stderr line for every remote ping. Not included in `debug`.
    pub debug_ping: bool,
}

#[derive(Debug)]
pub enum ServeError {
    Http(std::io::Error),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::Http(err) => write!(f, "http server: {err}"),
        }
    }
}

impl std::error::Error for ServeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServeError::Http(err) => Some(err),
        }
    }
}

pub async fn serve(
    http: TcpListener,
    app: App,
    worker: print_queue::PrintWorker,
    shutdown: watch::Receiver<bool>,
) -> Result<(), ServeError> {
    let http_shutdown = shutdown.clone();
    tokio::try_join!(
        http::serve(http, app, http_shutdown),
        print_queue::run(worker, shutdown),
    )?;
    Ok(())
}

pub fn run(config: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if config.sync_interval.is_zero() {
        return Err("sync interval must be greater than zero".into());
    }
    if config.sync_in_flight == 0 {
        return Err("sync in flight must be greater than zero".into());
    }
    if config.remote_base.is_empty() {
        return Err("remote base url is empty".into());
    }
    if config.debug {
        init_debug_log();
    }
    eprintln!("registration-server remote {}", config.remote_base);
    if config.debug {
        log::debug!(
            "debug logging on dist={} credentials={} sync_every={}s sync_in_flight={}",
            config.dist.display(),
            config.credentials_path.display(),
            config.sync_interval.as_secs(),
            config.sync_in_flight
        );
    }

    let (mut app, worker) = App::new();
    let remote = Remote::new(&config.remote_base)?;
    let credentials = CredentialFile::new(&config.credentials_path);
    match credentials.load() {
        Ok(loaded) => match loaded.device_id() {
            Some(device_id) => eprintln!(
                "registration-server credentials {} device_id={device_id}",
                config.credentials_path.display()
            ),
            None => eprintln!(
                "registration-server credentials {} has no device_id",
                config.credentials_path.display()
            ),
        },
        Err(err) => {
            eprintln!(
                "registration-server credentials {}: {err}",
                config.credentials_path.display()
            );
            return Err(err.to_string().into());
        }
    }
    let current = Arc::new(Mutex::new(CurrentEvent::new(base_dir(
        &config.credentials_path,
    ))));
    {
        let opened = CurrentEvent::lock(&current);
        opened.open_bound(&credentials)?;
        if let Some(event_id) = opened.event_id() {
            eprintln!("registration-server event {event_id}");
        }
    }
    app.current = Arc::clone(&current);
    app.operator = OperatorState::new(credentials, remote)
        .with_link(Arc::clone(&app.link))
        .with_current(Arc::clone(&current));
    app.dist = config.dist.clone();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signal_tx = shutdown_tx.clone();
    let _close_event = CloseEvent(Arc::clone(&current));
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);

    let http_listen = config.http_listen;
    let server_app = app.clone();
    let server_shutdown = shutdown_rx.clone();
    let server = std::thread::Builder::new()
        .name("server".into())
        .spawn(move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .max_blocking_threads(1)
                .build()?;
            rt.block_on(async move {
                let http = match TcpListener::bind(http_listen).await {
                    Ok(listener) => listener,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.to_string()));
                        return Err(Box::new(err) as Box<dyn std::error::Error + Send + Sync>);
                    }
                };
                let http_addr = http.local_addr()?;
                eprintln!("registration-server http+ws {http_addr}");
                let _ = ready_tx.send(Ok(()));
                let mut server = std::pin::pin!(serve(http, server_app, worker, server_shutdown));
                tokio::select! {
                    result = &mut server => {
                        result.map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
                    }
                    _ = shutdown_signal() => {
                        eprintln!("registration-server shutting down");
                        let _ = signal_tx.send(true);
                        server.await.map_err(|err| {
                            Box::new(err) as Box<dyn std::error::Error + Send + Sync>
                        })
                    }
                }
            })
        })?;

    match ready_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            let _ = server.join();
            return Err(err.into());
        }
        Err(_) => {
            let _ = server.join();
            return Err("server thread ended before listening".into());
        }
    }

    let remote_server = remote_server::RemoteServer::new(&config.remote_base)?;
    CurrentEvent::lock(&current).set_remote(remote_server.clone());

    let endpoints = config.endpoints;
    let interval = config.sync_interval;
    let in_flight = config.sync_in_flight;
    let remote_base = config.remote_base.clone();
    let debug_ping = config.debug_ping;
    let sync_thread = std::thread::Builder::new().name("sync".into()).spawn(
        move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            deprioritize_sync_thread();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let link = Arc::clone(&app.link);
            rt.block_on(async move {
                tokio::join!(
                    sync_loop(
                        app,
                        endpoints,
                        interval,
                        in_flight,
                        remote_server,
                        shutdown_rx.clone(),
                    ),
                    link.probe(remote_base, debug_ping, shutdown_rx),
                );
            });
            Ok(())
        },
    )?;

    let server_result = match server.join() {
        Ok(result) => result,
        Err(_) => Err("server thread panicked".into()),
    };
    drop(shutdown_tx);
    let sync_result = match sync_thread.join() {
        Ok(result) => result,
        Err(_) => Err("sync thread panicked".into()),
    };
    server_result.and(sync_result)
}

struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata
            .target()
            .starts_with("rust_reg::registration_server")
            && metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("registration-server {} {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static STDERR_LOG: StderrLog = StderrLog;

struct CloseEvent(Arc<Mutex<CurrentEvent>>);

impl Drop for CloseEvent {
    fn drop(&mut self) {
        let id = CurrentEvent::lock(&self.0).event_id();
        match CurrentEvent::lock(&self.0).close() {
            Ok(()) => {
                if id.is_some() {
                    eprintln!("registration-server event closed");
                }
            }
            Err(err) => eprintln!("registration-server event close failed: {err}"),
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(err) => {
                    eprintln!("registration-server: SIGTERM handler: {err}");
                    let _ = ctrl_c.await;
                    return;
                }
            };
        tokio::select! {
            result = ctrl_c => {
                if let Err(err) = result {
                    eprintln!("registration-server: SIGINT handler: {err}");
                }
            }
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        if let Err(err) = ctrl_c.await {
            eprintln!("registration-server: SIGINT handler: {err}");
        }
    }
}

fn init_debug_log() {
    let _ = log::set_logger(&STDERR_LOG);
    log::set_max_level(log::LevelFilter::Debug);
}

fn deprioritize_sync_thread() {
    #[cfg(target_os = "linux")]
    unsafe {
        const SYNC_NICE: i32 = 15;
        let tid = libc::syscall(libc::SYS_gettid);
        if tid <= 0 {
            eprintln!(
                "registration-server: gettid failed: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        let rc = libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, SYNC_NICE);
        if rc != 0 {
            eprintln!(
                "registration-server: setpriority failed: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}
