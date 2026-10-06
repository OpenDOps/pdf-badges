use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Path as AxumPath, Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH,
    LAST_MODIFIED, SET_COOKIE,
};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use cookie::{Cookie, SameSite};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::json;

use crate::registration_server::credentials::CredentialFile;
use crate::registration_server::current::{base_dir, CurrentEvent};
use crate::registration_server::net::Link;
use crate::registration_server::remote::{OperatorSession, Remote, RemoteError};

pub struct FileMeta {
    pub mtime_secs: u64,
    pub len: u64,
}

pub enum Freshness {
    Full,
    NotModified,
}

pub fn etag(mtime_secs: u64, len: u64) -> String {
    format!("\"{mtime_secs}-{len}\"")
}

pub fn conditional_get(
    meta: &FileMeta,
    if_none_match: Option<&str>,
    if_modified_since: Option<&str>,
) -> Freshness {
    let tag = etag(meta.mtime_secs, meta.len);
    if let Some(header) = if_none_match {
        return if none_match_hits(&tag, header) {
            Freshness::NotModified
        } else {
            Freshness::Full
        };
    }
    if let Some(since) = if_modified_since {
        if let Ok(parsed) = httpdate::parse_http_date(since) {
            let since_secs = system_time_secs(parsed);
            if since_secs >= meta.mtime_secs {
                return Freshness::NotModified;
            }
        }
    }
    Freshness::Full
}

pub fn safe_public_file(root: &Path, url_path: &str) -> Option<PathBuf> {
    if url_path.contains('\\') {
        return None;
    }
    let decoded = percent_decode(url_path)?;
    if decoded.contains('\\') || decoded.is_empty() || decoded == "/" || decoded.ends_with('/') {
        return None;
    }
    let relative = decoded.trim_start_matches('/');
    if relative.is_empty() {
        return None;
    }
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return None;
        }
    }
    let root = root.canonicalize().ok()?;
    let file = root.join(relative).canonicalize().ok()?;
    if !file.starts_with(&root) || !file.is_file() {
        return None;
    }
    Some(file)
}

pub fn admin_router(root: PathBuf) -> Router {
    Router::new().fallback(static_file).with_state(root)
}

fn none_match_hits(etag: &str, header: &str) -> bool {
    header.split(',').any(|part| {
        let part = part.trim();
        part == "*" || part == etag
    })
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn system_time_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn last_modified(mtime_secs: u64) -> String {
    httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(mtime_secs))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn file_meta(path: &Path) -> Option<FileMeta> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(FileMeta {
        mtime_secs: system_time_secs(modified),
        len: meta.len(),
    })
}

async fn static_file(State(root): State<PathBuf>, request: Request) -> Response {
    serve_public(&root, request).await
}

/// `GET` or `HEAD` for one file under `root`, with the same validators as [`admin_router`].
pub(crate) async fn serve_public(root: &Path, request: Request) -> Response {
    let method = request.method().clone();
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let Some(path) = safe_public_file(root, request.uri().path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let headers = request.headers().clone();
    serve_file(&path, method, &headers).await
}

/// `index.html` for `/` and `/events`, with the same validators as any other file.
pub(crate) async fn serve_index(root: &Path, request: Request) -> Response {
    let method = request.method().clone();
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let Some(path) = safe_public_file(root, "/index.html") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let headers = request.headers().clone();
    serve_file(&path, method, &headers).await
}

async fn serve_file(path: &Path, method: Method, headers: &HeaderMap) -> Response {
    let Some(meta) = file_meta(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let tag = etag(meta.mtime_secs, meta.len);
    let modified = last_modified(meta.mtime_secs);
    let if_none_match = headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    let if_modified_since = headers
        .get(IF_MODIFIED_SINCE)
        .and_then(|value| value.to_str().ok());
    let freshness = conditional_get(&meta, if_none_match, if_modified_since);
    let kind = content_type(path);
    let len = meta.len.to_string();
    let mut response = Response::builder()
        .header(ETAG, tag)
        .header(LAST_MODIFIED, modified)
        .header(CACHE_CONTROL, "no-cache")
        .header(CONTENT_TYPE, kind)
        .header(CONTENT_LENGTH, len);
    if matches!(freshness, Freshness::NotModified) {
        response = response.status(StatusCode::NOT_MODIFIED);
        return finish(response, Body::empty());
    }
    response = response.status(StatusCode::OK);
    if method == Method::HEAD {
        return finish(response, Body::empty());
    }
    match tokio::fs::read(path).await {
        Ok(bytes) => finish(response, Body::from(bytes)),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn finish(response: axum::http::response::Builder, body: Body) -> Response {
    response
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Operator half of the process: the credential file, the remote client, and
/// the in-memory sessions. A failed login leaves the session map empty.
#[derive(Clone)]
pub struct OperatorState {
    credentials: Arc<CredentialFile>,
    remote: Remote,
    sessions: Arc<Mutex<HashMap<String, OperatorLease>>>,
    pub link: Arc<Link>,
    session_ttl: Duration,
    current: Arc<Mutex<CurrentEvent>>,
    prints: Option<crate::registration_server::PrintQueue>,
}

struct OperatorLease {
    session: OperatorSession,
    expires_at: Instant,
}

impl OperatorState {
    pub fn new(credentials: CredentialFile, remote: Remote) -> Self {
        let current = Arc::new(Mutex::new(CurrentEvent::new(base_dir(credentials.path()))));
        Self {
            credentials: Arc::new(credentials),
            remote,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            link: Arc::new(Link::new()),
            session_ttl: OPERATOR_SESSION,
            current,
            prints: None,
        }
    }

    /// Print channel key badges use. Absent until the registration router attaches it.
    pub fn with_prints(mut self, prints: crate::registration_server::PrintQueue) -> Self {
        self.prints = Some(prints);
        self
    }

    /// How long an operator cookie stays valid without a session ping.
    pub fn with_session_ttl(mut self, ttl: Duration) -> Self {
        self.session_ttl = ttl;
        self
    }

    /// Use the process probe, so `/api/network` and later remote calls share one reading.
    pub fn with_link(mut self, link: Arc<Link>) -> Self {
        self.link = link;
        self
    }

    /// Share the process holder, so select and the sync thread open one event.
    pub fn with_current(mut self, current: Arc<Mutex<CurrentEvent>>) -> Self {
        self.current = current;
        self
    }

    pub fn current(&self) -> &Arc<Mutex<CurrentEvent>> {
        &self.current
    }

    pub fn credential_file(&self) -> &CredentialFile {
        &self.credentials
    }
}

/// JSON routes for the admin page. Static files stay on [`admin_router`].
pub fn api_router(state: OperatorState) -> Router {
    Router::new()
        .route("/api/login", post(login))
        .route("/api/events", get(events))
        .route("/api/events/select", post(select_event))
        .route("/api/session", post(session_ping))
        .route("/api/binding", get(binding))
        .route("/api/keys", get(keys).post(create_key))
        .route("/api/keys/{id}", patch(edit_key).delete(remove_key))
        .route("/api/keys/print", post(print_keys))
        .route("/api/network", get(network))
        .with_state(state)
}

#[derive(Deserialize)]
struct LoginBody {
    login: String,
    password: String,
}

async fn login(State(state): State<OperatorState>, body: axum::body::Bytes) -> Response {
    let budget = state.link.budget();
    let timeout_secs = budget.timeout_secs;
    let Ok(parsed) = serde_json::from_slice::<LoginBody>(&body) else {
        return login_failure(StatusCode::BAD_REQUEST, "bad_request", timeout_secs);
    };
    let login = parsed.login.trim();
    let password = parsed.password.trim();
    if login.is_empty() || password.is_empty() {
        return login_failure(StatusCode::BAD_REQUEST, "bad_request", timeout_secs);
    }
    let login = login.to_string();
    let password = password.to_string();

    let device_id = match state.credentials.load() {
        Ok(credentials) => credentials.device_id().unwrap_or("").to_string(),
        Err(_) => {
            log::debug!("login failed unknown_error");
            return login_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "unknown_error",
                timeout_secs,
            );
        }
    };
    log::debug!("login start device_id={device_id} timeout={timeout_secs}s");
    match state
        .remote
        .login_for(&device_id, &login, &password, budget.limit)
        .await
    {
        Ok(session) => {
            log::debug!("login ok");
            login_ok(&state, session, timeout_secs)
        }
        Err(err) => {
            log::debug!("login failed {err}");
            login_error(err, timeout_secs)
        }
    }
}

async fn network(State(state): State<OperatorState>) -> Response {
    (
        StatusCode::OK,
        Json(crate::registration_server::http::network_body(
            &state.link.snapshot(),
        )),
    )
        .into_response()
}

fn login_ok(state: &OperatorState, session: OperatorSession, timeout_secs: u64) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    {
        let mut sessions = state.sessions.lock().unwrap_or_else(|err| err.into_inner());
        sessions.clear();
        sessions.insert(
            id.clone(),
            OperatorLease {
                session,
                expires_at: Instant::now() + state.session_ttl,
            },
        );
    }
    let cookie = Cookie::build((OPERATOR_COOKIE, id))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .build()
        .to_string();
    (
        StatusCode::OK,
        [(SET_COOKIE, cookie)],
        Json(json!({ "ok": true, "data": { "timeout_secs": timeout_secs } })),
    )
        .into_response()
}

async fn events(State(state): State<OperatorState>, headers: HeaderMap) -> Response {
    let Some(session) = live_session(&state, &headers) else {
        return api_error(StatusCode::UNAUTHORIZED, "session_expired");
    };
    match session
        .list_events_within(1000, 0, state.link.budget().limit)
        .await
    {
        Ok(list) => {
            log::debug!("events count={}", list.len());
            let current = current_event(&state.credentials);
            api_ok(json!({
                "current": current,
                "events": list
                    .into_iter()
                    .map(|event| json!({
                        "id": event.unique_id,
                        "name": event.name,
                    }))
                    .collect::<Vec<_>>(),
            }))
        }
        Err(RemoteError::SessionExpired) => {
            log::debug!("events failed session_expired");
            drop_session(&state, &headers);
            api_error(StatusCode::UNAUTHORIZED, "session_expired")
        }
        Err(RemoteError::InProgress) => {
            log::debug!("events failed in_progress");
            api_error(StatusCode::TOO_MANY_REQUESTS, "in_progress")
        }
        Err(err) => {
            log::debug!("events failed {err}");
            let (status, code) = remote_status(&err);
            api_error(status, code)
        }
    }
}

#[derive(Deserialize)]
struct SelectBody {
    id: String,
    name: String,
}

async fn select_event(
    State(state): State<OperatorState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let budget = state.link.budget();
    let timeout_secs = budget.timeout_secs;
    let Ok(parsed) = serde_json::from_slice::<SelectBody>(&body) else {
        return select_failure(StatusCode::BAD_REQUEST, "bad_request", timeout_secs);
    };
    let id = parsed.id.trim();
    if rejected_event_id(id) {
        return select_failure(StatusCode::BAD_REQUEST, "bad_request", timeout_secs);
    }
    let id = id.to_string();
    let name = parsed.name.trim().to_string();
    let stored_name = if name.is_empty() {
        None
    } else {
        Some(name.clone())
    };

    let Some(session) = live_session(&state, &headers) else {
        return select_failure(StatusCode::UNAUTHORIZED, "session_expired", timeout_secs);
    };
    match session
        .bind_within(&state.credentials, &id, stored_name, budget.limit)
        .await
    {
        Ok(()) => {
            log::debug!("select {id} stored");
            if let Err(err) = CurrentEvent::lock(&state.current).switch_to(&id) {
                log::debug!("select {id} switch failed {err}");
                return select_failure(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unknown_error",
                    timeout_secs,
                );
            }
            api_ok(json!({
                "id": id,
                "name": name,
                "token_stored": true,
                "timeout_secs": timeout_secs,
            }))
        }
        Err(RemoteError::SessionExpired) => {
            log::debug!("select {id} failed session_expired");
            drop_session(&state, &headers);
            select_failure(StatusCode::UNAUTHORIZED, "session_expired", timeout_secs)
        }
        Err(RemoteError::InProgress) => {
            log::debug!("select {id} failed select_in_progress");
            select_failure(
                StatusCode::TOO_MANY_REQUESTS,
                "select_in_progress",
                timeout_secs,
            )
        }
        Err(err) => {
            log::debug!("select {id} failed {err}");
            let (status, code) = remote_status(&err);
            select_failure(status, code, timeout_secs)
        }
    }
}

fn rejected_event_id(id: &str) -> bool {
    id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
}

fn select_failure(status: StatusCode, code: &'static str, timeout_secs: u64) -> Response {
    (
        status,
        Json(json!({
            "ok": false,
            "error": { "code": code },
            "timeout_secs": timeout_secs,
        })),
    )
        .into_response()
}

async fn session_ping(State(state): State<OperatorState>, headers: HeaderMap) -> Response {
    if !touch_session(&state, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "session_expired");
    }
    api_ok(json!({}))
}

async fn binding(State(state): State<OperatorState>) -> Response {
    let Ok(credentials) = state.credentials.load() else {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    };
    api_ok(json!({
        "id": credentials.event_id(),
        "name": credentials.event_name(),
        "token_stored": credentials.project_token().is_some(),
    }))
}

async fn keys(State(state): State<OperatorState>, headers: HeaderMap) -> Response {
    if !desk_event_open(&state) {
        return api_ok(json!([]));
    }
    if !key_caller(&state, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match key_rows(&state) {
        Ok(rows) => api_ok(json!(rows)),
        Err(err) => {
            log::debug!("keys: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

async fn create_key(
    State(state): State<OperatorState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !desk_event_open(&state) {
        return keys_later();
    }
    if !key_caller(&state, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let comment = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("comment")
                .and_then(|item| item.as_str())
                .map(str::to_string)
        });
    match insert_operator_key(&state, comment.as_deref()) {
        Ok(row) => api_ok(row),
        Err(err) => {
            log::debug!("create key: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

async fn edit_key(
    State(state): State<OperatorState>,
    AxumPath(id): AxumPath<i64>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !desk_event_open(&state) {
        return api_error(StatusCode::CONFLICT, "no_event");
    }
    if !key_caller(&state, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let key = parsed
        .get("key")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let comment = parsed.get("comment").and_then(|item| item.as_str());
    if key.is_none() && comment.is_none() {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    match update_key(&state, id, key.as_deref(), comment) {
        Ok(true) => api_ok(json!({})),
        Ok(false) => api_error(StatusCode::NOT_FOUND, "not_found"),
        Err(err) => {
            log::debug!("edit key: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

async fn remove_key(
    State(state): State<OperatorState>,
    AxumPath(id): AxumPath<i64>,
    headers: HeaderMap,
) -> Response {
    if !desk_event_open(&state) {
        return api_error(StatusCode::CONFLICT, "no_event");
    }
    if !key_caller(&state, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match delete_key(&state, id) {
        Ok(true) => api_ok(json!({})),
        Ok(false) => api_error(StatusCode::NOT_FOUND, "not_found"),
        Err(err) => {
            log::debug!("delete key: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

async fn print_keys(State(state): State<OperatorState>, headers: HeaderMap) -> Response {
    if !desk_event_open(&state) {
        return keys_later();
    }
    if !key_caller(&state, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(prints) = state.prints.clone() else {
        return keys_later();
    };
    let Ok(rows) = key_rows(&state) else {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    };
    for row in &rows {
        let key = row.get("key").and_then(|item| item.as_str()).unwrap_or("");
        if key.is_empty() {
            continue;
        }
        if let Err(err) = prints.enqueue(crate::registration_server::PrintJob {
            id: key.to_string(),
            pages: 1,
        }) {
            log::debug!("print keys: {err:?}");
            return api_error(StatusCode::CONFLICT, "no_printer");
        }
    }
    api_ok(json!({}))
}

fn keys_later() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "ok": false,
            "error": { "code": "keys_later" }
        })),
    )
        .into_response()
}

const DESK_COOKIE: &str = "desk";

fn key_caller(state: &OperatorState, headers: &HeaderMap) -> bool {
    live_session(state, headers).is_some() || desk_row(state, headers).is_some()
}

fn desk_event_open(state: &OperatorState) -> bool {
    crate::registration_server::current::CurrentEvent::lock(state.current())
        .event_id()
        .is_some()
}

fn desk_row(state: &OperatorState, headers: &HeaderMap) -> Option<String> {
    let key = cookie_value(headers, DESK_COOKIE)?;
    let found = crate::registration_server::current::CurrentEvent::lock(state.current())
        .with_db(|db| {
            db.file()
                .query_row("SELECT key FROM keys WHERE key = ?1", [&key], |row| {
                    row.get::<_, String>(0)
                })
                .optional()
        })?
        .ok()?;
    found
}

fn key_rows(state: &OperatorState) -> Result<Vec<serde_json::Value>, rusqlite::Error> {
    crate::registration_server::current::CurrentEvent::lock(state.current())
        .with_db(|db| {
            let mut stmt = db
                .file()
                .prepare("SELECT id, key, comment, IFNULL(is_admin, 0) FROM keys ORDER BY id")?;
            let rows = stmt.query_map([], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "key": row.get::<_, String>(1)?,
                    "comment": row.get::<_, Option<String>>(2)?,
                    "is_admin": row.get::<_, i64>(3)? != 0,
                }))
            })?;
            rows.collect()
        })
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?
}

fn insert_operator_key(
    state: &OperatorState,
    comment: Option<&str>,
) -> Result<serde_json::Value, rusqlite::Error> {
    crate::registration_server::current::CurrentEvent::lock(state.current())
        .with_db(|db| {
            for _ in 0..8 {
                let key = format!("K{}", uuid::Uuid::new_v4().simple());
                match db.file().execute(
                    "INSERT INTO keys (key, is_admin, comment) VALUES (?1, 0, ?2)",
                    rusqlite::params![key, comment],
                ) {
                    Ok(_) => {
                        return Ok(json!({
                            "id": db.file().last_insert_rowid(),
                            "key": key,
                        }));
                    }
                    Err(rusqlite::Error::SqliteFailure(err, _))
                        if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                    {
                        continue;
                    }
                    Err(err) => return Err(err),
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows)
        })
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?
}

fn update_key(
    state: &OperatorState,
    id: i64,
    key: Option<&str>,
    comment: Option<&str>,
) -> Result<bool, rusqlite::Error> {
    crate::registration_server::current::CurrentEvent::lock(state.current())
        .with_db(|db| {
            let changed = if let Some(key) = key {
                db.file().execute(
                    "UPDATE keys SET key = ?1, comment = ?2 WHERE id = ?3",
                    rusqlite::params![key, comment, id],
                )?
            } else {
                db.file().execute(
                    "UPDATE keys SET comment = ?1 WHERE id = ?2",
                    rusqlite::params![comment, id],
                )?
            };
            Ok(changed > 0)
        })
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?
}

fn delete_key(state: &OperatorState, id: i64) -> Result<bool, rusqlite::Error> {
    crate::registration_server::current::CurrentEvent::lock(state.current())
        .with_db(|db| {
            let changed = db.file().execute("DELETE FROM keys WHERE id = ?1", [id])?;
            Ok(changed > 0)
        })
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?
}

const OPERATOR_SESSION: Duration = Duration::from_secs(60);

fn live_session(state: &OperatorState, headers: &HeaderMap) -> Option<OperatorSession> {
    let id = cookie_value(headers, OPERATOR_COOKIE)?;
    let mut sessions = state.sessions.lock().unwrap_or_else(|err| err.into_inner());
    let lease = sessions.get(&id)?;
    if Instant::now() >= lease.expires_at {
        sessions.remove(&id);
        return None;
    }
    Some(lease.session.clone())
}

fn touch_session(state: &OperatorState, headers: &HeaderMap) -> bool {
    let Some(id) = cookie_value(headers, OPERATOR_COOKIE) else {
        return false;
    };
    let mut sessions = state.sessions.lock().unwrap_or_else(|err| err.into_inner());
    let Some(lease) = sessions.get_mut(&id) else {
        return false;
    };
    if Instant::now() >= lease.expires_at {
        sessions.remove(&id);
        return false;
    }
    lease.expires_at = Instant::now() + state.session_ttl;
    true
}

fn drop_session(state: &OperatorState, headers: &HeaderMap) {
    let Some(id) = cookie_value(headers, OPERATOR_COOKIE) else {
        return;
    };
    let mut sessions = state.sessions.lock().unwrap_or_else(|err| err.into_inner());
    sessions.remove(&id);
}

fn current_event(file: &CredentialFile) -> serde_json::Value {
    let Ok(credentials) = file.load() else {
        return serde_json::Value::Null;
    };
    let Some(id) = credentials.event_id() else {
        return serde_json::Value::Null;
    };
    json!({
        "id": id,
        "name": credentials.event_name().unwrap_or(""),
    })
}

fn api_ok(data: serde_json::Value) -> Response {
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

const OPERATOR_COOKIE: &str = "operator";

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.trim().to_string())
    })
}

fn login_error(err: RemoteError, timeout_secs: u64) -> Response {
    if matches!(err, RemoteError::InProgress) {
        return login_failure(
            StatusCode::TOO_MANY_REQUESTS,
            "login_in_progress",
            timeout_secs,
        );
    }
    let (status, code) = remote_status(&err);
    login_failure(status, code, timeout_secs)
}

fn login_failure(status: StatusCode, code: &'static str, timeout_secs: u64) -> Response {
    (
        status,
        Json(json!({
            "ok": false,
            "error": { "code": code },
            "timeout_secs": timeout_secs,
        })),
    )
        .into_response()
}

fn remote_status(err: &RemoteError) -> (StatusCode, &'static str) {
    match err {
        RemoteError::NoDeviceId => (StatusCode::BAD_REQUEST, "no_device_id"),
        RemoteError::InvalidCred => (StatusCode::UNAUTHORIZED, "invalid_cred"),
        RemoteError::NoConnection => (StatusCode::BAD_GATEWAY, "no_connection"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "unknown_error"),
    }
}

fn api_error(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(json!({
            "ok": false,
            "error": { "code": code }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "reg-admin-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn meta_of(path: &Path) -> FileMeta {
        file_meta(path).unwrap()
    }

    mod conditional {
        use super::*;

        fn sample() -> (TempDir, PathBuf, FileMeta) {
            let dir = TempDir::new();
            let path = dir.0.join("note.txt");
            std::fs::write(&path, b"abcd").unwrap();
            let meta = meta_of(&path);
            (dir, path, meta)
        }

        #[test]
        fn etag_is_mtime_and_length() {
            let (_dir, _path, meta) = sample();
            assert_eq!(meta.len, 4);
            assert_eq!(
                etag(meta.mtime_secs, meta.len),
                format!("\"{}-4\"", meta.mtime_secs)
            );
        }

        #[test]
        fn if_none_match_is_304() {
            let (_dir, _path, meta) = sample();
            let tag = etag(meta.mtime_secs, meta.len);
            assert!(matches!(
                conditional_get(&meta, Some(&tag), None),
                Freshness::NotModified
            ));
            assert!(matches!(
                conditional_get(&meta, Some("*"), None),
                Freshness::NotModified
            ));
        }

        #[test]
        fn other_tag_is_200() {
            let (_dir, _path, meta) = sample();
            let since = last_modified(meta.mtime_secs);
            assert!(matches!(
                conditional_get(&meta, Some("\"other\""), Some(&since)),
                Freshness::Full
            ));
        }

        #[test]
        fn if_modified_since_is_304() {
            let (_dir, _path, meta) = sample();
            let since = last_modified(meta.mtime_secs);
            assert!(matches!(
                conditional_get(&meta, None, Some(&since)),
                Freshness::NotModified
            ));
            let earlier = last_modified(meta.mtime_secs.saturating_sub(1));
            assert!(matches!(
                conditional_get(&meta, None, Some(&earlier)),
                Freshness::Full
            ));
        }

        #[test]
        fn changed_file_is_200() {
            let (_dir, path, meta) = sample();
            let old = etag(meta.mtime_secs, meta.len);
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            use std::io::Write;
            file.write_all(b"wxyz").unwrap();
            let next = meta.mtime_secs + 10;
            file.set_modified(UNIX_EPOCH + Duration::from_secs(next))
                .unwrap();
            drop(file);
            let updated = meta_of(&path);
            let new_tag = etag(updated.mtime_secs, updated.len);
            assert_ne!(new_tag, old);
            assert!(matches!(
                conditional_get(&updated, Some(&old), None),
                Freshness::Full
            ));
        }
    }

    mod public_path {
        use super::*;

        fn tree() -> (TempDir, PathBuf) {
            let parent = TempDir::new();
            let root = parent.0.join("public");
            std::fs::create_dir_all(root.join("assets")).unwrap();
            std::fs::write(root.join("assets/app.js"), b"console.log(1)").unwrap();
            std::fs::write(parent.0.join("secret.txt"), b"secret").unwrap();
            (parent, root)
        }

        #[test]
        fn asset_resolves() {
            let (_parent, root) = tree();
            let file = safe_public_file(&root, "/assets/app.js").unwrap();
            assert_eq!(file, root.join("assets/app.js").canonicalize().unwrap());
        }

        #[test]
        fn parent_segment_refused() {
            let (_parent, root) = tree();
            assert!(safe_public_file(&root, "/assets/../secret.txt").is_none());
            assert!(safe_public_file(&root, "/assets/%2e%2e/secret.txt").is_none());
        }

        #[test]
        fn backslash_and_root_refused() {
            let (_parent, root) = tree();
            assert!(safe_public_file(&root, "/assets\\app.js").is_none());
            assert!(safe_public_file(&root, "/").is_none());
        }

        #[test]
        fn missing_file_refused() {
            let (_parent, root) = tree();
            assert!(safe_public_file(&root, "/assets/missing.js").is_none());
        }

        #[test]
        fn directory_refused() {
            let (_parent, root) = tree();
            assert!(safe_public_file(&root, "/assets").is_none());
        }
    }

    mod static_route {
        use super::*;
        use axum::body::to_bytes;
        use axum::http::Request;
        use tower::ServiceExt;

        fn tree() -> (TempDir, PathBuf) {
            let parent = TempDir::new();
            let root = parent.0.join("public");
            std::fs::create_dir_all(root.join("assets")).unwrap();
            std::fs::write(root.join("assets/app.js"), b"console.log(1)").unwrap();
            std::fs::write(parent.0.join("secret.txt"), b"secret").unwrap();
            (parent, root)
        }

        async fn call(root: &Path, request: Request<Body>) -> Response {
            admin_router(root.to_path_buf())
                .oneshot(request)
                .await
                .unwrap()
        }

        fn get(path: &str) -> Request<Body> {
            Request::builder().uri(path).body(Body::empty()).unwrap()
        }

        #[tokio::test]
        async fn static_get_200() {
            let (_parent, root) = tree();
            let response = call(&root, get("/assets/app.js")).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(CONTENT_TYPE).unwrap(),
                "text/javascript"
            );
            let expected = std::fs::read(root.join("assets/app.js")).unwrap();
            assert_eq!(
                response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                expected.len().to_string()
            );
            assert!(response.headers().get(ETAG).is_some());
            assert!(response.headers().get(LAST_MODIFIED).is_some());
            assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-cache");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(body.as_ref(), expected.as_slice());
        }

        #[tokio::test]
        async fn static_revalidate_304() {
            let (_parent, root) = tree();
            let first = call(&root, get("/assets/app.js")).await;
            let tag = first
                .headers()
                .get(ETAG)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let request = Request::builder()
                .uri("/assets/app.js")
                .header(IF_NONE_MATCH, tag)
                .body(Body::empty())
                .unwrap();
            let response = call(&root, request).await;
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert!(response.headers().get(ETAG).is_some());
            assert!(response.headers().get(LAST_MODIFIED).is_some());
            assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-cache");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(body.is_empty());
        }

        #[tokio::test]
        async fn static_head() {
            let (_parent, root) = tree();
            let len = std::fs::metadata(root.join("assets/app.js")).unwrap().len();
            let request = Request::builder()
                .method(Method::HEAD)
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap();
            let response = call(&root, request).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                len.to_string()
            );
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(body.is_empty());
        }

        #[tokio::test]
        async fn static_escape_404() {
            let (_parent, root) = tree();
            let response = call(&root, get("/assets/../secret.txt")).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(response.headers().get(ETAG).is_none());
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(body.is_empty());
        }

        #[tokio::test]
        async fn static_post_405() {
            let (_parent, root) = tree();
            let request = Request::builder()
                .method(Method::POST)
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap();
            let response = call(&root, request).await;
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        }
    }
}
