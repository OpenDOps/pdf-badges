//! Venue process. Local HTTP, WebSocket, and printing run on one
//! current-thread runtime. Remote sync runs on a second thread at a lower
//! nice value and waits while a local call or a print job is in progress.
//!
//! The Registration gRPC service is compiled for tests only. Venue clients
//! are browsers, which decode JSON natively, so the device binary serves
//! those calls over HTTP.

mod admin;
mod credentials;
mod gate;
#[cfg(test)]
mod grpc;
mod http;
mod print_queue;
mod remote;
mod store;
mod sync;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::watch;

pub use credentials::{
    CredentialError, CredentialFile, Credentials, FILE_NAME as CREDENTIALS_FILE_NAME,
};
pub use gate::Gate;
pub use print_queue::{PrintJob, PrintQueue};
pub use remote::{
    remote_base_from_env, remote_base_url, Exhibition, OperatorSession, Remote, RemoteError,
    DEFAULT_REMOTE_HOST, REMOTE_ENV,
};
pub use store::{Registration, Store};
pub use sync::{sync_loop, sync_once, SYNC_IN_FLIGHT};

#[derive(Clone)]
pub struct App {
    pub store: Arc<Store>,
    pub gate: Arc<Gate>,
    pub prints: PrintQueue,
}

impl App {
    pub fn new() -> (Self, print_queue::PrintWorker) {
        let gate = Arc::new(Gate::new());
        let (prints, worker) = PrintQueue::pair(Arc::clone(&gate));
        (
            Self {
                store: Arc::new(Store::new()),
                gate,
                prints,
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
    /// Normalized remote base, from `REGISTRATION_REMOTE` at process start.
    pub remote_base: String,
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
    if config.remote_base.is_empty() {
        return Err("remote base url is empty".into());
    }
    eprintln!("registration-server remote {}", config.remote_base);

    let (app, worker) = App::new();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
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
                tokio::select! {
                    result = serve(http, server_app, worker, server_shutdown) => {
                        result.map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
                    }
                    _ = tokio::signal::ctrl_c() => Ok(()),
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

    let endpoints = config.endpoints;
    let interval = config.sync_interval;
    let sync_thread = std::thread::Builder::new().name("sync".into()).spawn(
        move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            deprioritize_sync_thread();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(sync_loop(app, endpoints, interval, shutdown_rx));
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
