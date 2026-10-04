//! Remote bind: password to a short session, then a project token.
//!
//! `reqwest` performs the calls. The `cookie` crate reads `JSESSIONID` from
//! `Set-Cookie`, and later calls send that one cookie. The temptoken and the
//! session id stay in memory. `bind` writes the project token and drops them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cookie::Cookie;
use reqwest::header::SET_COOKIE;
use reqwest::Url;
use serde_json::Value;

use crate::registration_server::credentials::{CredentialError, CredentialFile};

/// Environment variable read when the registration server starts.
pub const REMOTE_ENV: &str = "REGISTRATION_REMOTE";

/// Host used when [`REMOTE_ENV`] is unset or empty.
pub const DEFAULT_REMOTE_HOST: &str = "kuprin.su";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Cap for the two login calls. A hung login holds the single-flight flag
/// until this returns, so the operator can try again. A shorter
/// [`Remote::with_timeout`] still wins.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);
const NOT_LOGGED_IN: &str = "You are not logged in!";

/// One event from `boxapi/expos/ru`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub unique_id: String,
    pub name: String,
}

/// Failure talking to the remote registration server.
#[derive(Debug)]
pub enum RemoteError {
    NoDeviceId,
    InvalidCred,
    NoConnection,
    /// The session cookie is no longer accepted.
    SessionExpired,
    /// The file already names a different device.
    DeviceMismatch {
        stored: String,
        requested: String,
    },
    BadResponse(String),
    Status(u16),
    Url(String),
    Credentials(CredentialError),
    /// Another call to the remote server is already running.
    InProgress,
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoteError::NoDeviceId => write!(f, "no_device_id"),
            RemoteError::InvalidCred => write!(f, "invalid_cred"),
            RemoteError::NoConnection => write!(f, "no_connection"),
            RemoteError::SessionExpired => write!(f, "not_logged_in"),
            RemoteError::DeviceMismatch { stored, requested } => write!(
                f,
                "credential file device_id is {stored}, bind was requested for {requested}"
            ),
            RemoteError::BadResponse(detail) => write!(f, "remote response: {detail}"),
            RemoteError::Status(code) => write!(f, "remote status {code}"),
            RemoteError::Url(detail) => write!(f, "remote url: {detail}"),
            RemoteError::Credentials(err) => write!(f, "{err}"),
            RemoteError::InProgress => write!(f, "in_progress"),
        }
    }
}

impl std::error::Error for RemoteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RemoteError::Credentials(err) => Some(err),
            _ => None,
        }
    }
}

impl From<CredentialError> for RemoteError {
    fn from(err: CredentialError) -> Self {
        RemoteError::Credentials(err)
    }
}

/// Base URL of the remote server.
///
/// `None`, empty, and whitespace use `https://kuprin.su/`. A value without a
/// scheme is an `https` host, so `kuprin.su` and `http://kuprin.su/` are the
/// same base. Any `http` URL is rewritten to `https`, except loopback, which
/// stays `http` for the local stand-in.
pub fn remote_base_url(raw: Option<&str>) -> Result<Url, RemoteError> {
    let raw = raw.map(str::trim).filter(|value| !value.is_empty());
    let raw = raw.unwrap_or(DEFAULT_REMOTE_HOST);
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let mut url = Url::parse(&with_scheme).map_err(|err| RemoteError::Url(err.to_string()))?;
    if url.host_str().is_none() {
        return Err(RemoteError::Url("remote host is missing".into()));
    }
    if url.scheme() == "http" && !is_loopback(url.host_str()) {
        url.set_scheme("https")
            .map_err(|()| RemoteError::Url("remote url must use https".into()))?;
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn is_loopback(host: Option<&str>) -> bool {
    matches!(host, Some("localhost" | "127.0.0.1" | "::1"))
}

/// [`remote_base_url`] of the `REGISTRATION_REMOTE` environment variable.
pub fn remote_base_from_env() -> Result<Url, RemoteError> {
    let raw = std::env::var(REMOTE_ENV).ok();
    remote_base_url(raw.as_deref())
}

/// HTTP client for the remote registration server.
///
/// Login, the event list, and bind share one in-flight call. A second
/// call while the first is still running returns [`RemoteError::InProgress`]
/// and does not open a socket.
#[derive(Clone, Debug)]
pub struct Remote {
    base: Url,
    timeout: Duration,
    /// One call to the remote server at a time, shared with sessions it opens.
    flight: Arc<AtomicBool>,
}

impl Remote {
    pub fn new(base: &str) -> Result<Self, RemoteError> {
        Self::with_timeout(base, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(base: &str, timeout: Duration) -> Result<Self, RemoteError> {
        Ok(Self {
            base: remote_base_url(Some(base))?,
            timeout,
            flight: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    /// Exchange the password for an in-memory session.
    ///
    /// Posts `auth/login`, reads `temptoken`, then `GET /` with that token.
    /// The temptoken is not returned and is not written to disk. `device_id`
    /// must already be provisioned; an empty id does not open a connection.
    pub async fn login(
        &self,
        device_id: &str,
        email: &str,
        password: &str,
    ) -> Result<OperatorSession, RemoteError> {
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return Err(RemoteError::NoDeviceId);
        }
        self.login_for(device_id, email, password, self.timeout.min(LOGIN_TIMEOUT))
            .await
    }

    /// Same exchange as [`Self::login`], bounded by `limit`.
    ///
    /// `limit` is capped at twenty seconds. The two hops share that budget.
    pub async fn login_for(
        &self,
        device_id: &str,
        email: &str,
        password: &str,
        limit: Duration,
    ) -> Result<OperatorSession, RemoteError> {
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return Err(RemoteError::NoDeviceId);
        }
        let email = email.trim();
        let _flight = enter(&self.flight)?;
        let limit = limit.min(LOGIN_TIMEOUT).max(Duration::from_secs(1));
        let http = session_client(self.timeout)?;
        let base = self.base.clone();
        let exchange = async {
            let temptoken = request_temptoken(&http, &base, email, password, limit).await?;
            let session_id = accept_session(&http, &base, &temptoken, email, limit).await?;
            Ok::<_, RemoteError>((http, session_id))
        };
        let (http, session_id) = match tokio::time::timeout(limit, exchange).await {
            Ok(Ok(pair)) => pair,
            Ok(Err(err)) => return Err(err),
            Err(_elapsed) => return Err(RemoteError::NoConnection),
        };
        Ok(OperatorSession {
            http,
            base: self.base.clone(),
            device_id: device_id.to_string(),
            session_id,
            flight: Arc::clone(&self.flight),
        })
    }
}

/// In-memory `JSESSIONID` for one operator.
///
/// Bind stores the project token and leaves this session in place. The local
/// operator cookie is what expires.
#[derive(Clone)]
pub struct OperatorSession {
    http: reqwest::Client,
    base: Url,
    device_id: String,
    session_id: String,
    flight: Arc<AtomicBool>,
}

impl std::fmt::Debug for OperatorSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperatorSession")
            .field("base", &self.base.as_str())
            .field("device_id", &self.device_id)
            .finish()
    }
}

impl OperatorSession {
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// `GET boxapi/expos/ru`. `name` is `name.ru.str`, or empty when that path is absent.
    pub async fn list_events(&self, limit: u64, skip: u64) -> Result<Vec<Event>, RemoteError> {
        self.list_events_within(limit, skip, DEFAULT_TIMEOUT).await
    }

    /// `GET boxapi/expos/ru`, giving up after `timeout`.
    pub async fn list_events_within(
        &self,
        limit: u64,
        skip: u64,
        timeout: Duration,
    ) -> Result<Vec<Event>, RemoteError> {
        let _flight = enter(&self.flight)?;
        let url = join(&self.base, "boxapi/expos/ru")?;
        let response = send(
            self.http
                .get(url)
                .timeout(timeout.max(Duration::from_secs(1)))
                .header(reqwest::header::COOKIE, session_cookie(&self.session_id))
                .query(&[("limit", limit.to_string()), ("skip", skip.to_string())]),
        )
        .await?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|_| RemoteError::BadResponse("event list is not json".into()))?;
        if session_expired(&body) {
            return Err(RemoteError::SessionExpired);
        }
        if !status.is_success() {
            return Err(RemoteError::Status(status.as_u16()));
        }
        let list = body
            .get("list")
            .ok_or_else(|| RemoteError::BadResponse("event list is missing".into()))?;
        let items = list
            .as_array()
            .ok_or_else(|| RemoteError::BadResponse("event list is not an array".into()))?;
        let mut events = Vec::with_capacity(items.len());
        for item in items {
            let Some(unique_id) = item.get("uniqueId").and_then(Value::as_str) else {
                continue;
            };
            let unique_id = unique_id.trim();
            if unique_id.is_empty() {
                continue;
            }
            let name = item
                .pointer("/name/ru/str")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            events.push(Event {
                unique_id: unique_id.to_string(),
                name,
            });
        }
        Ok(events)
    }

    /// `GET profile/synchdev/{device}/{event}` and store the project token.
    pub async fn bind(
        &self,
        file: &CredentialFile,
        event_id: &str,
        event_name: Option<String>,
    ) -> Result<(), RemoteError> {
        self.bind_within(file, event_id, event_name, DEFAULT_TIMEOUT)
            .await
    }

    /// Same bind as [`Self::bind`], giving up after `timeout`.
    pub async fn bind_within(
        &self,
        file: &CredentialFile,
        event_id: &str,
        event_name: Option<String>,
        timeout: Duration,
    ) -> Result<(), RemoteError> {
        let event_id = event_id.trim();
        if event_id.is_empty() {
            return Err(RemoteError::BadResponse("event id is empty".into()));
        }
        let stored = file.load()?;
        if let Some(stored_id) = stored.device_id() {
            if stored_id != self.device_id {
                return Err(RemoteError::DeviceMismatch {
                    stored: stored_id.to_string(),
                    requested: self.device_id.clone(),
                });
            }
        }

        let url = synchdev_url(&self.base, &self.device_id, event_id)?;
        let _flight = enter(&self.flight)?;
        let limit = timeout.max(Duration::from_secs(1));
        let response = send(
            self.http
                .get(url)
                .timeout(limit)
                .header(reqwest::header::COOKIE, session_cookie(&self.session_id)),
        )
        .await?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|_| RemoteError::BadResponse("bind response is not json".into()))?;
        if session_expired(&body) {
            return Err(RemoteError::SessionExpired);
        }
        if !status.is_success() {
            return Err(RemoteError::Status(status.as_u16()));
        }
        let token = body
            .get("token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| RemoteError::BadResponse("project token is missing".into()))?
            .to_string();

        let device_id = self.device_id.clone();
        let base_url = self.base.as_str().to_string();
        file.update(|credentials| {
            if credentials.device_id().is_none() {
                credentials.set_device_id(&device_id)?;
            }
            credentials.set_base_url(&base_url)?;
            credentials.bind(event_id, token, event_name)?;
            Ok(())
        })?;
        Ok(())
    }
}

/// One remote call at a time. The guard clears the flag when the call returns.
fn enter(flight: &Arc<AtomicBool>) -> Result<FlightGuard, RemoteError> {
    if flight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(RemoteError::InProgress);
    }
    Ok(FlightGuard(Arc::clone(flight)))
}

struct FlightGuard(Arc<AtomicBool>);

impl Drop for FlightGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn session_client(timeout: Duration) -> Result<reqwest::Client, RemoteError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .build()
        .map_err(|_| RemoteError::NoConnection)
}

fn session_cookie(session_id: &str) -> String {
    format!("JSESSIONID={session_id}")
}

async fn request_temptoken(
    http: &reqwest::Client,
    base: &Url,
    email: &str,
    password: &str,
    timeout: Duration,
) -> Result<String, RemoteError> {
    let url = join(base, "auth/login")?;
    let response = send(
        http.post(url)
            .timeout(timeout)
            .form(&[("email", email), ("pass", password)]),
    )
    .await?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|_| RemoteError::BadResponse("login response is not json".into()))?;
    if let Some(token) = body.get("temptoken").and_then(Value::as_str) {
        let token = token.trim();
        if !token.is_empty() && status.is_success() {
            return Ok(token.to_string());
        }
    }
    if !status.is_success() && !failing_status(body.get("status")) {
        return Err(RemoteError::Status(status.as_u16()));
    }
    Err(RemoteError::InvalidCred)
}

async fn accept_session(
    http: &reqwest::Client,
    base: &Url,
    temptoken: &str,
    email: &str,
    timeout: Duration,
) -> Result<String, RemoteError> {
    let response = send(http.get(base.clone()).timeout(timeout).query(&[
        ("ak-fivesec-token", temptoken),
        ("ak-fivesec-token-email", email),
    ]))
    .await?;
    let status = response.status();
    let session_id = response
        .headers()
        .get_all(SET_COOKIE)
        .into_iter()
        .find_map(|value| {
            let raw = value.to_str().ok()?;
            let cookie = Cookie::parse(raw).ok()?;
            if cookie.name() == "JSESSIONID" && !cookie.value().is_empty() {
                Some(cookie.value().to_string())
            } else {
                None
            }
        });
    if !status.is_success() {
        return Err(RemoteError::Status(status.as_u16()));
    }
    session_id.ok_or_else(|| RemoteError::BadResponse("JSESSIONID is missing".into()))
}

fn join(base: &Url, path: &str) -> Result<Url, RemoteError> {
    base.join(path)
        .map_err(|err| RemoteError::Url(err.to_string()))
}

fn synchdev_url(base: &Url, device_id: &str, event_id: &str) -> Result<Url, RemoteError> {
    let mut url = base.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| RemoteError::Url("remote url cannot take a path".into()))?;
        segments.pop_if_empty();
        segments.push("profile");
        segments.push("synchdev");
        segments.push(device_id);
        segments.push(event_id);
    }
    Ok(url)
}

async fn send(request: reqwest::RequestBuilder) -> Result<reqwest::Response, RemoteError> {
    request.send().await.map_err(transport_error)
}

fn transport_error(err: reqwest::Error) -> RemoteError {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        RemoteError::NoConnection
    } else {
        RemoteError::BadResponse(err.to_string())
    }
}

fn failing_status(status: Option<&Value>) -> bool {
    match status {
        Some(Value::Number(number)) => number.as_i64().is_some_and(|value| value > 0),
        Some(Value::String(text)) => text.trim().parse::<i64>().is_ok_and(|value| value > 0),
        _ => false,
    }
}

fn session_expired(body: &Value) -> bool {
    body.pointer("/status/errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| {
            errors
                .iter()
                .any(|error| error.get("msg").and_then(Value::as_str) == Some(NOT_LOGGED_IN))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration_server::credentials::Credentials;
    use axum::extract::{Path, Query};
    use axum::http::{header, HeaderMap, StatusCode};
    use axum::routing::get;
    use axum::{Json, Router};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rust-reg-remote-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Clone, Default)]
    struct Seen {
        login_body: Arc<Mutex<String>>,
        session_query: Arc<Mutex<Vec<(String, String)>>>,
    }

    struct TestServer {
        base: String,
        seen: Seen,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    impl TestServer {
        async fn start() -> Self {
            let seen = Seen::default();
            let app = Router::new()
                .route("/auth/login", axum::routing::post(login))
                .route("/", get(session))
                .route("/boxapi/expos/ru", get(events))
                .route("/profile/synchdev/{device_id}/{event_id}", get(synchdev))
                .with_state(seen.clone());
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            Self {
                base: format!("http://{addr}/"),
                seen,
                handle,
            }
        }
    }

    async fn login(State(seen): State<Seen>, body: String) -> impl axum::response::IntoResponse {
        *seen.login_body.lock().await = body.clone();
        if body.contains("pass=bad") {
            return (StatusCode::OK, Json(serde_json::json!({ "status": 1 })));
        }
        if body.contains("pass=nocookie") {
            return (
                StatusCode::OK,
                Json(serde_json::json!({ "temptoken": "temp-nocookie" })),
            );
        }
        (
            StatusCode::OK,
            Json(serde_json::json!({ "temptoken": "temp-1" })),
        )
    }

    async fn session(
        State(seen): State<Seen>,
        Query(query): Query<HashMap<String, String>>,
    ) -> impl axum::response::IntoResponse {
        let pairs: Vec<_> = query.into_iter().collect();
        *seen.session_query.lock().await = pairs.clone();
        let token = pairs
            .iter()
            .find(|(key, _)| key == "ak-fivesec-token")
            .map(|(_, value)| value.as_str());
        if token == Some("temp-nocookie") {
            return (StatusCode::OK, [(header::SET_COOKIE, "")], "ok");
        }
        (
            StatusCode::OK,
            [(header::SET_COOKIE, "JSESSIONID=sess-1; Path=/; HttpOnly")],
            "ok",
        )
    }

    async fn events(
        headers: HeaderMap,
        Query(query): Query<HashMap<String, String>>,
    ) -> impl axum::response::IntoResponse {
        if query.get("skip").map(String::as_str) == Some("9") {
            return Json(serde_json::json!({
                "status": { "errors": [{ "msg": "You are not logged in!" }] }
            }));
        }
        let cookie = headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !cookie
            .split(';')
            .any(|part| part.trim() == "JSESSIONID=sess-1")
        {
            return Json(serde_json::json!({
                "status": { "errors": [{ "msg": "You are not logged in!" }] }
            }));
        }
        Json(serde_json::json!({
            "list": [
                {
                    "uniqueId": "event-1",
                    "name": { "ru": { "str": "Мероприятие" } }
                },
                { "uniqueId": "event-2" }
            ]
        }))
    }

    async fn synchdev(
        Path((device_id, event_id)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> impl axum::response::IntoResponse {
        let cookie = headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !cookie.contains("JSESSIONID=sess-1") {
            return Json(serde_json::json!({
                "status": { "errors": [{ "msg": "You are not logged in!" }] }
            }));
        }
        Json(serde_json::json!({
            "token": format!("project-{device_id}-{event_id}")
        }))
    }

    use axum::extract::State;

    #[test]
    fn default_host_and_explicit_scheme() {
        let default = remote_base_url(None).unwrap();
        assert_eq!(default.as_str(), "https://kuprin.su/");
        assert_eq!(
            remote_base_url(Some("  ")).unwrap().as_str(),
            "https://kuprin.su/"
        );
        assert_eq!(
            remote_base_url(Some("kuprin.su")).unwrap().as_str(),
            "https://kuprin.su/"
        );
        assert_eq!(
            remote_base_url(Some("http://kuprin.su")).unwrap().as_str(),
            "https://kuprin.su/"
        );
        assert_eq!(
            remote_base_url(Some("http://stage.kuprin.su"))
                .unwrap()
                .as_str(),
            "https://stage.kuprin.su/"
        );
        assert_eq!(
            remote_base_url(Some("https://stage.kuprin.su"))
                .unwrap()
                .as_str(),
            "https://stage.kuprin.su/"
        );
        assert_eq!(
            remote_base_url(Some("http://127.0.0.1:9"))
                .unwrap()
                .as_str(),
            "http://127.0.0.1:9/"
        );
        assert!(remote_base_url(Some("http://")).is_err());
    }

    #[tokio::test]
    async fn empty_device_id_does_not_connect() {
        let remote =
            Remote::with_timeout("http://127.0.0.1:1", Duration::from_millis(200)).unwrap();
        let err = remote.login("  ", "a@b.c", "secret").await.unwrap_err();
        assert!(matches!(err, RemoteError::NoDeviceId));
    }

    #[tokio::test]
    async fn rejected_password_is_invalid_cred() {
        let server = TestServer::start().await;
        let remote = Remote::new(&server.base).unwrap();
        let err = remote
            .login("device-1", "operator@example.com", "bad")
            .await
            .unwrap_err();
        assert!(matches!(err, RemoteError::InvalidCred));
        let body = server.seen.login_body.lock().await.clone();
        assert!(body.contains("email=operator%40example.com"));
        assert!(body.contains("pass=bad"));
    }

    #[tokio::test]
    async fn missing_session_cookie_is_a_bad_response() {
        let server = TestServer::start().await;
        let remote = Remote::new(&server.base).unwrap();
        let err = remote
            .login("device-1", "operator@example.com", "nocookie")
            .await
            .unwrap_err();
        assert!(matches!(err, RemoteError::BadResponse(_)), "{err}");
    }

    #[tokio::test]
    async fn login_lists_and_bind_stores_only_the_project_token() {
        let server = TestServer::start().await;
        let remote = Remote::new(&server.base).unwrap();
        let session = remote
            .login("device-1", " operator@example.com ", "secret-pass")
            .await
            .unwrap();
        let query = server.seen.session_query.lock().await.clone();
        assert!(query.contains(&("ak-fivesec-token".into(), "temp-1".into())));
        assert!(query.contains(&(
            "ak-fivesec-token-email".into(),
            "operator@example.com".into()
        )));

        let list = session.list_events(1000, 0).await.unwrap();
        assert_eq!(
            list,
            vec![
                Event {
                    unique_id: "event-1".into(),
                    name: "Мероприятие".into(),
                },
                Event {
                    unique_id: "event-2".into(),
                    name: String::new(),
                },
            ]
        );

        let scratch = Scratch::new();
        let path = scratch.0.join("credentials.yml");
        let file = CredentialFile::new(&path);
        file.update(|credentials| {
            credentials.set_device_id("device-1")?;
            Ok(())
        })
        .unwrap();
        session
            .bind(&file, "event-1", Some("Мероприятие".into()))
            .await
            .unwrap();

        let stored = CredentialFile::new(&path).load().unwrap();
        assert!(stored.is_logged_in());
        assert_eq!(stored.device_id(), Some("device-1"));
        assert_eq!(stored.event_id(), Some("event-1"));
        assert_eq!(stored.event_name(), Some("Мероприятие"));
        assert_eq!(stored.project_token(), Some("project-device-1-event-1"));
        assert_eq!(stored.base_url(), Some(remote.base().as_str()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("JSESSIONID"));
        assert!(!text.contains("temp-1"));
        assert!(!text.contains("secret-pass"));
    }

    #[tokio::test]
    async fn expired_cookie_on_the_list_is_not_logged_in() {
        let server = TestServer::start().await;
        let remote = Remote::new(&server.base).unwrap();
        let session = remote
            .login("device-1", "operator@example.com", "secret-pass")
            .await
            .unwrap();
        let err = session.list_events(10, 9).await.unwrap_err();
        assert!(matches!(err, RemoteError::SessionExpired));
    }

    #[tokio::test]
    async fn connection_refused_is_no_connection() {
        let remote =
            Remote::with_timeout("http://127.0.0.1:1", Duration::from_millis(200)).unwrap();
        let err = remote
            .login("device-1", "a@b.c", "secret")
            .await
            .unwrap_err();
        assert!(matches!(err, RemoteError::NoConnection), "{err}");
    }

    #[tokio::test]
    async fn bind_refuses_a_different_device_id_without_calling_the_server() {
        let server = TestServer::start().await;
        let remote = Remote::new(&server.base).unwrap();
        let session = remote
            .login("device-1", "operator@example.com", "secret-pass")
            .await
            .unwrap();
        let scratch = Scratch::new();
        let path = scratch.0.join("credentials.yml");
        let file = CredentialFile::new(&path);
        let mut credentials = Credentials::default();
        credentials.set_device_id("other-device").unwrap();
        file.store(&credentials).unwrap();

        let err = session.bind(&file, "event-1", None).await.unwrap_err();
        assert!(matches!(err, RemoteError::DeviceMismatch { .. }), "{err}");
        let stored = file.load().unwrap();
        assert_eq!(stored.device_id(), Some("other-device"));
        assert!(!stored.is_logged_in());
    }
}
