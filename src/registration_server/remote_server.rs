//! Remote-server client. Calls that send the project token.
//!
//! `send` and each chunk of the response body are `.await` points, so other
//! tasks on this runtime run while the socket is waiting. Login, the event
//! list, and bind stay in `remote` and keep the operator session.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use reqwest::Url;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::registration_server::remote_base_url;

const MAX_BODY_BYTES: usize = 1024 * 1024;

/// How [`RemoteServer::request`] carries the caller fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method<'a> {
    /// `application/x-www-form-urlencoded` body.
    #[cfg_attr(not(test), allow(dead_code))]
    Post,
    /// Fields in the query string. No body.
    Get,
    /// File parts plus the form fields, including `dev_id`, `token`, and `eid`.
    #[cfg_attr(not(test), allow(dead_code))]
    Multipart {
        files: &'a [&'a std::path::Path],
        /// Part names parallel to `files`. An empty slice uses each file's name.
        names: &'a [&'a str],
    },
}

/// The three values every tokened call sends.
pub struct Auth<'a> {
    pub device_id: &'a str,
    pub event_id: &'a str,
    pub project_token: &'a str,
}

/// Failure from a tokened call. The operator session uses [`crate::registration_server::RemoteError`].
#[derive(Debug)]
pub enum RemoteServerError {
    /// `device_id`, `event_id`, or `project_token` is missing. No socket was opened.
    InvalidCred,
    /// `error_msg` was `not_logged_in` or `no_pattern`. The binding stays until the caller clears it.
    TokenRejected(&'static str),
    /// [`RemoteServer::drop_requests`] cancelled this call, or the event was already dropped.
    Dropped,
    NoConnection,
    Status(u16),
    BadResponse(String),
    Url(String),
}

impl std::fmt::Display for RemoteServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoteServerError::InvalidCred => write!(f, "invalid_cred"),
            RemoteServerError::TokenRejected(msg) => write!(f, "{msg}"),
            RemoteServerError::Dropped => write!(f, "dropped"),
            RemoteServerError::NoConnection => write!(f, "no_connection"),
            RemoteServerError::Status(code) => write!(f, "remote status {code}"),
            RemoteServerError::BadResponse(detail) => write!(f, "remote response: {detail}"),
            RemoteServerError::Url(detail) => write!(f, "remote url: {detail}"),
        }
    }
}

impl std::error::Error for RemoteServerError {}

/// HTTP client for kuprin calls that carry the project token.
///
/// It does not send `JSESSIONID` and it does not take the operator single-flight flag.
/// Each call is registered under the event id it was started for. [`RemoteServer::drop_requests`]
/// cancels the ones still running for that id.
#[derive(Clone)]
pub struct RemoteServer {
    base: Url,
    http: reqwest::Client,
    flights: Arc<Mutex<Flights>>,
}

struct Flights {
    next: u64,
    /// Event ids whose running calls were dropped. A later call stays dropped until
    /// [`RemoteServer::allow_requests`].
    blocked: HashSet<String>,
    by_event: HashMap<String, Vec<InFlight>>,
}

struct InFlight {
    id: u64,
    cancel: oneshot::Sender<()>,
}

struct Ticket {
    flights: Arc<Mutex<Flights>>,
    event_id: String,
    id: u64,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut flights = lock_flights(&self.flights);
        let Some(list) = flights.by_event.get_mut(&self.event_id) else {
            return;
        };
        list.retain(|flight| flight.id != self.id);
        if list.is_empty() {
            flights.by_event.remove(&self.event_id);
        }
    }
}

impl std::fmt::Debug for RemoteServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteServer")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl RemoteServer {
    /// `base` is the same string [`crate::registration_server::Remote::new`] accepts.
    pub fn new(base: &str) -> Result<Self, RemoteServerError> {
        let base =
            remote_base_url(Some(base)).map_err(|err| RemoteServerError::Url(err.to_string()))?;
        let http = reqwest::Client::builder()
            .build()
            .map_err(|_| RemoteServerError::NoConnection)?;
        Ok(Self {
            base,
            http,
            flights: Arc::new(Mutex::new(Flights {
                next: 0,
                blocked: HashSet::new(),
                by_event: HashMap::new(),
            })),
        })
    }

    /// Send `fields` plus `dev_id`, `token`, and `eid`, and return the JSON body.
    ///
    /// The three credential fields replace a caller field of the same name.
    /// A missing credential returns [`RemoteServerError::InvalidCred`] before
    /// the client opens a socket. The call is registered under `auth.event_id`
    /// before the socket, and [`RemoteServer::drop_requests`] for that id
    /// cancels it. A 200 body that is not JSON is [`RemoteServerError::BadResponse`].
    pub async fn request(
        &self,
        method: Method<'_>,
        path: &str,
        fields: &[(&str, &str)],
        auth: Auth<'_>,
        timeout: Duration,
    ) -> Result<Value, RemoteServerError> {
        let bytes = self
            .request_bytes(method, path, fields, auth, timeout)
            .await?;
        serde_json::from_slice(&bytes)
            .map_err(|_| RemoteServerError::BadResponse("response is not json".into()))
    }

    /// Same send as [`Self::request`], returning the body bytes.
    ///
    /// A JSON body with `error_msg` `not_logged_in` or `no_pattern` is
    /// [`RemoteServerError::TokenRejected`]. Any other body, including a body
    /// that is not JSON, is returned as bytes.
    pub async fn request_bytes(
        &self,
        method: Method<'_>,
        path: &str,
        fields: &[(&str, &str)],
        auth: Auth<'_>,
        timeout: Duration,
    ) -> Result<Vec<u8>, RemoteServerError> {
        let device_id = required(auth.device_id)?;
        let event_id = required(auth.event_id)?;
        let token = required(auth.project_token)?;
        if let Method::Multipart { files, .. } = method {
            for file in files {
                if !file.is_file() {
                    return Err(RemoteServerError::BadResponse(format!(
                        "not a file: {}",
                        file.display()
                    )));
                }
            }
        }
        let (ticket, cancel) = self.begin(event_id)?;
        let result = self
            .drive(
                method, path, fields, device_id, event_id, token, timeout, cancel,
            )
            .await;
        drop(ticket);
        let bytes = result?;
        reject_dead_token(&bytes)?;
        Ok(bytes)
    }

    /// Cancel every call still running for `event_id`.
    ///
    /// A later [`Self::request`] for that id returns [`RemoteServerError::Dropped`]
    /// and does not open a socket, until [`Self::allow_requests`] for the same id.
    /// Calls registered under any other event id keep running.
    pub fn drop_requests(&self, event_id: &str) {
        let event_id = event_id.trim();
        if event_id.is_empty() {
            return;
        }
        let mut flights = lock_flights(&self.flights);
        flights.blocked.insert(event_id.to_string());
        if let Some(running) = flights.by_event.remove(event_id) {
            for flight in running {
                drop(flight.cancel);
            }
        }
    }

    /// Let calls for `event_id` run again after [`Self::drop_requests`].
    pub(crate) fn allow_requests(&self, event_id: &str) {
        let event_id = event_id.trim();
        if event_id.is_empty() {
            return;
        }
        lock_flights(&self.flights).blocked.remove(event_id);
    }

    fn begin(&self, event_id: &str) -> Result<(Ticket, oneshot::Receiver<()>), RemoteServerError> {
        let mut flights = lock_flights(&self.flights);
        if flights.blocked.contains(event_id) {
            return Err(RemoteServerError::Dropped);
        }
        let (cancel, rx) = oneshot::channel();
        let id = flights.next;
        flights.next = flights.next.wrapping_add(1);
        flights
            .by_event
            .entry(event_id.to_string())
            .or_default()
            .push(InFlight { id, cancel });
        Ok((
            Ticket {
                flights: Arc::clone(&self.flights),
                event_id: event_id.to_string(),
                id,
            },
            rx,
        ))
    }

    async fn drive(
        &self,
        method: Method<'_>,
        path: &str,
        fields: &[(&str, &str)],
        device_id: &str,
        event_id: &str,
        token: &str,
        timeout: Duration,
        mut cancel: oneshot::Receiver<()>,
    ) -> Result<Vec<u8>, RemoteServerError> {
        let send = async {
            let url = self
                .base
                .join(path)
                .map_err(|err| RemoteServerError::Url(err.to_string()))?;
            let pairs = with_auth(fields, device_id, event_id, token);
            let builder = match method {
                Method::Post => self.http.post(url).form(&pairs),
                Method::Get => self.http.get(url).query(&pairs),
                Method::Multipart { files, names } => {
                    let mut form = reqwest::multipart::Form::new();
                    for (name, value) in &pairs {
                        form = form.text(name.clone(), value.clone());
                    }
                    for (index, file) in files.iter().enumerate() {
                        let name = names
                            .get(index)
                            .copied()
                            .filter(|name| !name.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| {
                                file.file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "file".to_string())
                            });
                        let part = reqwest::multipart::Part::file(file)
                            .await
                            .map_err(|err| RemoteServerError::BadResponse(err.to_string()))?;
                        form = form.part(name, part);
                    }
                    self.http.post(url).multipart(form)
                }
            };
            let response = builder
                .timeout(timeout)
                .send()
                .await
                .map_err(transport_error)?;
            read_body_bytes(response).await
        };
        tokio::pin!(send);
        tokio::select! {
            biased;
            _ = &mut cancel => Err(RemoteServerError::Dropped),
            result = &mut send => result,
        }
    }
}

fn lock_flights(flights: &Mutex<Flights>) -> std::sync::MutexGuard<'_, Flights> {
    flights.lock().unwrap_or_else(|err| err.into_inner())
}

fn required(value: &str) -> Result<&str, RemoteServerError> {
    let value = value.trim();
    if value.is_empty() {
        Err(RemoteServerError::InvalidCred)
    } else {
        Ok(value)
    }
}

/// Caller fields first. `dev_id`, `token`, and `eid` then replace any caller field of that name.
fn with_auth(
    fields: &[(&str, &str)],
    device_id: &str,
    event_id: &str,
    token: &str,
) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = fields
        .iter()
        .filter(|(name, _)| !matches!(*name, "dev_id" | "token" | "eid"))
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect();
    pairs.push(("dev_id".to_string(), device_id.to_string()));
    pairs.push(("token".to_string(), token.to_string()));
    pairs.push(("eid".to_string(), event_id.to_string()));
    pairs
}

fn transport_error(err: reqwest::Error) -> RemoteServerError {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        RemoteServerError::NoConnection
    } else {
        RemoteServerError::BadResponse(err.to_string())
    }
}

/// Headers come back from `send`. Body chunks come from the byte stream.
/// Both awaits leave the runtime free to poll other tasks.
async fn read_body_bytes(response: reqwest::Response) -> Result<Vec<u8>, RemoteServerError> {
    let status = response.status();
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(transport_error)?;
        if body.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err(RemoteServerError::BadResponse(format!(
                "body exceeds {MAX_BODY_BYTES} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        return Err(RemoteServerError::Status(status.as_u16()));
    }
    Ok(body)
}

fn reject_dead_token(bytes: &[u8]) -> Result<(), RemoteServerError> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(());
    };
    match value.get("error_msg").and_then(Value::as_str) {
        Some("not_logged_in") => Err(RemoteServerError::TokenRejected("not_logged_in")),
        Some("no_pattern") => Err(RemoteServerError::TokenRejected("no_pattern")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::header;
    use axum::routing::any;
    use axum::{Json, Router};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const TIMEOUT: Duration = Duration::from_secs(2);

    fn auth<'a>() -> Auth<'a> {
        Auth {
            device_id: "device-1",
            event_id: "EVT1",
            project_token: "stored-token",
        }
    }

    #[derive(Clone)]
    struct Hit {
        method: String,
        path: String,
        query: String,
        content_type: String,
        cookie: Option<String>,
        body: String,
    }

    #[derive(Clone)]
    struct StandIn {
        base: String,
        hits: Arc<Mutex<Vec<Hit>>>,
        handle: Arc<tokio::task::JoinHandle<()>>,
    }

    impl StandIn {
        async fn start() -> Self {
            let hits = Arc::new(Mutex::new(Vec::new()));
            let app = Router::new()
                .route("/boxapi/getconf", any(record))
                .route("/boxapi/barcodesinfo", any(record))
                .route("/boxapi/uploadphoto", any(record))
                .with_state(Arc::clone(&hits));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            Self {
                base: format!("http://{addr}/"),
                hits,
                handle: Arc::new(handle),
            }
        }

        fn hit(&self) -> Hit {
            let hits = self.hits.lock().unwrap();
            assert_eq!(hits.len(), 1);
            hits[0].clone()
        }
    }

    impl Drop for StandIn {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn record(
        State(hits): State<Arc<Mutex<Vec<Hit>>>>,
        req: axum::extract::Request,
    ) -> Json<Value> {
        let method = req.method().as_str().to_string();
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or("").to_string();
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let cookie = req
            .headers()
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(req.into_body(), MAX_BODY_BYTES)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        hits.lock().unwrap().push(Hit {
            method,
            path,
            query,
            content_type,
            cookie,
            body,
        });
        Json(json!({"status": "ok"}))
    }

    fn form_map(raw: &str) -> HashMap<String, String> {
        let mut pairs = HashMap::new();
        if raw.is_empty() {
            return pairs;
        }
        for pair in raw.split('&') {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            pairs.insert(percent_decode(name), percent_decode(value));
        }
        pairs
    }

    fn percent_decode(raw: &str) -> String {
        let bytes = raw.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'+' => out.push(b' '),
                b'%' if index + 2 < bytes.len() => {
                    let hex = &raw[index + 1..index + 3];
                    match u8::from_str_radix(hex, 16) {
                        Ok(byte) => out.push(byte),
                        Err(_) => out.extend_from_slice(&bytes[index..index + 3]),
                    }
                    index += 2;
                }
                byte => out.push(byte),
            }
            index += 1;
        }
        String::from_utf8(out).unwrap()
    }

    async fn refused(method: Method<'_>, auth: Auth<'_>) -> RemoteServerError {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = RemoteServer::new(&format!("http://{addr}/")).unwrap();
        let err = client
            .request(
                method,
                "boxapi/getconf",
                &[("last_synch", "0")],
                auth,
                TIMEOUT,
            )
            .await
            .unwrap_err();
        let accepted = tokio::time::timeout(Duration::from_millis(50), listener.accept()).await;
        assert!(accepted.is_err(), "a missing credential opened a socket");
        err
    }

    mod form {
        use super::*;

        #[tokio::test]
        async fn post_sends_the_three_fields() {
            let stand = StandIn::start().await;
            let client = RemoteServer::new(&stand.base).unwrap();
            let body = client
                .request(
                    Method::Post,
                    "boxapi/getconf",
                    &[("last_synch", "0")],
                    auth(),
                    TIMEOUT,
                )
                .await
                .unwrap();
            assert_eq!(body["status"], "ok");

            let hit = stand.hit();
            assert_eq!(hit.method, "POST");
            assert_eq!(hit.path, "/boxapi/getconf");
            assert!(hit.query.is_empty());
            assert!(hit
                .content_type
                .starts_with("application/x-www-form-urlencoded"));
            assert!(hit.cookie.is_none());
            let fields = form_map(&hit.body);
            assert_eq!(fields.get("last_synch").map(String::as_str), Some("0"));
            assert_eq!(fields.get("dev_id").map(String::as_str), Some("device-1"));
            assert_eq!(
                fields.get("token").map(String::as_str),
                Some("stored-token")
            );
            assert_eq!(fields.get("eid").map(String::as_str), Some("EVT1"));
            assert_eq!(fields.len(), 4);
        }

        #[tokio::test]
        async fn stored_token_replaces_a_caller_token() {
            let stand = StandIn::start().await;
            let client = RemoteServer::new(&stand.base).unwrap();
            client
                .request(
                    Method::Post,
                    "boxapi/getconf",
                    &[("token", "other"), ("last_synch", "0")],
                    auth(),
                    TIMEOUT,
                )
                .await
                .unwrap();
            let fields = form_map(&stand.hit().body);
            assert_eq!(
                fields.get("token").map(String::as_str),
                Some("stored-token")
            );
            assert_eq!(fields.get("last_synch").map(String::as_str), Some("0"));
        }

        #[tokio::test]
        async fn missing_credential_opens_no_socket() {
            for auth in [
                Auth {
                    device_id: "",
                    event_id: "EVT1",
                    project_token: "stored-token",
                },
                Auth {
                    device_id: "device-1",
                    event_id: "",
                    project_token: "stored-token",
                },
                Auth {
                    device_id: "device-1",
                    event_id: "EVT1",
                    project_token: "",
                },
                Auth {
                    device_id: "device-1",
                    event_id: "EVT1",
                    project_token: "   ",
                },
            ] {
                let err = refused(Method::Post, auth).await;
                assert!(matches!(err, RemoteServerError::InvalidCred), "{err}");
                assert_eq!(err.to_string(), "invalid_cred");
            }
        }

        /// A current-thread runtime only runs the other task when `request` awaits.
        /// A blocking read would leave `progressed` false until `request` returned.
        #[tokio::test(flavor = "current_thread")]
        async fn other_tasks_run_while_the_response_streams() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let client = RemoteServer::new(&format!("http://{addr}/")).unwrap();
            let progressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = Arc::clone(&progressed);

            tokio::spawn(async move {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    let n = tokio::io::AsyncReadExt::read(&mut sock, &mut tmp)
                        .await
                        .unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
                let payload = br#"{"ok":true}"#;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                tokio::io::AsyncWriteExt::write_all(&mut sock, head.as_bytes())
                    .await
                    .unwrap();
                tokio::io::AsyncWriteExt::flush(&mut sock).await.unwrap();
                tokio::time::sleep(Duration::from_millis(40)).await;
                tokio::io::AsyncWriteExt::write_all(&mut sock, &payload[..5])
                    .await
                    .unwrap();
                tokio::io::AsyncWriteExt::flush(&mut sock).await.unwrap();
                tokio::time::sleep(Duration::from_millis(40)).await;
                tokio::io::AsyncWriteExt::write_all(&mut sock, &payload[5..])
                    .await
                    .unwrap();
            });

            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });

            let body = client
                .request(
                    Method::Post,
                    "boxapi/getconf",
                    &[("last_synch", "0")],
                    auth(),
                    TIMEOUT,
                )
                .await
                .unwrap();
            assert_eq!(body["ok"], true);
            assert!(
                progressed.load(std::sync::atomic::Ordering::SeqCst),
                "no other task ran while the response was in flight"
            );
        }
    }

    mod get {
        use super::*;

        #[tokio::test]
        async fn get_puts_the_fields_in_the_query() {
            let stand = StandIn::start().await;
            let client = RemoteServer::new(&stand.base).unwrap();
            let body = client
                .request(
                    Method::Get,
                    "boxapi/barcodesinfo",
                    &[("last_synch", "1")],
                    auth(),
                    TIMEOUT,
                )
                .await
                .unwrap();
            assert_eq!(body["status"], "ok");

            let hit = stand.hit();
            assert_eq!(hit.method, "GET");
            assert_eq!(hit.path, "/boxapi/barcodesinfo");
            assert!(hit.body.is_empty());
            assert!(hit.cookie.is_none());
            let fields = form_map(&hit.query);
            assert_eq!(fields.get("last_synch").map(String::as_str), Some("1"));
            assert_eq!(fields.get("dev_id").map(String::as_str), Some("device-1"));
            assert_eq!(
                fields.get("token").map(String::as_str),
                Some("stored-token")
            );
            assert_eq!(fields.get("eid").map(String::as_str), Some("EVT1"));
            assert_eq!(fields.len(), 4);
        }

        #[tokio::test]
        async fn get_missing_credential_opens_no_socket() {
            let err = refused(
                Method::Get,
                Auth {
                    device_id: "device-1",
                    event_id: "EVT1",
                    project_token: "",
                },
            )
            .await;
            assert!(matches!(err, RemoteServerError::InvalidCred), "{err}");
            assert_eq!(err.to_string(), "invalid_cred");
        }
    }

    mod multipart {
        use super::*;
        use std::path::{Path, PathBuf};

        const PHOTO: &[u8] = b"photo-bytes-91";

        struct TempPath(PathBuf);

        impl TempPath {
            fn file(bytes: &[u8]) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-photo-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                std::fs::write(&path, bytes).unwrap();
                Self(path)
            }

            fn dir() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-photo-dir-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                std::fs::create_dir(&path).unwrap();
                Self(path)
            }
        }

        impl Drop for TempPath {
            fn drop(&mut self) {
                if self.0.is_dir() {
                    let _ = std::fs::remove_dir(&self.0);
                } else {
                    let _ = std::fs::remove_file(&self.0);
                }
            }
        }

        async fn assert_no_socket(files: &[&Path]) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let client = RemoteServer::new(&format!("http://{addr}/")).unwrap();
            let err = client
                .request(
                    Method::Multipart { files, names: &[] },
                    "boxapi/uploadphoto",
                    &[("f_1", "uid")],
                    auth(),
                    TIMEOUT,
                )
                .await
                .unwrap_err();
            let accepted = tokio::time::timeout(Duration::from_millis(50), listener.accept()).await;
            assert!(
                accepted.is_err(),
                "a path that is not a file opened a socket"
            );
            assert!(matches!(err, RemoteServerError::BadResponse(_)), "{err}");
        }

        #[tokio::test]
        async fn multipart_sends_the_file_and_the_fields() {
            let photo = TempPath::file(PHOTO);
            let stand = StandIn::start().await;
            let client = RemoteServer::new(&stand.base).unwrap();
            let body = client
                .request(
                    Method::Multipart {
                        files: &[photo.0.as_path()],
                        names: &[],
                    },
                    "boxapi/uploadphoto",
                    &[("f_1", "uid")],
                    auth(),
                    TIMEOUT,
                )
                .await
                .unwrap();
            assert_eq!(body["status"], "ok");

            let hit = stand.hit();
            assert_eq!(hit.method, "POST");
            assert_eq!(hit.path, "/boxapi/uploadphoto");
            assert!(hit.content_type.starts_with("multipart/form-data"));
            assert!(hit.cookie.is_none());
            assert!(hit.body.contains("photo-bytes-91"));
            for name in ["f_1", "dev_id", "token", "eid"] {
                assert!(
                    hit.body.contains(&format!("name=\"{name}\"")),
                    "missing part {name}"
                );
            }
            assert!(hit.body.contains("uid"));
            assert!(hit.body.contains("device-1"));
            assert!(hit.body.contains("stored-token"));
            assert!(hit.body.contains("EVT1"));
        }

        #[tokio::test]
        async fn missing_file_opens_no_socket() {
            let missing = Path::new("/no/such/rust-reg-photo.png");
            assert_no_socket(&[missing]).await;
            let dir = TempPath::dir();
            assert_no_socket(&[dir.0.as_path()]).await;
        }
    }

    mod outcome {
        use super::*;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        #[derive(Clone)]
        struct Reply {
            status: StatusCode,
            body: &'static str,
        }

        async fn post(reply: Reply) -> Result<Value, RemoteServerError> {
            let app = Router::new()
                .route("/boxapi/getconf", any(answer))
                .with_state(reply);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let client = RemoteServer::new(&format!("http://{addr}/")).unwrap();
            let result = client
                .request(
                    Method::Post,
                    "boxapi/getconf",
                    &[("last_synch", "0")],
                    auth(),
                    TIMEOUT,
                )
                .await;
            handle.abort();
            result
        }

        async fn answer(State(reply): State<Reply>) -> axum::response::Response {
            (reply.status, reply.body).into_response()
        }

        #[tokio::test]
        async fn other_error_msg_returns_the_body() {
            let body = post(Reply {
                status: StatusCode::OK,
                body: r#"{"error_msg":"no_barcodes","info":[1]}"#,
            })
            .await
            .unwrap();
            assert_eq!(body["error_msg"], "no_barcodes");
            assert_eq!(body["info"], json!([1]));
        }

        #[tokio::test]
        async fn not_logged_in_is_rejected() {
            let err = post(Reply {
                status: StatusCode::OK,
                body: r#"{"error_msg":"not_logged_in"}"#,
            })
            .await
            .unwrap_err();
            assert!(
                matches!(err, RemoteServerError::TokenRejected("not_logged_in")),
                "{err}"
            );
        }

        #[tokio::test]
        async fn no_pattern_is_rejected() {
            let err = post(Reply {
                status: StatusCode::OK,
                body: r#"{"error_msg":"no_pattern"}"#,
            })
            .await
            .unwrap_err();
            assert!(
                matches!(err, RemoteServerError::TokenRejected("no_pattern")),
                "{err}"
            );
        }

        #[tokio::test]
        async fn http_error_is_status() {
            let err = post(Reply {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: r#"{"error_msg":"not_logged_in"}"#,
            })
            .await
            .unwrap_err();
            assert!(matches!(err, RemoteServerError::Status(500)), "{err}");
        }

        #[tokio::test]
        async fn timeout_is_no_connection() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let client = RemoteServer::new(&format!("http://{addr}/")).unwrap();
            let server = tokio::spawn(async move {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await;
                tokio::time::sleep(Duration::from_secs(5)).await;
            });
            let err = client
                .request(
                    Method::Post,
                    "boxapi/getconf",
                    &[("last_synch", "0")],
                    auth(),
                    Duration::from_millis(200),
                )
                .await
                .unwrap_err();
            server.abort();
            assert!(matches!(err, RemoteServerError::NoConnection), "{err}");
            assert_eq!(err.to_string(), "no_connection");
        }

        #[tokio::test]
        async fn non_json_is_a_bad_response() {
            let err = post(Reply {
                status: StatusCode::OK,
                body: "ok",
            })
            .await
            .unwrap_err();
            assert!(matches!(err, RemoteServerError::BadResponse(_)), "{err}");
        }
    }

    mod drop_event {
        use super::*;
        use crate::registration_server::current::CurrentEvent;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::routing::get;
        use tokio::sync::watch;

        fn auth_for(event_id: &str) -> Auth<'_> {
            Auth {
                device_id: "device-1",
                event_id,
                project_token: "stored-token",
            }
        }

        struct Hold {
            base: String,
            hits: Arc<Mutex<Vec<String>>>,
            release: watch::Sender<bool>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Hold {
            async fn start() -> Self {
                let hits = Arc::new(Mutex::new(Vec::new()));
                let (release, gate) = watch::channel(false);
                let app = Router::new()
                    .route("/boxapi/held", get(held))
                    .route("/boxapi/getconf", get(ready))
                    .with_state((Arc::clone(&hits), gate));
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let handle = tokio::spawn(async move {
                    axum::serve(listener, app).await.unwrap();
                });
                Self {
                    base: format!("http://{addr}/"),
                    hits,
                    release,
                    handle,
                }
            }
        }

        impl Drop for Hold {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn held(
            State((hits, mut release)): State<(Arc<Mutex<Vec<String>>>, watch::Receiver<bool>)>,
            req: axum::extract::Request,
        ) -> impl IntoResponse {
            hits.lock().unwrap().push(req.uri().path().to_string());
            let _ = release.wait_for(|open| *open).await;
            (StatusCode::OK, r#"{"ok":true}"#)
        }

        async fn ready(
            State((hits, _release)): State<(Arc<Mutex<Vec<String>>>, watch::Receiver<bool>)>,
            req: axum::extract::Request,
        ) -> impl IntoResponse {
            hits.lock().unwrap().push(req.uri().path().to_string());
            (StatusCode::OK, r#"{"ok":true}"#)
        }

        fn scratch_dir() -> std::path::PathBuf {
            let path = std::env::temp_dir().join(format!(
                "rust-reg-drop-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        struct Scratch(std::path::PathBuf);

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        async fn wait_until(hold: &Hold, count: usize) {
            let started = tokio::time::Instant::now();
            while hold.hits.lock().unwrap().len() < count {
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("timed out waiting for {count} requests");
                }
                tokio::task::yield_now().await;
            }
        }

        fn spawn_held(
            remote: &RemoteServer,
            event_id: &'static str,
        ) -> tokio::task::JoinHandle<Result<Value, RemoteServerError>> {
            let remote = remote.clone();
            tokio::spawn(async move {
                remote
                    .request(
                        Method::Get,
                        "boxapi/held",
                        &[],
                        auth_for(event_id),
                        Duration::from_secs(5),
                    )
                    .await
            })
        }

        #[tokio::test]
        async fn drop_requests_cancels_only_that_event() {
            let hold = Hold::start().await;
            let remote = RemoteServer::new(&hold.base).unwrap();
            let aaa = spawn_held(&remote, "AAA");
            let mut bbb = spawn_held(&remote, "BBB");
            wait_until(&hold, 2).await;
            let started = tokio::time::Instant::now();
            remote.drop_requests("AAA");
            let err = aaa.await.unwrap().unwrap_err();
            assert!(matches!(err, RemoteServerError::Dropped), "{err}");
            assert!(started.elapsed() < Duration::from_millis(500));
            tokio::select! {
                result = &mut bbb => panic!("bbb ended before release: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
            let later = remote
                .request(Method::Get, "boxapi/getconf", &[], auth_for("AAA"), TIMEOUT)
                .await
                .unwrap_err();
            assert!(matches!(later, RemoteServerError::Dropped), "{later}");
            assert!(!hold
                .hits
                .lock()
                .unwrap()
                .iter()
                .any(|hit| hit == "/boxapi/getconf"));
            let _ = hold.release.send(true);
            let body = bbb.await.unwrap().unwrap();
            assert_eq!(body["ok"], true);
            let other = remote
                .request(Method::Get, "boxapi/getconf", &[], auth_for("BBB"), TIMEOUT)
                .await
                .unwrap();
            assert_eq!(other["ok"], true);
        }

        #[tokio::test]
        async fn switch_drops_the_previous_event() {
            let hold = Hold::start().await;
            let remote = RemoteServer::new(&hold.base).unwrap();
            let scratch = Scratch(scratch_dir());
            let current = CurrentEvent::new(&scratch.0);
            current.set_remote(remote.clone());
            current.switch_to("AAA").unwrap();
            let pending = spawn_held(&remote, "AAA");
            wait_until(&hold, 1).await;
            let started = tokio::time::Instant::now();
            assert_eq!(current.switch_to("BBB").unwrap(), "BBB");
            let err = pending.await.unwrap().unwrap_err();
            assert!(matches!(err, RemoteServerError::Dropped), "{err}");
            assert!(started.elapsed() < Duration::from_millis(500));
            let body = remote
                .request(Method::Get, "boxapi/getconf", &[], auth_for("BBB"), TIMEOUT)
                .await
                .unwrap();
            assert_eq!(body["ok"], true);
            let again = remote
                .request(Method::Get, "boxapi/getconf", &[], auth_for("AAA"), TIMEOUT)
                .await
                .unwrap_err();
            assert!(matches!(again, RemoteServerError::Dropped), "{again}");
            assert_eq!(current.switch_to("AAA").unwrap(), "AAA");
            let resumed = remote
                .request(Method::Get, "boxapi/getconf", &[], auth_for("AAA"), TIMEOUT)
                .await
                .unwrap();
            assert_eq!(resumed["ok"], true);
        }

        #[tokio::test]
        async fn switch_to_the_same_event_keeps_the_request() {
            let hold = Hold::start().await;
            let remote = RemoteServer::new(&hold.base).unwrap();
            let scratch = Scratch(scratch_dir());
            let current = CurrentEvent::new(&scratch.0);
            current.set_remote(remote.clone());
            current.switch_to("AAA").unwrap();
            let mut pending = spawn_held(&remote, "AAA");
            wait_until(&hold, 1).await;
            assert_eq!(current.switch_to("AAA").unwrap(), "AAA");
            tokio::select! {
                result = &mut pending => panic!("same id dropped the request: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
            let _ = hold.release.send(true);
            let body = pending.await.unwrap().unwrap();
            assert_eq!(body["ok"], true);
        }
    }

    mod bytes {
        use super::*;
        use axum::http::StatusCode;

        #[tokio::test]
        async fn bytes_that_are_not_json_are_returned() {
            let app =
                Router::new().route("/boxapi/getbg", any(|| async { (StatusCode::OK, "PK") }));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let client = RemoteServer::new(&format!("http://{addr}/")).unwrap();
            let bytes = client
                .request_bytes(Method::Get, "boxapi/getbg", &[], auth(), TIMEOUT)
                .await
                .unwrap();
            assert_eq!(bytes, b"PK");
            let err = client
                .request(Method::Get, "boxapi/getbg", &[], auth(), TIMEOUT)
                .await
                .unwrap_err();
            assert!(matches!(err, RemoteServerError::BadResponse(_)), "{err}");
            handle.abort();
        }
    }
}
