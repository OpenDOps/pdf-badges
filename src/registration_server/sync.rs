//! The event-sync queue. A host binds a store, a transport, and a catalog.
//! The registration server is the first host. Other hosts and the C ABI are
//! described in `docs/event-sync/design.md`.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::sync::watch;

use crate::registration_server::current::{CurrentError, CurrentEvent};
use crate::registration_server::event_db::{
    OutgoingAttachment, OutgoingScan, ScanWrite, VisitorWrite, WaitingRegistration,
};
use crate::registration_server::gate::Gate;
use crate::registration_server::remote_server::{Auth, Method, RemoteServer, RemoteServerError};
use crate::registration_server::store::{Registration, Store};
use crate::registration_server::App;

/// Remote calls that may wait on a response at once. Parsing stays one body at a time.
///
/// Compiled from the `SYNC_IN_FLIGHT` environment variable. Unset is 100.
/// `--sync-in-flight` replaces this for that process.
pub const SYNC_IN_FLIGHT: usize = parse_sync_in_flight(env!("SYNC_IN_FLIGHT"));

const fn parse_sync_in_flight(raw: &str) -> usize {
    let bytes = raw.as_bytes();
    let mut n: usize = 0;
    let mut i = 0;
    if bytes.is_empty() {
        panic!("SYNC_IN_FLIGHT is empty");
    }
    while i < bytes.len() {
        let digit = bytes[i];
        if !digit.is_ascii_digit() {
            panic!("SYNC_IN_FLIGHT is not a positive integer");
        }
        n = match n.checked_mul(10) {
            Some(n) => n,
            None => panic!("SYNC_IN_FLIGHT overflows usize"),
        };
        n = match n.checked_add((digit - b'0') as usize) {
            Some(n) => n,
            None => panic!("SYNC_IN_FLIGHT overflows usize"),
        };
        i += 1;
    }
    if n == 0 {
        panic!("SYNC_IN_FLIGHT must be greater than zero");
    }
    n
}
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
enum DownloadError {
    Http(reqwest::Error),
    Status(reqwest::StatusCode),
    TooLarge,
    Parse(serde_json::Error),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Http(err) => write!(f, "{err}"),
            DownloadError::Status(status) => write!(f, "remote status {status}"),
            DownloadError::TooLarge => write!(f, "body exceeds {MAX_BODY_BYTES} bytes"),
            DownloadError::Parse(err) => write!(f, "{err}"),
        }
    }
}

impl From<reqwest::Error> for DownloadError {
    fn from(err: reqwest::Error) -> Self {
        DownloadError::Http(err)
    }
}

pub fn client(in_flight: usize) -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(in_flight)
        .build()
        .expect("registration sync client")
}

pub async fn sync_loop(
    app: App,
    endpoints: Vec<String>,
    interval: Duration,
    in_flight: usize,
    remote: RemoteServer,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                log::debug!("sync cycle endpoints={}", endpoints.len());
                let timeout = app.link.budget().limit;
                sync_wake(&app, &remote, &endpoints, timeout, in_flight).await;
            }
            _ = app.sync_notify.notified() => {
                log::debug!("sync wake requested");
                let timeout = app.link.budget().limit;
                sync_wake(&app, &remote, &endpoints, timeout, in_flight).await;
            }
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

/// Pump, then probe the stored token. An accepted probe drains the catalog.
/// A wake that starts while one is already running does not open a socket.
async fn sync_wake(
    app: &App,
    remote: &RemoteServer,
    _endpoints: &[String],
    timeout: Duration,
    in_flight: usize,
) {
    let Some(_busy) = Busy::enter(&app.sync_running) else {
        return;
    };
    pump_open(app);
    app.gate.wait_until_idle().await;
    if probe(app, remote, timeout).await == Probe::Accepted {
        drain_queue(app, remote, catalog(app), in_flight, timeout).await;
    }
    app.sync_passes.fetch_add(1, Ordering::Release);
}

struct Busy<'a>(&'a AtomicBool);

impl Busy<'_> {
    fn enter(flag: &AtomicBool) -> Option<Busy<'_>> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Busy(flag))
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Probe {
    Accepted,
    Stopped,
}

/// `GET boxapi/barcodesinfo` when the open event is the one in the credential file.
///
/// The `info` array is not read. `TokenRejected` clears the binding. Any other
/// error leaves the binding in place. Either failure skips the rest of the wake.
async fn probe(app: &App, remote: &RemoteServer, timeout: Duration) -> Probe {
    let loaded = match app.operator.credential_file().load() {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("registration-server probe: {err}");
            return Probe::Stopped;
        }
    };
    let (device_id, event_id, token) = match (
        loaded.device_id(),
        loaded.event_id(),
        loaded.project_token(),
    ) {
        (Some(device_id), Some(event_id), Some(token)) if loaded.is_logged_in() => (
            device_id.to_string(),
            event_id.to_string(),
            token.to_string(),
        ),
        _ => return Probe::Stopped,
    };
    let open = CurrentEvent::lock(&app.current).event_id();
    if open.as_deref() != Some(event_id.as_str()) {
        return Probe::Stopped;
    }
    match remote
        .request(
            Method::Get,
            "boxapi/barcodesinfo",
            &[],
            Auth {
                device_id: &device_id,
                event_id: &event_id,
                project_token: &token,
            },
            timeout,
        )
        .await
    {
        Ok(_) => Probe::Accepted,
        Err(RemoteServerError::TokenRejected(_)) => {
            remote.drop_requests(&event_id);
            if let Err(err) = CurrentEvent::lock(&app.current).clear(app.operator.credential_file())
            {
                eprintln!("registration-server clear: {err}");
            }
            Probe::Stopped
        }
        Err(
            RemoteServerError::InvalidCred
            | RemoteServerError::NoConnection
            | RemoteServerError::Status(_)
            | RemoteServerError::BadResponse(_)
            | RemoteServerError::Url(_)
            | RemoteServerError::Dropped,
        ) => Probe::Stopped,
    }
}

/// One batch, then the mutex is released before any download starts.
fn pump_open(app: &App) {
    if let Err(err) = CurrentEvent::lock(&app.current).pump() {
        eprintln!("registration-server pump: {err}");
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Download,
    Upload,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CallMethod {
    Get,
    Post,
    Multipart,
}

/// Cursor sent on the call and stored from the response after that body commits.
struct Cursor {
    name: String,
    send_as: &'static str,
    take_from: &'static str,
}

struct Job {
    lane: Lane,
    method: CallMethod,
    path: String,
    event_id: String,
    fields: Vec<(String, String)>,
    /// Each cursor is sent on the call and stored from the response after commit.
    cursors: Vec<Cursor>,
    /// The next call of this same path. Appended after this body commits.
    /// A different path does not wait for it.
    after: Vec<Job>,
    hold_parse: Option<watch::Receiver<bool>>,
    log: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    /// Multipart file parts. Empty for a form call. Several photo calls may be queued together.
    files: Vec<(String, PathBuf)>,
}

struct Sent {
    job: Job,
    result: Result<Vec<u8>, RemoteServerError>,
}

/// Send ready calls through [`RemoteServer`]. The ready queue and the sockets
/// share `cap`. Downloads of one path stay in order. A free socket takes a
/// download before an upload. Parses run one at a time, after the gate is idle.
async fn drain_queue(
    app: &App,
    remote: &RemoteServer,
    jobs: Vec<Job>,
    cap: usize,
    timeout: Duration,
) -> Vec<(String, &'static str)> {
    let Some(creds) = load_creds(app) else {
        return Vec::new();
    };
    if cap == 0 {
        return Vec::new();
    }
    let mut planner: VecDeque<Job> = jobs.into();
    let mut ready: VecDeque<Job> = VecDeque::new();
    let mut flying = FuturesUnordered::new();
    let mut parse_q: VecDeque<(Job, Vec<u8>)> = VecDeque::new();
    let mut path_busy: HashSet<String> = HashSet::new();
    let mut stop = false;
    let mut outcomes = Vec::new();

    loop {
        if stop {
            ready.clear();
            planner.clear();
        } else {
            discard_stale(app, &mut ready, &mut planner);
            queue_ready(&mut planner, &mut ready, &path_busy, cap);
            send_ready(
                app,
                remote,
                &creds,
                timeout,
                cap,
                &mut ready,
                &mut flying,
                &mut path_busy,
            );
        }

        if !parse_q.is_empty() {
            app.gate.wait_until_idle().await;
            let Some((job, bytes)) = parse_q.pop_front() else {
                continue;
            };
            parse_job(app, &job, &bytes).await;
            if tracks_path(&job) {
                path_busy.remove(&job.path);
            }
            let path = job.path.clone();
            if !stop {
                planner.extend(follow_ups(app, job, &bytes));
            }
            outcomes.push((path, "ok"));
            continue;
        }

        if flying.is_empty() {
            if stop || !has_sendable(&ready, &path_busy) {
                break;
            }
            continue;
        }

        let Some(sent) = flying.next().await else {
            break;
        };
        match sent.result {
            Ok(bytes) if soft_error(&bytes) => {
                stop = true;
                ready.clear();
                planner.clear();
                if tracks_path(&sent.job) {
                    path_busy.remove(&sent.job.path);
                }
                outcomes.push((sent.job.path, "soft"));
            }
            Ok(bytes) => parse_q.push_back((sent.job, bytes)),
            Err(RemoteServerError::TokenRejected(_)) => {
                stop = true;
                ready.clear();
                planner.clear();
                remote.drop_requests(&sent.job.event_id);
                if let Err(err) =
                    CurrentEvent::lock(&app.current).clear(app.operator.credential_file())
                {
                    eprintln!("registration-server clear: {err}");
                }
                if tracks_path(&sent.job) {
                    path_busy.remove(&sent.job.path);
                }
                outcomes.push((sent.job.path, "rejected"));
            }
            Err(RemoteServerError::Dropped) => {
                stop = true;
                ready.clear();
                planner.clear();
                if tracks_path(&sent.job) {
                    path_busy.remove(&sent.job.path);
                }
                outcomes.push((sent.job.path, "dropped"));
            }
            Err(_) => {
                stop = true;
                ready.clear();
                planner.clear();
                if tracks_path(&sent.job) {
                    path_busy.remove(&sent.job.path);
                }
                outcomes.push((sent.job.path, "error"));
            }
        }
    }
    outcomes
}

struct Creds {
    device_id: String,
    token: String,
}

fn load_creds(app: &App) -> Option<Creds> {
    let loaded = app.operator.credential_file().load().ok()?;
    Some(Creds {
        device_id: loaded.device_id()?.to_string(),
        token: loaded.project_token()?.to_string(),
    })
}

fn discard_stale(app: &App, ready: &mut VecDeque<Job>, planner: &mut VecDeque<Job>) {
    let open = CurrentEvent::lock(&app.current).event_id();
    let keep = |job: &Job| open.as_deref() == Some(job.event_id.as_str());
    ready.retain(keep);
    planner.retain(keep);
}

/// One download path has at most one call queued, on a socket, or still being parsed.
/// The same holds for `uploadreg`, `uploadscans`, and `leadgenapi/upload`.
/// A later call of that path stays in the planner until this one finishes.
/// `uploadphoto` calls for different ids may sit in the queue together.
/// Any other path may take a free socket.
fn endpoint_busy(path: &str, path_busy: &HashSet<String>) -> bool {
    path_busy.contains(path)
}

/// Downloads, and upload paths whose next batch waits for this one to commit.
fn tracks_path(job: &Job) -> bool {
    job.lane == Lane::Download
        || matches!(
            endpoint(&job.path),
            "boxapi/uploadreg" | "boxapi/uploadscans" | "leadgenapi/upload"
        )
}

fn path_is_open(job: &Job, ready: &VecDeque<Job>, path_busy: &HashSet<String>) -> bool {
    if !tracks_path(job) {
        return true;
    }
    !endpoint_busy(&job.path, path_busy)
        && !ready
            .iter()
            .any(|queued| queued.path == job.path && tracks_path(queued))
}

/// Move planner calls into the ready queue. A download whose path already has
/// a call queued or on a socket is left in the planner.
fn queue_ready(
    planner: &mut VecDeque<Job>,
    ready: &mut VecDeque<Job>,
    path_busy: &HashSet<String>,
    cap: usize,
) {
    let mut held = VecDeque::new();
    while ready.len() < cap {
        let Some(job) = planner.pop_front() else {
            break;
        };
        if !path_is_open(&job, ready, path_busy) {
            held.push_back(job);
            if !planner
                .iter()
                .any(|waiting| path_is_open(waiting, ready, path_busy))
            {
                break;
            }
            continue;
        }
        ready.push_back(job);
    }
    while let Some(job) = held.pop_back() {
        planner.push_front(job);
    }
}

fn has_sendable(ready: &VecDeque<Job>, path_busy: &HashSet<String>) -> bool {
    ready.iter().any(|job| match job.lane {
        Lane::Download => !endpoint_busy(&job.path, path_busy),
        Lane::Upload => {
            !ready.iter().any(|other| other.lane == Lane::Download)
                && !endpoint_busy(&job.path, path_busy)
        }
    })
}

fn send_ready(
    app: &App,
    remote: &RemoteServer,
    creds: &Creds,
    timeout: Duration,
    cap: usize,
    ready: &mut VecDeque<Job>,
    flying: &mut FuturesUnordered<
        std::pin::Pin<Box<dyn std::future::Future<Output = Sent> + Send>>,
    >,
    path_busy: &mut HashSet<String>,
) {
    while flying.len() < cap {
        let download = ready
            .iter()
            .position(|job| job.lane == Lane::Download && !endpoint_busy(&job.path, path_busy));
        if let Some(index) = download {
            let job = ready.remove(index).expect("download index");
            spawn_send(app, remote, creds, timeout, job, flying, path_busy);
            continue;
        }
        if ready.iter().any(|job| job.lane == Lane::Download) {
            break;
        }
        let upload = ready
            .iter()
            .position(|job| job.lane == Lane::Upload && !endpoint_busy(&job.path, path_busy));
        let Some(index) = upload else {
            break;
        };
        let job = ready.remove(index).expect("upload index");
        spawn_send(app, remote, creds, timeout, job, flying, path_busy);
    }
}

fn spawn_send(
    app: &App,
    remote: &RemoteServer,
    creds: &Creds,
    timeout: Duration,
    mut job: Job,
    flying: &mut FuturesUnordered<
        std::pin::Pin<Box<dyn std::future::Future<Output = Sent> + Send>>,
    >,
    path_busy: &mut HashSet<String>,
) {
    let open = CurrentEvent::lock(&app.current).event_id();
    if open.as_deref() != Some(job.event_id.as_str()) {
        return;
    }
    if !fill_upload(app, &mut job) {
        return;
    }
    let mut pairs = job.fields.clone();
    for cursor in &job.cursors {
        if cursor.send_as.is_empty() {
            continue;
        }
        match CurrentEvent::lock(&app.current).cursor(&job.event_id, &cursor.name) {
            Ok(value) => pairs.push((cursor.send_as.to_string(), value)),
            Err(_) => return,
        }
    }
    if tracks_path(&job) {
        path_busy.insert(job.path.clone());
    }
    let remote = remote.clone();
    let device_id = creds.device_id.clone();
    let token = creds.token.clone();
    let path = job.path.clone();
    let event_id = job.event_id.clone();
    let call = job.method;
    let photo_files = job.files.clone();
    flying.push(Box::pin(async move {
        let fields: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let paths: Vec<&Path> = photo_files.iter().map(|(_, file)| file.as_path()).collect();
        let names: Vec<&str> = photo_files.iter().map(|(name, _)| name.as_str()).collect();
        let method = match call {
            CallMethod::Get => Method::Get,
            CallMethod::Post => Method::Post,
            CallMethod::Multipart => Method::Multipart {
                files: &paths,
                names: &names,
            },
        };
        let result = remote
            .request_bytes(
                method,
                &path,
                &fields,
                Auth {
                    device_id: &device_id,
                    event_id: &event_id,
                    project_token: &token,
                },
                timeout,
            )
            .await;
        Sent { job, result }
    }));
}

async fn parse_job(app: &App, job: &Job, bytes: &[u8]) {
    if let Some(log) = &job.log {
        log.lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(format!("start {}", job.path));
    }
    if let Some(hold) = &job.hold_parse {
        let mut hold = hold.clone();
        let _ = hold.wait_for(|go| *go).await;
    }
    for cursor in &job.cursors {
        if let Some(value) = cursor_value(bytes, cursor.take_from) {
            if let Err(err) =
                CurrentEvent::lock(&app.current).set_cursor(&job.event_id, &cursor.name, &value)
            {
                eprintln!("registration-server cursor: {err}");
            }
        }
    }
    if endpoint(&job.path) == "boxapi/ticketbarcodes" {
        store_ticket_barcodes(app, job, bytes);
    }
    if endpoint(&job.path) == "boxapi/barcodes" {
        store_barcodes(app, job, bytes);
    }
    if endpoint(&job.path) == "tracked/UniRegUser" {
        store_tracked(app, job, bytes, VISITOR_CURSOR);
    }
    if endpoint(&job.path) == "tracked/OrgRegUser" {
        store_tracked(app, job, bytes, ORG_CURSOR);
    }
    if endpoint(&job.path) == "tracked/ScanUserRef" {
        store_scans(app, job, bytes);
    }
    if endpoint(&job.path) == "boxapi/getconf" && field(job, "conf_id").is_empty() {
        remember_zone(app, job, bytes);
    }
    if endpoint(&job.path) == "boxapi/uploadreg" {
        accept_uploaded(app, job, bytes);
    }
    if endpoint(&job.path) == "boxapi/uploadscans" {
        accept_scans(app, job, bytes);
    }
    if endpoint(&job.path) == "leadgenapi/upload" {
        accept_attachments(app, job, bytes);
    }
    if let Some(log) = &job.log {
        log.lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(format!("store {}", job.path));
    }
}

fn cursor_value(bytes: &[u8], take_from: &str) -> Option<String> {
    if take_from == "forms.change" {
        let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
        return json_text(value.get("forms")?.get("change")?);
    }
    json_field(bytes, take_from)
}

fn json_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn json_field(bytes: &[u8], key: &str) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    json_text(value.get(key)?)
}

const ENUM_NAMES: &str = "country,region,city";
const ENUMS_LANGS: &str = "ru,en,phone_code,phone_mask";
const ENUM_LANGS: &str = "ru,en,phone_mask";

/// Calls ready for the open event. Same-path downloads are not duplicated here:
/// the next call of a path is appended by [`follow_ups`] after the previous one commits.
fn catalog(app: &App) -> Vec<Job> {
    let Some(event_id) = CurrentEvent::lock(&app.current).event_id() else {
        return Vec::new();
    };
    let mut jobs = vec![
        dbenums_job(&event_id),
        empty_getconf(&event_id),
        zones_job(&event_id),
        info_job(&event_id),
        visitors_job(&event_id),
    ];
    if let Some(org_id) = stored_id(app, &event_id, ORG_ID_CURSOR) {
        jobs.push(managers_job(&event_id, &org_id));
        jobs.push(org_users_job(&event_id, &org_id));
    }
    if let Some(zone_id) = stored_id(app, &event_id, ZONE_CURSOR) {
        jobs.push(scans_job(&event_id, &zone_id));
    }
    if has_waiting(app, &event_id) {
        jobs.push(uploadreg_job(&event_id));
    }
    if has_outgoing_scans(app, &event_id) {
        jobs.push(uploadscans_job(&event_id));
    }
    if has_outgoing_attachments(app, &event_id) {
        jobs.push(upload_attachments_job(&event_id));
    }
    jobs
}

fn dbenums_job(event_id: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/dbenums".to_string(),
        event_id: event_id.to_string(),
        fields: vec![
            ("enums".to_string(), ENUM_NAMES.to_string()),
            ("langs".to_string(), ENUMS_LANGS.to_string()),
        ],
        cursors: vec![Cursor {
            name: "dbenums".to_string(),
            send_as: "version",
            take_from: "version",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

struct EnumDep {
    name: String,
    ids: Vec<(String, String)>,
}

fn follow_ups(app: &App, job: Job, bytes: &[u8]) -> Vec<Job> {
    let mut next = match endpoint(&job.path) {
        "boxapi/dbenums" => first_dbenum(&job.event_id, bytes),
        "boxapi/getconf" if field(&job, "conf_id").is_empty() => {
            let mut heads = form_heads(&job.event_id, bytes);
            heads.extend(badge_head(&job.event_id, bytes));
            heads
        }
        "boxapi/barcodesinfo" => barcode_head(app, &job.event_id, bytes),
        "tracked/UniRegUser"
            if tracked_page_continues(app, &job.event_id, bytes, VISITOR_CURSOR) =>
        {
            vec![visitors_job(&job.event_id)]
        }
        "tracked/OrgRegUser" if tracked_page_continues(app, &job.event_id, bytes, ORG_CURSOR) => {
            vec![org_users_job(&job.event_id, field(&job, "extraId"))]
        }
        "tracked/ScanUserRef" if scan_page_continues(app, &job.event_id, bytes) => {
            vec![scans_job(&job.event_id, zone_of(&job))]
        }
        "boxapi/uploadreg" => uploadreg_follow(app, &job, bytes),
        "boxapi/uploadscans" if more_scans(app, &job) => vec![uploadscans_job(&job.event_id)],
        "leadgenapi/upload" if more_attachments(app, &job) => {
            vec![upload_attachments_job(&job.event_id)]
        }
        _ => Vec::new(),
    };
    next.extend(job.after);
    next
}

const BC_MIN: i64 = 150;
const BC_ADD: i64 = 50;
const BC_TAKE_MIN: i64 = 10;

const VISITOR_PAGE: usize = 100;
const VISITOR_CURSOR: &str = "visitors";
const ORG_CURSOR: &str = "org_users";
const ORG_ID_CURSOR: &str = "org_id";
const ZONE_CURSOR: &str = "zone";
const SCAN_CURSOR: &str = "scans";

fn visitors_job(event_id: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "tracked/UniRegUser".to_string(),
        event_id: event_id.to_string(),
        fields: vec![
            ("json".to_string(), format!("{event_id}_reg_track_request")),
            ("limit".to_string(), VISITOR_PAGE.to_string()),
        ],
        cursors: vec![Cursor {
            name: VISITOR_CURSOR.to_string(),
            send_as: "from_time",
            take_from: "from_time",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn store_tracked(app: &App, job: &Job, bytes: &[u8], cursor: &str) {
    let Some(page) = visitor_page(bytes) else {
        return;
    };
    for visitor in &page.visitors {
        let written = CurrentEvent::lock(&app.current).write(&job.event_id, visitor);
        match written {
            Ok(()) => {
                let noted =
                    CurrentEvent::lock(&app.current).note_on_server(&job.event_id, &visitor.uid);
                if let Err(err) = noted {
                    if matches!(err, CurrentError::NotOpen { .. }) {
                        note(job, "NotOpen");
                    } else {
                        eprintln!("registration-server visitors: {err}");
                    }
                    return;
                }
            }
            Err(CurrentError::NotOpen { .. }) => {
                note(job, "NotOpen");
                return;
            }
            Err(err) => {
                eprintln!("registration-server visitors: {err}");
                return;
            }
        }
    }
    let Some(from_time) = page.from_time else {
        return;
    };
    if let Err(err) =
        CurrentEvent::lock(&app.current).set_cursor(&job.event_id, cursor, &from_time.to_string())
    {
        if matches!(err, CurrentError::NotOpen { .. }) {
            note(job, "NotOpen");
        } else {
            eprintln!("registration-server visitors: {err}");
        }
    }
}

fn note(job: &Job, event: &str) {
    if let Some(log) = &job.log {
        log.lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(event.to_string());
    }
}

fn tracked_page_continues(app: &App, event_id: &str, bytes: &[u8], cursor: &str) -> bool {
    let Some(page) = visitor_page(bytes) else {
        return false;
    };
    if page.visitors.len() != VISITOR_PAGE {
        return false;
    }
    let Some(from_time) = page.from_time else {
        return false;
    };
    CurrentEvent::lock(&app.current)
        .cursor(event_id, cursor)
        .ok()
        .is_some_and(|stored| stored == from_time.to_string())
}

fn stored_id(app: &App, event_id: &str, name: &str) -> Option<String> {
    let value = CurrentEvent::lock(&app.current)
        .cursor(event_id, name)
        .ok()?;
    if value.is_empty() || value == "0" {
        None
    } else {
        Some(value)
    }
}

fn has_waiting(app: &App, event_id: &str) -> bool {
    CurrentEvent::lock(&app.current)
        .has_waiting_registration(event_id)
        .unwrap_or(false)
}

fn remember_zone(app: &App, job: &Job, bytes: &[u8]) {
    let Some(zone) = json_field(bytes, "zone") else {
        return;
    };
    if zone.is_empty() || zone == "0" {
        return;
    }
    if let Err(err) = CurrentEvent::lock(&app.current).set_cursor(&job.event_id, ZONE_CURSOR, &zone)
    {
        eprintln!("registration-server zone: {err}");
    }
}

fn managers_job(event_id: &str, org_id: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/getmanagers".to_string(),
        event_id: event_id.to_string(),
        fields: vec![("orgid".to_string(), org_id.to_string())],
        cursors: vec![Cursor {
            name: "getmanagers".to_string(),
            send_as: "last_synch",
            take_from: "last_synch",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn org_users_job(event_id: &str, org_id: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "tracked/OrgRegUser".to_string(),
        event_id: event_id.to_string(),
        fields: vec![
            ("json".to_string(), format!("{event_id}_reg_track_request")),
            ("extraId".to_string(), org_id.to_string()),
            ("limit".to_string(), VISITOR_PAGE.to_string()),
        ],
        cursors: vec![Cursor {
            name: ORG_CURSOR.to_string(),
            send_as: "from_time",
            take_from: "from_time",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn scans_job(event_id: &str, zone_id: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "tracked/ScanUserRef".to_string(),
        event_id: event_id.to_string(),
        fields: vec![
            (
                "json".to_string(),
                format!("{event_id}_regscan_track_request_short"),
            ),
            ("extraId".to_string(), format!("zone_{zone_id}")),
            ("limit".to_string(), VISITOR_PAGE.to_string()),
        ],
        cursors: vec![Cursor {
            name: SCAN_CURSOR.to_string(),
            send_as: "from_time",
            take_from: "from_time",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn uploadreg_job(event_id: &str) -> Job {
    Job {
        lane: Lane::Upload,
        method: CallMethod::Post,
        path: "boxapi/uploadreg".to_string(),
        event_id: event_id.to_string(),
        fields: Vec::new(),
        cursors: Vec::new(),
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn uploadscans_job(event_id: &str) -> Job {
    Job {
        lane: Lane::Upload,
        method: CallMethod::Post,
        path: "boxapi/uploadscans".to_string(),
        event_id: event_id.to_string(),
        fields: Vec::new(),
        cursors: Vec::new(),
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn upload_attachments_job(event_id: &str) -> Job {
    Job {
        lane: Lane::Upload,
        method: CallMethod::Post,
        path: "leadgenapi/upload".to_string(),
        event_id: event_id.to_string(),
        fields: Vec::new(),
        cursors: Vec::new(),
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn photo_job(event_id: &str, id: &str, path: PathBuf) -> Job {
    Job {
        lane: Lane::Upload,
        method: CallMethod::Multipart,
        path: "boxapi/uploadphoto".to_string(),
        event_id: event_id.to_string(),
        fields: Vec::new(),
        cursors: Vec::new(),
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: vec![(format!("f_{id}"), path)],
    }
}

const REG_BATCH: usize = 20;
const SCAN_BATCH: usize = 100;
const ATTACH_BATCH: usize = 20;

fn fill_upload(app: &App, job: &mut Job) -> bool {
    match endpoint(&job.path) {
        "boxapi/uploadreg" => fill_registrations(app, job),
        "boxapi/uploadscans" => fill_scans(app, job),
        "leadgenapi/upload" => fill_attachments(app, job),
        "boxapi/uploadphoto" => job.files.iter().all(|(_, path)| path.is_file()),
        _ => true,
    }
}

fn fill_registrations(app: &App, job: &mut Job) -> bool {
    let Ok(rows) = CurrentEvent::lock(&app.current).waiting_registrations(&job.event_id, REG_BATCH)
    else {
        return false;
    };
    if rows.is_empty() {
        return false;
    }
    job.fields.retain(|(name, _)| name != "vals");
    job.fields
        .push(("vals".to_string(), registration_vals(&rows)));
    true
}

fn fill_scans(app: &App, job: &mut Job) -> bool {
    let Ok(rows) = CurrentEvent::lock(&app.current).outgoing_scans(&job.event_id, SCAN_BATCH)
    else {
        return false;
    };
    if rows.is_empty() {
        return false;
    }
    job.fields.retain(|(name, _)| name != "vals");
    job.fields.push(("vals".to_string(), scan_vals(&rows)));
    true
}

fn fill_attachments(app: &App, job: &mut Job) -> bool {
    let Ok(rows) =
        CurrentEvent::lock(&app.current).outgoing_attachments(&job.event_id, ATTACH_BATCH)
    else {
        return false;
    };
    if rows.is_empty() {
        return false;
    }
    job.fields
        .retain(|(name, _)| name != "vals" && name != "no_need_data" && name != "expoId");
    job.fields
        .push(("vals".to_string(), attachment_vals(&rows)));
    job.fields
        .push(("no_need_data".to_string(), "true".to_string()));
    job.fields
        .push(("expoId".to_string(), job.event_id.clone()));
    true
}

fn registration_vals(rows: &[WaitingRegistration]) -> String {
    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "id": row.uid,
                "data": row.data,
                "ts": row.ts,
                "barcode": row.barcode,
                "org_id": row.org_id,
                "category": row.category,
                "ticket_status": row.ticket_status,
            })
        })
        .collect();
    serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string())
}

fn scan_vals(rows: &[OutgoingScan]) -> String {
    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "scan_id": row.scanid,
                "zone_id": row.zone_id,
                "entrence_type": row.entrence_type,
                "barcode": row.barcode,
                "time": row.time,
            })
        })
        .collect();
    serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string())
}

fn attachment_vals(rows: &[OutgoingAttachment]) -> String {
    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "uid": row.uid,
                "manager": row.man_id,
                "comment": row.comment,
                "zone": row.zone,
                "time": row.time,
            })
        })
        .collect();
    serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string())
}

fn accept_uploaded(app: &App, job: &Job, bytes: &[u8]) {
    let sent = text_ids(job, "id");
    let listed = synch_res_ids(bytes);
    let accepted: Vec<&str> = sent
        .iter()
        .filter(|id| listed.iter().any(|item| item == *id))
        .map(String::as_str)
        .collect();
    if accepted.is_empty() {
        return;
    }
    if let Err(err) =
        CurrentEvent::lock(&app.current).accept_registrations(&job.event_id, &accepted)
    {
        eprintln!("registration-server uploadreg: {err}");
    }
}

fn accept_scans(app: &App, job: &Job, bytes: &[u8]) {
    if !status_ok(bytes) {
        return;
    }
    let ids = number_ids(job, "scan_id");
    if ids.is_empty() {
        return;
    }
    if let Err(err) = CurrentEvent::lock(&app.current).mark_scans_synched(&job.event_id, &ids) {
        eprintln!("registration-server uploadscans: {err}");
    }
}

fn accept_attachments(app: &App, job: &Job, bytes: &[u8]) {
    if serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.get("users").cloned())
        .and_then(|users| users.as_array().cloned())
        .is_none()
    {
        return;
    }
    let rows = attachment_keys(job);
    if rows.is_empty() {
        return;
    }
    if let Err(err) = CurrentEvent::lock(&app.current).mark_attachments_sent(&job.event_id, &rows) {
        eprintln!("registration-server attachments: {err}");
    }
}

fn uploadreg_follow(app: &App, job: &Job, bytes: &[u8]) -> Vec<Job> {
    let sent = text_ids(job, "id");
    let listed = synch_res_ids(bytes);
    let mut jobs = Vec::new();
    for id in sent
        .iter()
        .filter(|id| listed.iter().any(|item| item == *id))
    {
        if let Some(path) = photo_path(app, &job.event_id, id) {
            jobs.push(photo_job(&job.event_id, id, path));
        }
    }
    if has_outgoing_attachments(app, &job.event_id) {
        jobs.push(upload_attachments_job(&job.event_id));
    }
    if has_outgoing_scans(app, &job.event_id) {
        jobs.push(uploadscans_job(&job.event_id));
    }
    if more_registrations(app, &job.event_id, &sent) {
        jobs.push(uploadreg_job(&job.event_id));
    }
    jobs
}

fn photo_path(app: &App, event_id: &str, id: &str) -> Option<PathBuf> {
    let path = CurrentEvent::lock(&app.current)
        .base_dir()
        .join(event_id)
        .join("photos")
        .join("uids")
        .join(format!("{id}.png"));
    path.is_file().then_some(path)
}

fn more_registrations(app: &App, event_id: &str, sent: &[String]) -> bool {
    let Ok(rows) = CurrentEvent::lock(&app.current).waiting_registrations(event_id, REG_BATCH)
    else {
        return false;
    };
    rows.iter().any(|row| !sent.iter().any(|id| id == &row.uid))
}

fn more_scans(app: &App, job: &Job) -> bool {
    let sent = number_ids(job, "scan_id");
    let Ok(rows) = CurrentEvent::lock(&app.current).outgoing_scans(&job.event_id, SCAN_BATCH)
    else {
        return false;
    };
    rows.iter().any(|row| !sent.contains(&row.scanid))
}

fn more_attachments(app: &App, job: &Job) -> bool {
    let sent = attachment_keys(job);
    let Ok(rows) =
        CurrentEvent::lock(&app.current).outgoing_attachments(&job.event_id, ATTACH_BATCH)
    else {
        return false;
    };
    rows.iter()
        .any(|row| !sent.contains(&(row.man_id, row.uid)))
}

fn has_outgoing_scans(app: &App, event_id: &str) -> bool {
    CurrentEvent::lock(&app.current)
        .outgoing_scans(event_id, 1)
        .ok()
        .is_some_and(|rows| !rows.is_empty())
}

fn has_outgoing_attachments(app: &App, event_id: &str) -> bool {
    CurrentEvent::lock(&app.current)
        .outgoing_attachments(event_id, 1)
        .ok()
        .is_some_and(|rows| !rows.is_empty())
}

fn text_ids(job: &Job, key: &str) -> Vec<String> {
    vals_array(job)
        .into_iter()
        .filter_map(|item| json_text(item.get(key)?))
        .collect()
}

fn number_ids(job: &Job, key: &str) -> Vec<i64> {
    text_ids(job, key)
        .iter()
        .filter_map(|text| text.parse().ok())
        .collect()
}

fn attachment_keys(job: &Job) -> Vec<(i64, i64)> {
    vals_array(job)
        .into_iter()
        .filter_map(|item| {
            let man_id = json_text(item.get("manager")?)?.parse().ok()?;
            let uid = json_text(item.get("uid")?)?.parse().ok()?;
            Some((man_id, uid))
        })
        .collect()
}

fn vals_array(job: &Job) -> Vec<serde_json::Value> {
    let raw = field(job, "vals");
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

fn synch_res_ids(bytes: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let Some(list) = value.get("synch_res").and_then(|item| item.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            json_text(item)
                .or_else(|| json_text(item.get("id").unwrap_or(&serde_json::Value::Null)))
        })
        .collect()
}

fn status_ok(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(|status| status.as_str())
                .map(str::to_string)
        })
        .is_some_and(|status| status == "ok")
}

fn zone_of(job: &Job) -> &str {
    field(job, "extraId").trim_start_matches("zone_")
}

fn store_scans(app: &App, job: &Job, bytes: &[u8]) {
    let Some(page) = scan_page(bytes) else {
        return;
    };
    for scan in &page.scans {
        match CurrentEvent::lock(&app.current).store_scan(&job.event_id, scan) {
            Ok(()) => {}
            Err(CurrentError::NotOpen { .. }) => {
                note(job, "NotOpen");
                return;
            }
            Err(err) => {
                eprintln!("registration-server scans: {err}");
                return;
            }
        }
    }
    let Some(from_time) = page.from_time else {
        return;
    };
    if let Err(err) = CurrentEvent::lock(&app.current).set_cursor(
        &job.event_id,
        SCAN_CURSOR,
        &from_time.to_string(),
    ) {
        eprintln!("registration-server scans: {err}");
    }
}

fn scan_page_continues(app: &App, event_id: &str, bytes: &[u8]) -> bool {
    let Some(page) = scan_page(bytes) else {
        return false;
    };
    if page.scans.len() != VISITOR_PAGE {
        return false;
    }
    let Some(from_time) = page.from_time else {
        return false;
    };
    CurrentEvent::lock(&app.current)
        .cursor(event_id, SCAN_CURSOR)
        .ok()
        .is_some_and(|stored| stored == from_time.to_string())
}

struct ScanPage {
    scans: Vec<ScanWrite>,
    from_time: Option<i64>,
}

fn scan_page(bytes: &[u8]) -> Option<ScanPage> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    let arr = value.get("arr")?.as_array()?;
    let mut scans = Vec::new();
    let mut from_time = 0i64;
    for item in arr {
        if let Some(ts) = json_i64(item.get("ts")) {
            if ts > from_time {
                from_time = ts;
            }
        }
        if let Some(scan) = scan_from_item(item) {
            scans.push(scan);
        }
    }
    Some(ScanPage {
        scans,
        from_time: if from_time > 0 { Some(from_time) } else { None },
    })
}

fn scan_from_item(item: &serde_json::Value) -> Option<ScanWrite> {
    let obj = item.get("obj").unwrap_or(item);
    let scanid = json_i64(obj.get("scanid").or_else(|| obj.get("id")))?;
    let zone_id = json_i64(obj.get("zone_id")).or_else(|| {
        obj.get("scanzone")
            .and_then(|zone| json_i64(zone.get("_id")))
    });
    Some(ScanWrite {
        scanid,
        zone_id: zone_id.unwrap_or(0),
        entrence_type: json_i64(obj.get("entrence_type")).unwrap_or(0),
        barcode: obj.get("barcode").and_then(json_text).unwrap_or_default(),
        time: json_i64(obj.get("time"))
            .or_else(|| json_i64(item.get("ts")))
            .unwrap_or(0),
    })
}

struct VisitorPage {
    visitors: Vec<VisitorWrite>,
    from_time: Option<i64>,
}

fn visitor_page(bytes: &[u8]) -> Option<VisitorPage> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    let arr = value.get("arr")?.as_array()?;
    let mut visitors = Vec::new();
    let mut from_time = 0i64;
    for item in arr {
        if let Some(ts) = json_i64(item.get("ts")) {
            if ts > from_time {
                from_time = ts;
            }
        }
        if let Some(visitor) = visitor_from_item(item) {
            visitors.push(visitor);
        }
    }
    Some(VisitorPage {
        visitors,
        from_time: if from_time > 0 { Some(from_time) } else { None },
    })
}

fn visitor_from_item(item: &serde_json::Value) -> Option<VisitorWrite> {
    let obj = item.get("obj").unwrap_or(item);
    let uid = json_text(obj.get("uid").or_else(|| obj.get("id"))?)?;
    if uid.is_empty() {
        return None;
    }
    let added_ts = json_i64(item.get("obj_ts"))
        .or_else(|| json_i64(item.get("ts")))
        .or_else(|| json_i64(obj.get("added_ts")))
        .unwrap_or(0);
    Some(VisitorWrite {
        uid,
        data: serde_json::to_string(obj).unwrap_or_else(|_| "{}".to_string()),
        name: obj.get("name").and_then(json_text).unwrap_or_default(),
        surname: obj.get("surname").and_then(json_text).unwrap_or_default(),
        c_name: obj.get("c_name").and_then(json_text).unwrap_or_default(),
        category: json_i64(obj.get("category")).unwrap_or(0),
        ticket_status: json_i64(obj.get("ticket_status")).unwrap_or(0),
        gotsome: json_i64(obj.get("gotsome")).unwrap_or(0),
        give_packet: json_i64(obj.get("give_packet")).unwrap_or(0),
        added_ts,
        org_id: json_i64(obj.get("org_id")),
        email: obj.get("email").and_then(json_text).unwrap_or_default(),
    })
}

fn json_i64(value: Option<&serde_json::Value>) -> Option<i64> {
    json_text(value?).and_then(|text| text.parse().ok())
}

fn zones_job(event_id: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/control_zones".to_string(),
        event_id: event_id.to_string(),
        fields: Vec::new(),
        cursors: vec![Cursor {
            name: "control_zones".to_string(),
            send_as: "last_synch",
            take_from: "last_synch",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn info_job(event_id: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Get,
        path: "boxapi/barcodesinfo".to_string(),
        event_id: event_id.to_string(),
        fields: Vec::new(),
        cursors: Vec::new(),
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn getbadge_job(event_id: &str, cat_id: i64, is_cert: bool) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/getbadge".to_string(),
        event_id: event_id.to_string(),
        fields: vec![
            ("cat_id".to_string(), cat_id.to_string()),
            (
                "is_cert".to_string(),
                if is_cert { "true" } else { "false" }.to_string(),
            ),
        ],
        cursors: vec![Cursor {
            name: "getbadge".to_string(),
            send_as: "last_synch",
            take_from: "last_synch",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn badge_head(event_id: &str, bytes: &[u8]) -> Vec<Job> {
    let ids = cat_ids(bytes);
    let jobs = ids
        .iter()
        .map(|id| getbadge_job(event_id, *id, false))
        .chain(ids.iter().map(|id| getbadge_job(event_id, *id, true)));
    chain(jobs).into_iter().collect()
}

fn cat_ids(bytes: &[u8]) -> Vec<i64> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let Some(list) = value.get("cats").and_then(|cats| cats.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            json_text(item.get("cat_id").or_else(|| item.get("id"))?)
                .and_then(|text| text.parse().ok())
        })
        .filter(|id| *id > -1)
        .collect()
}

struct Pool {
    cat_id: i64,
    bc_type: i64,
    remote_clean: i64,
    date: String,
    cert_id: Option<i64>,
}

fn barcode_head(app: &App, event_id: &str, bytes: &[u8]) -> Vec<Job> {
    let pools = pools_from_info(bytes);
    let mut jobs = Vec::new();
    for pool in pools {
        reconcile_pool(app, event_id, &pool);
        let Some(take) = pool_take(app, event_id, &pool) else {
            continue;
        };
        jobs.push(barcode_job(event_id, &pool, take));
    }
    chain(jobs.into_iter()).into_iter().collect()
}

fn barcode_job(event_id: &str, pool: &Pool, take: i64) -> Job {
    let mut fields = vec![
        ("cat_id".to_string(), pool.cat_id.to_string()),
        ("take".to_string(), take.to_string()),
        ("bc_type".to_string(), pool.bc_type.to_string()),
    ];
    if let Some(cert_id) = pool.cert_id {
        fields.push(("cert_id".to_string(), cert_id.to_string()));
    } else {
        fields.push(("multi_day".to_string(), "true".to_string()));
    }
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/barcodes".to_string(),
        event_id: event_id.to_string(),
        fields,
        cursors: vec![Cursor {
            name: pool_cursor_name(pool.cat_id, pool.bc_type, &pool.date),
            send_as: "",
            take_from: "clean_id",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn pool_cursor_name(cat_id: i64, bc_type: i64, date: &str) -> String {
    if date.is_empty() {
        format!("barcodes:{cat_id}:{bc_type}")
    } else {
        format!("barcodes:{cat_id}:{bc_type}:{date}")
    }
}

fn reconcile_pool(app: &App, event_id: &str, pool: &Pool) {
    let current = CurrentEvent::lock(&app.current).barcode_clean_id(
        event_id,
        pool.cat_id,
        pool.bc_type,
        &pool.date,
    );
    let Ok(Some(stored)) = current else {
        return;
    };
    if stored == pool.remote_clean {
        return;
    }
    if let Err(err) = CurrentEvent::lock(&app.current).clean_barcodes(
        event_id,
        pool.cat_id,
        pool.bc_type,
        &pool.date,
    ) {
        eprintln!("registration-server barcodes: {err}");
    }
}

fn pool_take(app: &App, event_id: &str, pool: &Pool) -> Option<i64> {
    let count = CurrentEvent::lock(&app.current)
        .barcode_count(event_id, pool.cat_id, pool.bc_type, &pool.date)
        .unwrap_or(0);
    let take = if count < BC_MIN {
        BC_MIN + BC_ADD - count
    } else {
        0
    };
    if take < BC_TAKE_MIN {
        None
    } else {
        Some(take)
    }
}

fn pools_from_info(bytes: &[u8]) -> Vec<Pool> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let Some(info) = value.get("info").and_then(|info| info.as_array()) else {
        return Vec::new();
    };
    let mut pools = Vec::new();
    for cat in info {
        let Some(cat_id) = json_text(cat.get("cat_id").unwrap_or(&serde_json::Value::Null))
            .and_then(|text| text.parse().ok())
        else {
            continue;
        };
        if let Some(list) = cat.get("clean_ids").and_then(|ids| ids.as_array()) {
            for item in list {
                if let Some(pool) = pool_from_clean(cat_id, item, "", None) {
                    pools.push(pool);
                }
            }
        }
        if let Some(dates) = cat.get("dates").and_then(|dates| dates.as_array()) {
            for day in dates {
                let date = day.get("date").and_then(json_text).unwrap_or_default();
                let cert_id = day
                    .get("cert_id")
                    .and_then(json_text)
                    .and_then(|text| text.parse().ok());
                if let Some(list) = day.get("clean_ids").and_then(|ids| ids.as_array()) {
                    for item in list {
                        if let Some(pool) = pool_from_clean(cat_id, item, &date, cert_id) {
                            pools.push(pool);
                        }
                    }
                }
            }
        }
    }
    pools
}

fn pool_from_clean(
    cat_id: i64,
    item: &serde_json::Value,
    date: &str,
    cert_id: Option<i64>,
) -> Option<Pool> {
    let bc_type = json_text(item.get("bcType")?)?.parse().ok()?;
    let remote_clean = json_text(item.get("clean_id")?)?.parse().ok()?;
    Some(Pool {
        cat_id,
        bc_type,
        remote_clean,
        date: date.to_string(),
        cert_id,
    })
}

fn first_dbenum(event_id: &str, bytes: &[u8]) -> Vec<Job> {
    let mut rest = Vec::new();
    for dep in enum_dependencies(bytes).into_iter().rev() {
        let mut job = dbenum_job(event_id, &dep);
        job.after = rest;
        rest = vec![job];
    }
    rest
}

fn dbenum_job(event_id: &str, dep: &EnumDep) -> Job {
    let mut fields = vec![
        ("enum".to_string(), dep.name.clone()),
        ("langs".to_string(), ENUM_LANGS.to_string()),
        ("gzipped".to_string(), "false".to_string()),
    ];
    fields.extend(dep.ids.iter().cloned());
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/dbenum".to_string(),
        event_id: event_id.to_string(),
        fields,
        cursors: Vec::new(),
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn empty_getconf(event_id: &str) -> Job {
    getconf_job(event_id, "", "getconf")
}

fn getconf_job(event_id: &str, conf_id: &str, cursor_name: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/getconf".to_string(),
        event_id: event_id.to_string(),
        fields: vec![("conf_id".to_string(), conf_id.to_string())],
        cursors: vec![
            Cursor {
                name: cursor_name.to_string(),
                send_as: "last_synch",
                take_from: "last_synch",
            },
            Cursor {
                name: "forms".to_string(),
                send_as: "last_cnfids_synch",
                take_from: "forms.change",
            },
        ],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn getbg_job(event_id: &str, conf_id: &str, resolution: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/getbg".to_string(),
        event_id: event_id.to_string(),
        fields: vec![
            ("resolution".to_string(), resolution.to_string()),
            ("conf_id".to_string(), conf_id.to_string()),
        ],
        cursors: vec![Cursor {
            name: format!("getbg:{conf_id}:{resolution}"),
            send_as: "last_synch",
            take_from: "last_synch",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn ticket_job(event_id: &str, conf_id: &str) -> Job {
    Job {
        lane: Lane::Download,
        method: CallMethod::Post,
        path: "boxapi/ticketbarcodes".to_string(),
        event_id: event_id.to_string(),
        fields: vec![("conf_id".to_string(), conf_id.to_string())],
        cursors: vec![Cursor {
            name: format!("ticketbarcodes:{conf_id}"),
            send_as: "clean_id",
            take_from: "clean_id",
        }],
        after: Vec::new(),
        hold_parse: None,
        log: None,
        files: Vec::new(),
    }
}

fn form_heads(event_id: &str, bytes: &[u8]) -> Vec<Job> {
    let ids = form_ids(bytes);
    let mut heads = Vec::new();
    if let Some(job) = chain(
        ids.iter()
            .map(|id| getconf_job(event_id, id, &format!("getconf:{id}"))),
    ) {
        heads.push(job);
    }
    if let Some(job) = chain(ids.iter().flat_map(|id| {
        [
            getbg_job(event_id, id, "pad"),
            getbg_job(event_id, id, "phone"),
        ]
    })) {
        heads.push(job);
    }
    if let Some(job) = chain(ids.iter().map(|id| ticket_job(event_id, id))) {
        heads.push(job);
    }
    heads
}

fn chain(jobs: impl DoubleEndedIterator<Item = Job>) -> Option<Job> {
    let mut rest = None;
    for mut job in jobs.rev() {
        if let Some(next) = rest {
            job.after = vec![next];
        }
        rest = Some(job);
    }
    rest
}

fn form_ids(bytes: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let Some(list) = value
        .get("forms")
        .and_then(|forms| forms.get("list"))
        .and_then(|list| list.as_array())
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| json_text(item.get("id")?))
        .filter(|id| !id.is_empty())
        .collect()
}

fn field<'a>(job: &'a Job, name: &str) -> &'a str {
    job.fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or("")
}

fn store_barcodes(app: &App, job: &Job, bytes: &[u8]) {
    let Some(pool) = line_pool(bytes) else {
        return;
    };
    let bc_type = field(job, "bc_type").parse::<i64>().unwrap_or(0);
    let date = job
        .cursors
        .first()
        .map(|cursor| pool_date(&cursor.name))
        .unwrap_or_default();
    let name = pool_cursor_name(pool.cat_id, bc_type, &date);
    if let Err(err) = CurrentEvent::lock(&app.current).store_barcodes(
        &job.event_id,
        pool.cat_id,
        pool.clean_id,
        bc_type,
        &date,
        &pool.codes,
    ) {
        eprintln!("registration-server barcodes: {err}");
        return;
    }
    if let Err(err) = CurrentEvent::lock(&app.current).set_cursor(
        &job.event_id,
        &name,
        &pool.clean_id.to_string(),
    ) {
        eprintln!("registration-server barcodes: {err}");
    }
}

fn pool_date(name: &str) -> String {
    let mut parts = name.splitn(4, ':');
    let _prefix = parts.next();
    let _cat = parts.next();
    let _bc_type = parts.next();
    parts.next().unwrap_or("").to_string()
}

struct LinePool {
    cat_id: i64,
    clean_id: i64,
    codes: Vec<String>,
}

fn line_pool(bytes: &[u8]) -> Option<LinePool> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let size: usize = lines.next()?.parse().ok()?;
    let cat_id: i64 = lines.next()?.parse().ok()?;
    let clean_id: i64 = lines.next()?.parse().ok()?;
    let _total = lines.next()?;
    let _last = lines.next()?;
    let codes = lines.take(size).map(str::to_string).collect();
    Some(LinePool {
        cat_id,
        clean_id,
        codes,
    })
}

fn store_ticket_barcodes(app: &App, job: &Job, bytes: &[u8]) {
    let Some((clean_id, codes)) = ticket_payload(bytes) else {
        return;
    };
    let form_id = field(job, "conf_id").parse::<i64>().unwrap_or(-1);
    if let Err(err) = CurrentEvent::lock(&app.current).store_ticket_barcodes(
        &job.event_id,
        form_id,
        clean_id,
        &codes,
    ) {
        eprintln!("registration-server tickets: {err}");
        return;
    }
    if let Err(err) = CurrentEvent::lock(&app.current).set_cursor(
        &job.event_id,
        &format!("ticketbarcodes:{form_id}"),
        &clean_id.to_string(),
    ) {
        eprintln!("registration-server tickets: {err}");
    }
}

fn ticket_payload(bytes: &[u8]) -> Option<(i64, Vec<String>)> {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
        if value.get("barcodes").is_some() || value.get("clean_id").is_some() {
            let clean_id = value
                .get("clean_id")
                .and_then(json_text)
                .and_then(|text| text.parse().ok())
                .unwrap_or(0);
            let codes = value
                .get("barcodes")
                .and_then(|list| list.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            return Some((clean_id, codes));
        }
    }
    let pool = line_pool(bytes)?;
    Some((pool.clean_id, pool.codes))
}

fn enum_dependencies(bytes: &[u8]) -> Vec<EnumDep> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let Some(list) = value.get("requested_enums").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    for item in list {
        let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let mut ids = Vec::new();
        if let Some(map) = item.get("ids").and_then(|v| v.as_object()) {
            for (key, value) in map {
                let text = match value {
                    serde_json::Value::String(text) => text.clone(),
                    serde_json::Value::Number(number) => number.to_string(),
                    _ => continue,
                };
                ids.push((key.clone(), text));
            }
        }
        deps.push(EnumDep {
            name: name.to_string(),
            ids,
        });
    }
    deps
}

fn endpoint(path: &str) -> &str {
    path.trim_start_matches('/')
}

fn soft_error(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    matches!(
        value.get("error_msg").and_then(serde_json::Value::as_str),
        Some(msg) if !msg.is_empty()
    )
}

pub async fn sync_once(
    client: &reqwest::Client,
    endpoints: &[String],
    store: &Store,
    gate: &Gate,
    in_flight: usize,
) {
    futures::stream::iter(endpoints.iter().cloned())
        .map(|url| {
            let client = client.clone();
            async move {
                gate.wait_until_idle().await;
                let body = read_body(&client, &url).await;
                (url, body)
            }
        })
        .buffer_unordered(in_flight)
        .for_each(|(url, body)| async move {
            gate.wait_until_idle().await;
            match body {
                Ok(bytes) => match parse_batch(&bytes) {
                    Ok(rows) => {
                        log::debug!("sync {url} rows={}", rows.len());
                        store.write_batch(rows);
                    }
                    Err(err) => eprintln!("registration-server sync {url}: {err}"),
                },
                Err(err) => eprintln!("registration-server sync {url}: {err}"),
            }
        })
        .await;
}

async fn read_body(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, DownloadError> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(DownloadError::Status(response.status()));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err(DownloadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_batch(bytes: &[u8]) -> Result<Vec<Registration>, DownloadError> {
    serde_json::from_slice(bytes).map_err(DownloadError::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn parse_sync_in_flight_reads_a_positive_integer() {
        assert_eq!(parse_sync_in_flight("200"), 200);
        assert!(SYNC_IN_FLIGHT > 0);
    }

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn serve_json(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });
        format!("http://{addr}/rows")
    }

    #[tokio::test]
    async fn two_endpoints_land_as_one_batch_each() {
        let first = serve_json(r#"[{"id":"1","name":"Ann"}]"#).await;
        let second = serve_json(r#"[{"id":"2","name":"Bo"}]"#).await;
        let store = Store::new();
        let gate = Gate::new();
        sync_once(
            &client(SYNC_IN_FLIGHT),
            &[first, second],
            &store,
            &gate,
            SYNC_IN_FLIGHT,
        )
        .await;
        assert_eq!(store.lookup("1").unwrap().name, "Ann");
        assert_eq!(store.lookup("2").unwrap().name, "Bo");
    }

    #[tokio::test]
    async fn a_local_call_holds_the_download() {
        let url = serve_json(r#"[{"id":"1","name":"Ann"}]"#).await;
        let store = Arc::new(Store::new());
        let gate = Arc::new(Gate::new());
        let guard = gate.enter_request();
        let pending = {
            let store = Arc::clone(&store);
            let gate = Arc::clone(&gate);
            tokio::spawn(async move {
                sync_once(
                    &client(SYNC_IN_FLIGHT),
                    &[url],
                    &store,
                    &gate,
                    SYNC_IN_FLIGHT,
                )
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(store.lookup("1").is_none());
        drop(guard);
        pending.await.unwrap();
        assert_eq!(store.lookup("1").unwrap().name, "Ann");
    }

    mod prove {
        use super::*;
        use crate::registration_server::credentials::CredentialFile;
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::{VisitorWrite, DB_FILE};
        use crate::registration_server::print_queue::PrintWorker;
        use crate::registration_server::remote::Remote;
        use crate::registration_server::remote_server::RemoteServer;
        use crate::registration_server::{App, OperatorState};
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::routing::get;
        use axum::Router;
        use rusqlite::Connection;
        use std::collections::HashMap;
        use std::path::{Path, PathBuf};
        use std::sync::Mutex as StdMutex;

        const TIMEOUT: Duration = Duration::from_secs(2);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-probe-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4()
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

        struct Held {
            app: App,
            _worker: PrintWorker,
        }

        fn visitor(uid: &str) -> VisitorWrite {
            VisitorWrite {
                uid: uid.to_string(),
                data: format!(r#"{{"uid":"{uid}"}}"#),
                name: "запись".to_string(),
                surname: "проба".to_string(),
                c_name: String::new(),
                category: 7,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 10,
                org_id: None,
                email: format!("{uid}@example.com"),
            }
        }

        fn file_has(path: &Path, uid: &str) -> bool {
            let conn = Connection::open(path).unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM mem_u WHERE uid = ?1", [uid], |row| {
                    row.get(0)
                })
                .unwrap();
            count > 0
        }

        fn held(dir: &Path, event_id: Option<&str>) -> Held {
            let file = CredentialFile::new(dir.join("credentials.yml"));
            file.update(|credentials| {
                credentials.set_device_id("device-1")?;
                credentials.set_base_url("https://kuprin.su/")?;
                if let Some(event_id) = event_id {
                    credentials.bind(event_id, "stored-token", Some("Show".to_string()))?;
                }
                Ok(())
            })
            .unwrap();
            let (mut app, worker) = App::new();
            let current = Arc::new(std::sync::Mutex::new(CurrentEvent::new(dir)));
            let operator = OperatorState::new(file, Remote::new("http://127.0.0.1:1").unwrap())
                .with_current(Arc::clone(&current));
            if event_id.is_some() {
                CurrentEvent::lock(&current)
                    .open_bound(operator.credential_file())
                    .unwrap();
            }
            app.current = current;
            app.operator = operator;
            Held {
                app,
                _worker: worker,
            }
        }

        #[derive(Clone)]
        struct Hit {
            method: String,
            path: String,
            query: String,
        }

        #[derive(Clone)]
        struct Reply {
            status: StatusCode,
            body: &'static str,
        }

        struct Stand {
            base: String,
            hits: Arc<StdMutex<Vec<Hit>>>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Stand {
            async fn start(reply: Reply) -> Self {
                let hits = Arc::new(StdMutex::new(Vec::new()));
                let app = Router::new()
                    .route("/boxapi/barcodesinfo", get(barcodes))
                    .route("/rows", get(rows))
                    .with_state((Arc::clone(&hits), reply));
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let handle = tokio::spawn(async move {
                    axum::serve(listener, app).await.unwrap();
                });
                Self {
                    base: format!("http://{addr}/"),
                    hits,
                    handle,
                }
            }

            fn hits(&self) -> Vec<Hit> {
                self.hits.lock().unwrap().clone()
            }
        }

        impl Drop for Stand {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn barcodes(
            State((hits, reply)): State<(Arc<StdMutex<Vec<Hit>>>, Reply)>,
            req: axum::extract::Request,
        ) -> axum::response::Response {
            record(&hits, &req);
            (reply.status, reply.body).into_response()
        }

        async fn rows(
            State((hits, _reply)): State<(Arc<StdMutex<Vec<Hit>>>, Reply)>,
            req: axum::extract::Request,
        ) -> axum::response::Response {
            record(&hits, &req);
            (StatusCode::OK, r#"[{"id":"1","name":"Ann"}]"#).into_response()
        }

        fn record(hits: &StdMutex<Vec<Hit>>, req: &axum::extract::Request) {
            hits.lock().unwrap().push(Hit {
                method: req.method().as_str().to_string(),
                path: req.uri().path().to_string(),
                query: req.uri().query().unwrap_or("").to_string(),
            });
        }

        fn query_map(raw: &str) -> HashMap<String, String> {
            let mut pairs = HashMap::new();
            if raw.is_empty() {
                return pairs;
            }
            for pair in raw.split('&') {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                pairs.insert(name.to_string(), value.to_string());
            }
            pairs
        }

        fn remote(base: &str) -> RemoteServer {
            RemoteServer::new(base).unwrap()
        }

        #[tokio::test]
        async fn probe_accepts_barcodesinfo() {
            let stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"info":[{"cat_id":1}]}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("EVT1"));
            sync_wake(
                &held.app,
                &remote(&stand.base),
                &[],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;

            let hits = stand.hits();
            assert!(hits.len() >= 1);
            assert_eq!(hits[0].method, "GET");
            assert_eq!(hits[0].path, "/boxapi/barcodesinfo");
            let fields = query_map(&hits[0].query);
            assert_eq!(fields.get("dev_id").map(String::as_str), Some("device-1"));
            assert_eq!(
                fields.get("token").map(String::as_str),
                Some("stored-token")
            );
            assert_eq!(fields.get("eid").map(String::as_str), Some("EVT1"));
            assert_eq!(fields.len(), 3);
            let stored = held.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.project_token(), Some("stored-token"));
            assert_eq!(
                CurrentEvent::lock(&held.app.current).event_id().as_deref(),
                Some("EVT1")
            );
        }

        #[tokio::test]
        async fn unbound_probe_opens_no_socket() {
            let stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"info":[]}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, None);
            let rows = format!("{}rows", stand.base);
            sync_wake(
                &held.app,
                &remote(&stand.base),
                &[rows],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;
            assert!(stand.hits().is_empty());
            assert!(held.app.store.lookup("1").is_none());
        }

        #[tokio::test]
        async fn probe_waits_for_a_different_open_event() {
            let stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"info":[]}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("BBB"));
            CurrentEvent::lock(&held.app.current)
                .switch_to("AAA")
                .unwrap();
            sync_wake(
                &held.app,
                &remote(&stand.base),
                &[],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;
            assert!(stand.hits().is_empty());
            assert_eq!(
                CurrentEvent::lock(&held.app.current).event_id().as_deref(),
                Some("AAA")
            );
            let stored = held.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.event_id(), Some("BBB"));
            assert_eq!(stored.project_token(), Some("stored-token"));
        }

        #[tokio::test]
        async fn probe_runs_before_the_placeholder_downloads() {
            let stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"info":[{"cat_id":1}]}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("EVT1"));
            let rows = format!("{}rows", stand.base);
            sync_wake(
                &held.app,
                &remote(&stand.base),
                &[rows],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;
            let hits = stand.hits();
            assert!(hits.len() >= 1);
            assert_eq!(hits[0].path, "/boxapi/barcodesinfo");
            assert!(hits.iter().all(|hit| hit.path != "/rows"));
            assert!(held.app.store.lookup("1").is_none());
        }

        #[tokio::test]
        async fn a_local_call_holds_the_probe() {
            let stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"info":[{"cat_id":1}]}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("EVT1"));
            let guard = held.app.gate.enter_request();
            let pending = {
                let app = held.app.clone();
                let remote = remote(&stand.base);
                tokio::spawn(
                    async move { sync_wake(&app, &remote, &[], TIMEOUT, SYNC_IN_FLIGHT).await },
                )
            };
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(stand.hits().is_empty());
            drop(guard);
            pending.await.unwrap();
            assert!(stand.hits().len() >= 1);
            assert!(stand
                .hits()
                .iter()
                .all(|hit| hit.path == "/boxapi/barcodesinfo"));
        }

        #[tokio::test]
        async fn not_logged_in_clears_the_binding() {
            clear_on(r#"{"error_msg":"not_logged_in"}"#).await;
        }

        #[tokio::test]
        async fn no_pattern_clears_the_binding() {
            clear_on(r#"{"error_msg":"no_pattern"}"#).await;
        }

        async fn clear_on(body: &'static str) {
            let stand = Stand::start(Reply {
                status: StatusCode::OK,
                body,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("EVT1"));
            CurrentEvent::lock(&held.app.current)
                .write("EVT1", &visitor("1"))
                .unwrap();
            let rows = format!("{}rows", stand.base);
            sync_wake(
                &held.app,
                &remote(&stand.base),
                &[rows],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;
            assert!(CurrentEvent::lock(&held.app.current).event_id().is_none());
            let stored = held.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.device_id(), Some("device-1"));
            assert_eq!(stored.base_url(), Some("https://kuprin.su/"));
            assert_eq!(stored.event_id(), None);
            assert_eq!(stored.event_name(), None);
            assert_eq!(stored.project_token(), None);
            let text = std::fs::read_to_string(held.app.operator.credential_file().path()).unwrap();
            assert!(!text.contains("event_id"));
            assert!(!text.contains("event_name"));
            assert!(!text.contains("project_token"));
            let db = scratch.0.join("EVT1").join(DB_FILE);
            assert!(db.is_file());
            assert!(file_has(&db, "1"));
            assert!(stand.hits().iter().all(|hit| hit.path != "/rows"));
            assert!(held.app.store.lookup("1").is_none());
        }

        #[tokio::test]
        async fn the_next_wake_opens_no_socket() {
            let stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"error_msg":"not_logged_in"}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("EVT1"));
            sync_wake(
                &held.app,
                &remote(&stand.base),
                &[],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;
            let after_clear = stand.hits().len();
            assert_eq!(after_clear, 1);
            sync_wake(
                &held.app,
                &remote(&stand.base),
                &[],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;
            assert_eq!(stand.hits().len(), after_clear);
        }

        #[tokio::test]
        async fn other_error_keeps_the_token() {
            let stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"error_msg":"no_barcodes"}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("EVT1"));
            let rows = format!("{}rows", stand.base);
            sync_wake(
                &held.app,
                &remote(&stand.base),
                &[rows],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;
            let stored = held.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.project_token(), Some("stored-token"));
            assert_eq!(stored.event_id(), Some("EVT1"));
            assert_eq!(
                CurrentEvent::lock(&held.app.current).event_id().as_deref(),
                Some("EVT1")
            );
            assert!(stand.hits().iter().all(|hit| hit.path != "/rows"));
            assert!(held.app.store.lookup("1").is_none());
        }

        #[tokio::test]
        async fn a_dead_socket_keeps_the_token() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await;
            });
            let rows_stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"info":[]}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("EVT1"));
            let rows = format!("{}rows", rows_stand.base);
            sync_wake(
                &held.app,
                &remote(&format!("http://{addr}/")),
                &[rows],
                TIMEOUT,
                SYNC_IN_FLIGHT,
            )
            .await;
            let stored = held.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.project_token(), Some("stored-token"));
            assert_eq!(
                CurrentEvent::lock(&held.app.current).event_id().as_deref(),
                Some("EVT1")
            );
            assert!(rows_stand.hits().is_empty());
            assert!(held.app.store.lookup("1").is_none());
        }

        #[tokio::test]
        async fn a_switch_drops_the_probe_and_keeps_the_token() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let hits = Arc::new(StdMutex::new(0u32));
            let accepted = Arc::clone(&hits);
            tokio::spawn(async move {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await;
                *accepted.lock().unwrap() += 1;
                std::future::pending::<()>().await;
            });
            let rows_stand = Stand::start(Reply {
                status: StatusCode::OK,
                body: r#"{"info":[]}"#,
            })
            .await;
            let scratch = Scratch::new();
            let held = held(&scratch.0, Some("EVT1"));
            let remote = remote(&format!("http://{addr}/"));
            CurrentEvent::lock(&held.app.current).set_remote(remote.clone());
            let app = held.app.clone();
            let rows = format!("{}rows", rows_stand.base);
            let started = tokio::time::Instant::now();
            let pending = tokio::spawn(async move {
                sync_wake(
                    &app,
                    &remote,
                    &[rows],
                    Duration::from_secs(5),
                    SYNC_IN_FLIGHT,
                )
                .await
            });
            let wait = tokio::time::Instant::now();
            while *hits.lock().unwrap() == 0 {
                if wait.elapsed() > Duration::from_secs(2) {
                    panic!("probe was not sent");
                }
                tokio::task::yield_now().await;
            }
            assert_eq!(
                CurrentEvent::lock(&held.app.current)
                    .switch_to("BBB")
                    .unwrap(),
                "BBB"
            );
            pending.await.unwrap();
            assert!(started.elapsed() < Duration::from_secs(1));
            let stored = held.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.project_token(), Some("stored-token"));
            assert_eq!(stored.event_id(), Some("EVT1"));
            assert_eq!(stored.event_name(), Some("Show"));
            assert_eq!(
                CurrentEvent::lock(&held.app.current).event_id().as_deref(),
                Some("BBB")
            );
            assert!(rows_stand.hits().is_empty());
            assert!(held.app.store.lookup("1").is_none());
        }
    }

    mod support {
        use super::*;
        use crate::registration_server::credentials::CredentialFile;
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::print_queue::PrintWorker;
        use crate::registration_server::remote::Remote;
        use crate::registration_server::{App, OperatorState};
        use axum::extract::State;
        use axum::routing::get;
        use axum::Router;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::sync::Mutex as StdMutex;

        pub(super) struct Venue {
            pub(super) app: App,
            pub(super) _worker: PrintWorker,
        }

        pub(super) fn venue(event_id: &str) -> (Venue, PathBuf) {
            let dir = std::env::temp_dir().join(format!(
                "rust-reg-queue-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let file = CredentialFile::new(dir.join("credentials.yml"));
            file.update(|credentials| {
                credentials.set_device_id("device-1")?;
                credentials.set_base_url("https://kuprin.su/")?;
                credentials.bind(event_id, "stored-token", Some("Show".to_string()))?;
                Ok(())
            })
            .unwrap();
            let (mut app, worker) = App::new();
            let current = Arc::new(std::sync::Mutex::new(CurrentEvent::new(&dir)));
            let operator = OperatorState::new(file, Remote::new("http://127.0.0.1:1").unwrap())
                .with_current(Arc::clone(&current));
            CurrentEvent::lock(&current)
                .open_bound(operator.credential_file())
                .unwrap();
            app.current = current;
            app.operator = operator;
            (
                Venue {
                    app,
                    _worker: worker,
                },
                dir,
            )
        }

        pub(super) struct Script {
            pub(super) base: String,
            pub(super) hits: Arc<StdMutex<Vec<(String, String)>>>,
            release: watch::Sender<usize>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Script {
            pub(super) async fn start(bodies: HashMap<String, String>) -> Self {
                let hits = Arc::new(StdMutex::new(Vec::new()));
                let (release, gate) = watch::channel(0usize);
                let app = Router::new().fallback(get(answer)).with_state((
                    Arc::clone(&hits),
                    gate,
                    bodies,
                ));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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

            pub(super) fn release(&self, n: usize) {
                let _ = self.release.send(n);
            }

            pub(super) fn paths(&self) -> Vec<String> {
                self.hits
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(path, _)| path.clone())
                    .collect()
            }

            pub(super) fn queries(&self) -> Vec<String> {
                self.hits
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(_, query)| query.clone())
                    .collect()
            }
        }

        impl Drop for Script {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn answer(
            State((hits, mut gate, bodies)): State<(
                Arc<StdMutex<Vec<(String, String)>>>,
                watch::Receiver<usize>,
                HashMap<String, String>,
            )>,
            req: axum::extract::Request,
        ) -> String {
            let path = req.uri().path().to_string();
            let query = req.uri().query().unwrap_or("").to_string();
            let ticket = {
                let mut hits = hits.lock().unwrap();
                hits.push((path.clone(), query));
                hits.len()
            };
            let _ = gate.wait_for(|done| *done >= ticket).await;
            bodies
                .get(&path)
                .cloned()
                .unwrap_or_else(|| "{\"ok\":true}".into())
        }

        pub(super) fn job(lane: Lane, path: &str, event_id: &str) -> Job {
            Job {
                lane,
                method: CallMethod::Get,
                path: path.to_string(),
                event_id: event_id.to_string(),
                fields: Vec::new(),
                cursors: Vec::new(),
                after: Vec::new(),
                hold_parse: None,
                log: None,
                files: Vec::new(),
            }
        }

        pub(super) async fn until(script: &Script, n: usize) {
            let started = tokio::time::Instant::now();
            while script.paths().len() < n {
                if started.elapsed() > Duration::from_secs(2) {
                    panic!(
                        "timed out waiting for {n} requests, saw {}",
                        script.paths().len()
                    );
                }
                tokio::task::yield_now().await;
            }
        }
    }

    mod queue {
        use super::support::{venue, Script};
        use super::*;
        use crate::registration_server::current::CurrentEvent;
        use axum::extract::State;
        use axum::routing::get;
        use axum::Router;
        use std::sync::Mutex as StdMutex;

        struct Probe {
            base: String,
            hits: Arc<StdMutex<u32>>,
            release: watch::Sender<bool>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Probe {
            async fn start(body: &'static str) -> Self {
                let hits = Arc::new(StdMutex::new(0u32));
                let (release, gate) = watch::channel(false);
                let app = Router::new()
                    .route("/boxapi/barcodesinfo", get(hold))
                    .with_state((Arc::clone(&hits), gate, body));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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

        impl Drop for Probe {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn hold(
            State((hits, mut gate, body)): State<(
                Arc<StdMutex<u32>>,
                watch::Receiver<bool>,
                &'static str,
            )>,
        ) -> &'static str {
            *hits.lock().unwrap() += 1;
            let _ = gate.wait_for(|go| *go).await;
            body
        }

        #[tokio::test]
        async fn an_accepted_probe_arms_the_queue() {
            let probe = Probe::start(r#"{"info":[]}"#).await;
            probe.release.send(true).unwrap();
            let (venue, _dir) = venue("EVT1");
            let remote = Script::start(std::collections::HashMap::new()).await;
            let _ = remote;
            sync_wake(
                &venue.app,
                &crate::registration_server::remote_server::RemoteServer::new(&probe.base).unwrap(),
                &["http://127.0.0.1:9/rows".into()],
                Duration::from_secs(2),
                2,
            )
            .await;
            assert_eq!(*probe.hits.lock().unwrap(), 1);
            let stored = venue.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.project_token(), Some("stored-token"));
            assert_eq!(
                CurrentEvent::lock(&venue.app.current).event_id().as_deref(),
                Some("EVT1")
            );
        }

        #[tokio::test]
        async fn a_busy_queue_is_not_filled_again() {
            let probe = Probe::start(r#"{"info":[]}"#).await;
            let (venue, _dir) = venue("EVT1");
            let remote =
                crate::registration_server::remote_server::RemoteServer::new(&probe.base).unwrap();
            let app = venue.app.clone();
            let remote2 = remote.clone();
            let first = tokio::spawn(async move {
                sync_wake(&app, &remote2, &[], Duration::from_secs(2), 2).await
            });
            let started = tokio::time::Instant::now();
            while *probe.hits.lock().unwrap() == 0 {
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("probe was not sent");
                }
                tokio::task::yield_now().await;
            }
            let app = venue.app.clone();
            let remote2 = remote.clone();
            let second = tokio::spawn(async move {
                sync_wake(&app, &remote2, &[], Duration::from_secs(2), 2).await
            });
            second.await.unwrap();
            assert_eq!(*probe.hits.lock().unwrap(), 1);
            probe.release.send(true).unwrap();
            first.await.unwrap();
            assert_eq!(*probe.hits.lock().unwrap(), 1);
        }

        #[tokio::test]
        async fn a_rejected_probe_leaves_the_queue_empty() {
            let probe = Probe::start(r#"{"error_msg":"not_logged_in"}"#).await;
            probe.release.send(true).unwrap();
            let (venue, _dir) = venue("EVT1");
            sync_wake(
                &venue.app,
                &crate::registration_server::remote_server::RemoteServer::new(&probe.base).unwrap(),
                &[],
                Duration::from_secs(2),
                2,
            )
            .await;
            assert!(CurrentEvent::lock(&venue.app.current).event_id().is_none());
            assert_eq!(*probe.hits.lock().unwrap(), 1);
            let stored = venue.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.project_token(), None);
        }
    }

    mod slots {
        use super::support::{job, until, venue, Script};
        use super::*;
        use crate::registration_server::remote_server::RemoteServer;
        use std::collections::HashMap;

        fn bodies(paths: &[&str]) -> HashMap<String, String> {
            paths
                .iter()
                .map(|path| ((*path).to_string(), "{\"ok\":true}".to_string()))
                .collect()
        }

        #[tokio::test]
        async fn two_calls_are_in_flight() {
            let script = Script::start(bodies(&["/boxapi/a", "/boxapi/b"])).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                let remote = remote.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![
                            job(Lane::Download, "/boxapi/a", "EVT1"),
                            job(Lane::Download, "/boxapi/b", "EVT1"),
                        ],
                        2,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 2).await;
            assert_eq!(script.paths().len(), 2);
            script.release(2);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn the_same_endpoint_stays_sequential() {
            let script = Script::start(bodies(&["/boxapi/same"])).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![
                            job(Lane::Download, "/boxapi/same", "EVT1"),
                            job(Lane::Download, "/boxapi/same", "EVT1"),
                        ],
                        2,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 1).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(script.paths().len(), 1);
            script.release(1);
            until(&script, 2).await;
            script.release(2);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn the_queue_holds_one_cap_of_ready_calls() {
            let script = Script::start(bodies(&["/boxapi/a", "/boxapi/b", "/boxapi/c"])).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![
                            job(Lane::Download, "/boxapi/a", "EVT1"),
                            job(Lane::Download, "/boxapi/b", "EVT1"),
                            job(Lane::Download, "/boxapi/c", "EVT1"),
                        ],
                        2,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 2).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(script.paths().len(), 2);
            script.release(1);
            until(&script, 3).await;
            script.release(3);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn one_body_is_parsed_at_a_time() {
            let script = Script::start(bodies(&["/boxapi/a", "/boxapi/b"])).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            let log = Arc::new(std::sync::Mutex::new(Vec::new()));
            let (hold_tx, hold_rx) = watch::channel(false);
            let mut first = job(Lane::Download, "/boxapi/a", "EVT1");
            let mut second = job(Lane::Download, "/boxapi/b", "EVT1");
            first.hold_parse = Some(hold_rx.clone());
            second.hold_parse = Some(hold_rx);
            first.log = Some(Arc::clone(&log));
            second.log = Some(Arc::clone(&log));
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![first, second],
                        2,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 2).await;
            script.release(2);
            let started = tokio::time::Instant::now();
            loop {
                let events = log.lock().unwrap().clone();
                if events.iter().any(|event| event.starts_with("start ")) {
                    assert_eq!(
                        events
                            .iter()
                            .filter(|event| event.starts_with("start "))
                            .count(),
                        1
                    );
                    break;
                }
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("parse did not start");
                }
                tokio::task::yield_now().await;
            }
            hold_tx.send(true).unwrap();
            pending.await.unwrap();
            let events = log.lock().unwrap().clone();
            let starts: Vec<_> = events
                .iter()
                .filter(|event| event.starts_with("start "))
                .collect();
            let stores: Vec<_> = events
                .iter()
                .filter(|event| event.starts_with("store "))
                .collect();
            assert_eq!(starts.len(), 2);
            assert_eq!(stores.len(), 2);
            let first_store = events
                .iter()
                .position(|event| event.starts_with("store "))
                .unwrap();
            let second_start = events
                .iter()
                .enumerate()
                .filter(|(_, event)| event.starts_with("start "))
                .nth(1)
                .unwrap()
                .0;
            assert!(first_store < second_start);
        }

        #[tokio::test]
        async fn a_local_call_holds_the_parse() {
            let script = Script::start(bodies(&["/boxapi/a", "/boxapi/b"])).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            let log = Arc::new(std::sync::Mutex::new(Vec::new()));
            let mut first = job(Lane::Download, "/boxapi/a", "EVT1");
            let mut second = job(Lane::Download, "/boxapi/b", "EVT1");
            first.log = Some(Arc::clone(&log));
            second.log = Some(Arc::clone(&log));
            let guard = venue.app.gate.enter_request();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![first, second],
                        2,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 2).await;
            script.release(2);
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(log
                .lock()
                .unwrap()
                .iter()
                .all(|event| !event.starts_with("store ")));
            drop(guard);
            pending.await.unwrap();
            let events = log.lock().unwrap().clone();
            let first_store = events
                .iter()
                .position(|event| event.starts_with("store "))
                .unwrap();
            let second_start = events
                .iter()
                .enumerate()
                .filter(|(_, event)| event.starts_with("start "))
                .nth(1)
                .unwrap()
                .0;
            assert!(first_store < second_start);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event.starts_with("store "))
                    .count(),
                2
            );
        }
    }

    mod rank {
        use super::support::{job, until, venue, Script};
        use super::*;
        use crate::registration_server::remote_server::RemoteServer;
        use std::collections::HashMap;

        fn bodies(paths: &[&str]) -> HashMap<String, String> {
            paths
                .iter()
                .map(|path| ((*path).to_string(), "{\"ok\":true}".to_string()))
                .collect()
        }

        #[tokio::test]
        async fn a_download_takes_the_socket_ahead_of_an_upload() {
            let script = Script::start(bodies(&["/boxapi/down", "/boxapi/up"])).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![
                            job(Lane::Download, "/boxapi/down", "EVT1"),
                            job(Lane::Upload, "/boxapi/up", "EVT1"),
                        ],
                        1,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 1).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(script.paths(), vec!["/boxapi/down".to_string()]);
            script.release(1);
            until(&script, 2).await;
            assert_eq!(script.paths()[1], "/boxapi/up");
            script.release(2);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn an_empty_outbox_opens_no_socket() {
            let script = Script::start(bodies(&["/boxapi/down", "/boxapi/up"])).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            script.release(1);
            drain_queue(
                &venue.app,
                &remote,
                vec![job(Lane::Download, "/boxapi/down", "EVT1")],
                2,
                Duration::from_secs(2),
            )
            .await;
            assert_eq!(script.paths(), vec!["/boxapi/down".to_string()]);
        }

        #[tokio::test]
        async fn uploads_share_the_sockets() {
            let script = Script::start(bodies(&["/boxapi/up-a", "/boxapi/up-b"])).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![
                            job(Lane::Upload, "/boxapi/up-a", "EVT1"),
                            job(Lane::Upload, "/boxapi/up-b", "EVT1"),
                        ],
                        2,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 2).await;
            script.release(2);
            pending.await.unwrap();
        }
    }

    mod stop {
        use super::support::{job, until, venue, Script};
        use super::*;
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::{VisitorWrite, DB_FILE};
        use crate::registration_server::remote_server::RemoteServer;
        use std::collections::HashMap;

        fn bodies(pairs: &[(&str, &str)]) -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(path, body)| ((*path).to_string(), (*body).to_string()))
                .collect()
        }

        fn visitor(uid: &str) -> VisitorWrite {
            VisitorWrite {
                uid: uid.to_string(),
                data: format!(r#"{{"uid":"{uid}"}}"#),
                name: "запись".to_string(),
                surname: "проба".to_string(),
                c_name: String::new(),
                category: 7,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 10,
                org_id: None,
                email: format!("{uid}@example.com"),
            }
        }

        #[tokio::test]
        async fn not_logged_in_clears_the_queue() {
            let script = Script::start(bodies(&[
                ("/boxapi/one", r#"{"ok":true}"#),
                ("/boxapi/two", r#"{"error_msg":"not_logged_in"}"#),
                ("/boxapi/three", r#"{"ok":true}"#),
            ]))
            .await;
            script.release(3);
            let (venue, dir) = venue("EVT1");
            CurrentEvent::lock(&venue.app.current)
                .write("EVT1", &visitor("1"))
                .unwrap();
            let remote = RemoteServer::new(&script.base).unwrap();
            let outcomes = drain_queue(
                &venue.app,
                &remote,
                vec![
                    job(Lane::Download, "/boxapi/one", "EVT1"),
                    job(Lane::Download, "/boxapi/two", "EVT1"),
                    job(Lane::Download, "/boxapi/three", "EVT1"),
                ],
                1,
                Duration::from_secs(2),
            )
            .await;
            assert!(outcomes.iter().any(|(_, kind)| *kind == "rejected"));
            assert!(!script.paths().iter().any(|path| path == "/boxapi/three"));
            assert!(CurrentEvent::lock(&venue.app.current).event_id().is_none());
            let stored = venue.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.device_id(), Some("device-1"));
            assert_eq!(stored.base_url(), Some("https://kuprin.su/"));
            assert_eq!(stored.project_token(), None);
            assert!(dir.join("EVT1").join(DB_FILE).is_file());
        }

        #[tokio::test]
        async fn another_error_clears_the_queue_and_keeps_the_token() {
            let script = Script::start(bodies(&[
                ("/boxapi/one", r#"{"ok":true}"#),
                ("/boxapi/two", r#"{"error_msg":"no_barcodes"}"#),
                ("/boxapi/three", r#"{"ok":true}"#),
            ]))
            .await;
            script.release(3);
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                vec![
                    job(Lane::Download, "/boxapi/one", "EVT1"),
                    job(Lane::Download, "/boxapi/two", "EVT1"),
                    job(Lane::Download, "/boxapi/three", "EVT1"),
                ],
                1,
                Duration::from_secs(2),
            )
            .await;
            assert!(!script.paths().iter().any(|path| path == "/boxapi/three"));
            let stored = venue.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.project_token(), Some("stored-token"));
            assert_eq!(
                CurrentEvent::lock(&venue.app.current).event_id().as_deref(),
                Some("EVT1")
            );
            let again = remote
                .request_bytes(
                    Method::Get,
                    "boxapi/one",
                    &[],
                    crate::registration_server::remote_server::Auth {
                        device_id: "device-1",
                        event_id: "EVT1",
                        project_token: "stored-token",
                    },
                    Duration::from_secs(2),
                )
                .await;
            assert!(again.is_ok(), "{again:?}");
        }

        #[tokio::test]
        async fn a_switched_event_clears_the_previous_queue() {
            let script = Script::start(bodies(&[
                ("/boxapi/one", r#"{"ok":true}"#),
                ("/boxapi/two", r#"{"ok":true}"#),
            ]))
            .await;
            let (venue, _dir) = venue("AAA");
            let remote = RemoteServer::new(&script.base).unwrap();
            CurrentEvent::lock(&venue.app.current).set_remote(remote.clone());
            let pending = {
                let app = venue.app.clone();
                let remote = remote.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![
                            job(Lane::Download, "/boxapi/one", "AAA"),
                            job(Lane::Download, "/boxapi/two", "AAA"),
                        ],
                        1,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 1).await;
            assert_eq!(
                CurrentEvent::lock(&venue.app.current)
                    .switch_to("BBB")
                    .unwrap(),
                "BBB"
            );
            let outcomes = pending.await.unwrap();
            assert!(outcomes.iter().any(|(_, kind)| *kind == "dropped"));
            assert!(!script.paths().iter().any(|path| path == "/boxapi/two"));
            assert!(script
                .paths()
                .iter()
                .all(|path| path != "/boxapi/barcodesinfo"));
        }

        #[tokio::test]
        async fn the_next_wake_runs_the_new_event() {
            let mut map = bodies(&[("/boxapi/one", r#"{"ok":true}"#)]);
            map.insert(
                "/boxapi/barcodesinfo".to_string(),
                r#"{"info":[]}"#.to_string(),
            );
            let script = Script::start(map).await;
            let (venue, _dir) = venue("AAA");
            let remote = RemoteServer::new(&script.base).unwrap();
            CurrentEvent::lock(&venue.app.current).set_remote(remote.clone());
            let pending = {
                let app = venue.app.clone();
                let remote = remote.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![
                            job(Lane::Download, "/boxapi/one", "AAA"),
                            job(Lane::Download, "/boxapi/two", "AAA"),
                        ],
                        1,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&script, 1).await;
            venue
                .app
                .operator
                .credential_file()
                .update(|credentials| {
                    credentials.bind("BBB", "stored-token", Some("Next".to_string()))
                })
                .unwrap();
            assert_eq!(
                CurrentEvent::lock(&venue.app.current)
                    .switch_to("BBB")
                    .unwrap(),
                "BBB"
            );
            pending.await.unwrap();
            let before = script.paths().len();
            script.release(8);
            sync_wake(&venue.app, &remote, &[], Duration::from_secs(2), 2).await;
            let paths = script.paths();
            let queries = script.queries();
            let probe = paths
                .iter()
                .zip(queries.iter())
                .skip(before)
                .find(|(path, _)| path.as_str() == "/boxapi/barcodesinfo");
            let Some((_, query)) = probe else {
                panic!("next wake did not probe, hits={paths:?}");
            };
            assert!(query.contains("eid=BBB"), "{query}");
        }
    }

    mod cursors {
        use super::support::{job, venue, Script};
        use super::*;
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::DB_FILE;
        use crate::registration_server::remote_server::RemoteServer;
        use axum::extract::State;
        use axum::routing::get;
        use axum::Router;
        use rusqlite::OptionalExtension;
        use std::collections::HashMap;
        use std::path::Path;
        use std::sync::Mutex as StdMutex;

        fn tracked(event_id: &str, name: &str) -> Job {
            let mut job = job(Lane::Download, "/boxapi/cursor", event_id);
            job.cursors = vec![Cursor {
                name: name.to_string(),
                send_as: "last_synch",
                take_from: "last_synch",
            }];
            job
        }

        fn stored(dir: &Path, event_id: &str, name: &str) -> Option<String> {
            let conn = rusqlite::Connection::open(dir.join(event_id).join(DB_FILE)).unwrap();
            conn.query_row(
                "SELECT value FROM sync_cursor WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
        }

        fn stored_count(dir: &Path, event_id: &str) -> i64 {
            let conn = rusqlite::Connection::open(dir.join(event_id).join(DB_FILE)).unwrap();
            conn.query_row("SELECT COUNT(*) FROM sync_cursor", [], |row| row.get(0))
                .unwrap()
        }

        #[tokio::test]
        async fn a_missing_cursor_is_sent_as_zero() {
            let mut bodies = HashMap::new();
            bodies.insert("/boxapi/cursor".to_string(), r#"{"ok":true}"#.to_string());
            let script = Script::start(bodies).await;
            script.release(1);
            let (venue, dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                vec![tracked("EVT1", "zones")],
                1,
                Duration::from_secs(2),
            )
            .await;
            let query = &script.queries()[0];
            assert!(query.contains("last_synch=0"), "{query}");
            assert_eq!(stored_count(&dir, "EVT1"), 0);
        }

        #[tokio::test]
        async fn a_commit_stores_the_cursor() {
            let mut bodies = HashMap::new();
            bodies.insert(
                "/boxapi/cursor".to_string(),
                r#"{"last_synch":15}"#.to_string(),
            );
            let script = Script::start(bodies).await;
            script.release(4);
            let (venue, dir) = venue("EVT1");
            let remote = RemoteServer::new(&script.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                vec![tracked("EVT1", "zones"), tracked("EVT1", "zones")],
                2,
                Duration::from_secs(2),
            )
            .await;
            let queries = script.queries();
            assert!(queries[0].contains("last_synch=0"), "{}", queries[0]);
            assert!(queries[1].contains("last_synch=15"), "{}", queries[1]);
            assert_eq!(stored(&dir, "EVT1", "zones").as_deref(), Some("15"));
        }

        #[tokio::test]
        async fn a_failed_call_keeps_the_cursor() {
            let hits = Arc::new(StdMutex::new(Vec::new()));
            let recorded = Arc::clone(&hits);
            let app = Router::new()
                .fallback(get(
                    |State(hits): State<Arc<StdMutex<Vec<String>>>>,
                     req: axum::extract::Request| async move {
                        hits.lock()
                            .unwrap()
                            .push(req.uri().query().unwrap_or("").to_string());
                        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "nope")
                    },
                ))
                .with_state(recorded);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let (venue, dir) = venue("EVT1");
            CurrentEvent::lock(&venue.app.current)
                .set_cursor("EVT1", "zones", "15")
                .unwrap();
            let remote = RemoteServer::new(&format!("http://{addr}/")).unwrap();
            let outcomes = drain_queue(
                &venue.app,
                &remote,
                vec![tracked("EVT1", "zones")],
                1,
                Duration::from_secs(2),
            )
            .await;
            handle.abort();
            assert_eq!(outcomes, vec![("/boxapi/cursor".to_string(), "error")]);
            assert!(hits.lock().unwrap()[0].contains("last_synch=15"));
            assert_eq!(stored(&dir, "EVT1", "zones").as_deref(), Some("15"));
        }

        #[tokio::test]
        async fn a_new_event_has_no_cursors() {
            let (venue, dir) = venue("AAA");
            CurrentEvent::lock(&venue.app.current)
                .set_cursor("AAA", "zones", "15")
                .unwrap();
            assert_eq!(
                CurrentEvent::lock(&venue.app.current)
                    .switch_to("BBB")
                    .unwrap(),
                "BBB"
            );
            assert_eq!(stored_count(&dir, "BBB"), 0);
            assert_eq!(stored(&dir, "AAA", "zones").as_deref(), Some("15"));
        }
    }

    mod enums {
        use super::support::{job, venue};
        use super::*;
        use crate::registration_server::event_db::DB_FILE;
        use crate::registration_server::remote_server::RemoteServer;
        use axum::extract::State;
        use axum::routing::{get, post};
        use axum::Router;
        use rusqlite::OptionalExtension;
        use std::collections::HashMap;
        use std::path::Path;
        use std::sync::Mutex as StdMutex;

        #[derive(Clone)]
        struct Hit {
            path: String,
            body: String,
        }

        struct Stand {
            base: String,
            hits: Arc<StdMutex<Vec<Hit>>>,
            release: watch::Sender<usize>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Stand {
            async fn start(bodies: HashMap<String, String>) -> Self {
                let hits = Arc::new(StdMutex::new(Vec::new()));
                let (release, gate) = watch::channel(0usize);
                let app = Router::new()
                    .route("/boxapi/dbenums", post(answer))
                    .route("/boxapi/dbenum", post(answer))
                    .route("/boxapi/other", get(answer))
                    .with_state((Arc::clone(&hits), gate, bodies));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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

            pub(super) fn release(&self, n: usize) {
                let _ = self.release.send(n);
            }

            pub(super) fn hits(&self) -> Vec<Hit> {
                self.hits.lock().unwrap().clone()
            }
        }

        impl Drop for Stand {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn answer(
            State((hits, mut gate, bodies)): State<(
                Arc<StdMutex<Vec<Hit>>>,
                watch::Receiver<usize>,
                HashMap<String, String>,
            )>,
            req: axum::extract::Request,
        ) -> String {
            let path = req.uri().path().to_string();
            let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                .await
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let ticket = {
                let mut hits = hits.lock().unwrap();
                hits.push(Hit {
                    path: path.clone(),
                    body,
                });
                hits.len()
            };
            let _ = gate.wait_for(|done| *done >= ticket).await;
            bodies.get(&path).cloned().unwrap_or_else(|| "{}".into())
        }

        pub(super) fn form_map(raw: &str) -> HashMap<String, String> {
            let mut pairs = HashMap::new();
            for pair in raw.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                pairs.insert(percent_decode(name), percent_decode(value));
            }
            pairs
        }

        fn percent_decode(raw: &str) -> String {
            let bytes = raw.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'+' {
                    out.push(b' ');
                    i += 1;
                } else if bytes[i] == b'%' && i + 2 < bytes.len() {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        out.push(byte);
                        i += 3;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }

        fn stored(dir: &Path, event_id: &str, name: &str) -> Option<String> {
            let conn = rusqlite::Connection::open(dir.join(event_id).join(DB_FILE)).unwrap();
            conn.query_row(
                "SELECT value FROM sync_cursor WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
        }

        async fn until(stand: &Stand, n: usize) {
            let started = tokio::time::Instant::now();
            while stand.hits().len() < n {
                if started.elapsed() > Duration::from_secs(2) {
                    panic!(
                        "timed out waiting for {n} requests, saw {}",
                        stand.hits().len()
                    );
                }
                tokio::task::yield_now().await;
            }
        }

        fn two_deps() -> String {
            r#"{"version":3,"requested_enums":[{"name":"region","ids":{"country":1}},{"name":"city","ids":{"region":4}}]}"#
                .to_string()
        }

        #[tokio::test]
        async fn dbenums_is_queued_with_the_other_downloads() {
            let mut bodies = HashMap::new();
            bodies.insert(
                "/boxapi/dbenums".to_string(),
                r#"{"version":1}"#.to_string(),
            );
            bodies.insert("/boxapi/other".to_string(), "{}".to_string());
            let stand = Stand::start(bodies).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            let mut jobs = vec![dbenums_job("EVT1")];
            jobs.push(job(Lane::Download, "/boxapi/other", "EVT1"));
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, jobs, 2, Duration::from_secs(2)).await
                })
            };
            until(&stand, 2).await;
            let hits = stand.hits();
            let paths: Vec<&str> = hits.iter().map(|hit| hit.path.as_str()).collect();
            assert!(paths.contains(&"/boxapi/dbenums"), "{paths:?}");
            assert!(paths.contains(&"/boxapi/other"), "{paths:?}");
            let form = form_map(
                &hits
                    .iter()
                    .find(|hit| hit.path == "/boxapi/dbenums")
                    .unwrap()
                    .body,
            );
            assert_eq!(
                form.get("enums").map(String::as_str),
                Some("country,region,city")
            );
            assert_eq!(form.get("version").map(String::as_str), Some("0"));
            assert_eq!(
                form.get("langs").map(String::as_str),
                Some("ru,en,phone_code,phone_mask")
            );
            assert_eq!(form.get("dev_id").map(String::as_str), Some("device-1"));
            assert_eq!(form.get("token").map(String::as_str), Some("stored-token"));
            assert_eq!(form.get("eid").map(String::as_str), Some("EVT1"));
            stand.release(2);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn dbenum_follows_each_dependency() {
            let mut bodies = HashMap::new();
            bodies.insert("/boxapi/dbenums".to_string(), two_deps());
            bodies.insert("/boxapi/dbenum".to_string(), "{}".to_string());
            let stand = Stand::start(bodies).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            let jobs = vec![dbenums_job("EVT1")];
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, jobs, 2, Duration::from_secs(2)).await
                })
            };
            until(&stand, 1).await;
            stand.release(1);
            until(&stand, 2).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let enums: Vec<Hit> = stand
                .hits()
                .into_iter()
                .filter(|hit| hit.path == "/boxapi/dbenum")
                .collect();
            assert_eq!(enums.len(), 1);
            let first = form_map(&enums[0].body);
            assert_eq!(first.get("enum").map(String::as_str), Some("region"));
            assert_eq!(first.get("gzipped").map(String::as_str), Some("false"));
            assert_eq!(first.get("country").map(String::as_str), Some("1"));
            stand.release(2);
            until(&stand, 3).await;
            let enums: Vec<Hit> = stand
                .hits()
                .into_iter()
                .filter(|hit| hit.path == "/boxapi/dbenum")
                .collect();
            assert_eq!(enums.len(), 2);
            let second = form_map(&enums[1].body);
            assert_eq!(second.get("enum").map(String::as_str), Some("city"));
            assert_eq!(second.get("gzipped").map(String::as_str), Some("false"));
            assert_eq!(second.get("region").map(String::as_str), Some("4"));
            stand.release(3);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn the_version_moves_when_dbenums_commits() {
            let mut bodies = HashMap::new();
            bodies.insert("/boxapi/dbenums".to_string(), two_deps());
            bodies.insert("/boxapi/dbenum".to_string(), "{}".to_string());
            let stand = Stand::start(bodies).await;
            let (venue, dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            let jobs = vec![dbenums_job("EVT1")];
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, jobs, 2, Duration::from_secs(2)).await
                })
            };
            until(&stand, 1).await;
            stand.release(1);
            until(&stand, 2).await;
            assert_eq!(stored(&dir, "EVT1", "dbenums").as_deref(), Some("3"));
            let dbenums = stand
                .hits()
                .into_iter()
                .filter(|hit| hit.path == "/boxapi/dbenum")
                .count();
            assert_eq!(dbenums, 1);
            stand.release(4);
            pending.await.unwrap();
        }
    }

    mod forms {
        use super::support::venue;
        use super::*;
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::DB_FILE;
        use crate::registration_server::remote_server::RemoteServer;
        use axum::extract::State;
        use axum::routing::post;
        use axum::Router;
        use std::collections::HashMap;
        use std::path::Path;
        use std::sync::Mutex as StdMutex;

        #[derive(Clone)]
        struct Hit {
            path: String,
            body: String,
        }

        struct Stand {
            base: String,
            hits: Arc<StdMutex<Vec<Hit>>>,
            release: watch::Sender<usize>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Stand {
            async fn start(bodies: HashMap<String, String>) -> Self {
                let hits = Arc::new(StdMutex::new(Vec::new()));
                let (release, gate) = watch::channel(0usize);
                let app = Router::new()
                    .route("/boxapi/getconf", post(answer))
                    .route("/boxapi/getbg", post(answer))
                    .route("/boxapi/ticketbarcodes", post(answer))
                    .with_state((Arc::clone(&hits), gate, bodies));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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

            pub(super) fn release(&self, n: usize) {
                let _ = self.release.send(n);
            }

            pub(super) fn hits(&self) -> Vec<Hit> {
                self.hits.lock().unwrap().clone()
            }
        }

        impl Drop for Stand {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn answer(
            State((hits, mut gate, bodies)): State<(
                Arc<StdMutex<Vec<Hit>>>,
                watch::Receiver<usize>,
                HashMap<String, String>,
            )>,
            req: axum::extract::Request,
        ) -> String {
            let path = req.uri().path().to_string();
            let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                .await
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let ticket = {
                let mut hits = hits.lock().unwrap();
                hits.push(Hit {
                    path: path.clone(),
                    body,
                });
                hits.len()
            };
            let _ = gate.wait_for(|done| *done >= ticket).await;
            bodies.get(&path).cloned().unwrap_or_else(|| "{}".into())
        }

        pub(super) fn form_map(raw: &str) -> HashMap<String, String> {
            let mut pairs = HashMap::new();
            for pair in raw.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                pairs.insert(percent_decode(name), percent_decode(value));
            }
            pairs
        }

        fn percent_decode(raw: &str) -> String {
            let bytes = raw.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'+' {
                    out.push(b' ');
                    i += 1;
                } else if bytes[i] == b'%' && i + 2 < bytes.len() {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        out.push(byte);
                        i += 3;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }

        fn conf_body() -> String {
            r#"{"status":"ok","last_synch":2,"forms":{"list":[{"id":9}],"change":5}}"#.to_string()
        }

        fn ticket_body() -> String {
            "1\n0\n4\n1\n1\nABC\n".to_string()
        }

        fn stand_bodies() -> HashMap<String, String> {
            let mut bodies = HashMap::new();
            bodies.insert("/boxapi/getconf".to_string(), conf_body());
            bodies.insert("/boxapi/getbg".to_string(), "PK".to_string());
            bodies.insert("/boxapi/ticketbarcodes".to_string(), ticket_body());
            bodies
        }

        async fn until(stand: &Stand, n: usize) {
            let started = tokio::time::Instant::now();
            while stand.hits().len() < n {
                if started.elapsed() > Duration::from_secs(2) {
                    panic!(
                        "timed out waiting for {n} requests, saw {}",
                        stand.hits().len()
                    );
                }
                tokio::task::yield_now().await;
            }
        }

        fn ticket_rows(dir: &Path) -> Vec<(String, i64)> {
            let conn = rusqlite::Connection::open(dir.join("EVT1").join(DB_FILE)).unwrap();
            let mut stmt = conn
                .prepare("SELECT barcode, formId FROM ticketbarcodes ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .map(|row| row.unwrap())
                .collect()
        }

        #[tokio::test]
        async fn getconf_starts_with_an_empty_form() {
            let stand = Stand::start(stand_bodies()).await;
            let (venue, _dir) = venue("EVT1");
            CurrentEvent::lock(&venue.app.current)
                .set_cursor("EVT1", "getconf", "8")
                .unwrap();
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![empty_getconf("EVT1")],
                        4,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&stand, 1).await;
            let form = form_map(&stand.hits()[0].body);
            assert_eq!(stand.hits()[0].path, "/boxapi/getconf");
            assert_eq!(form.get("conf_id").map(String::as_str), Some(""));
            assert_eq!(form.get("last_synch").map(String::as_str), Some("8"));
            stand.release(8);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn each_form_gets_backgrounds_and_tickets() {
            let stand = Stand::start(stand_bodies()).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![empty_getconf("EVT1")],
                        4,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&stand, 1).await;
            stand.release(1);
            until(&stand, 4).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let hits = stand.hits();
            assert_eq!(hits.len(), 4);
            let forms: Vec<HashMap<String, String>> = hits
                .iter()
                .filter(|hit| hit.path == "/boxapi/getconf")
                .map(|hit| form_map(&hit.body))
                .collect();
            assert_eq!(forms.len(), 2);
            assert!(forms
                .iter()
                .any(|form| form.get("conf_id").map(String::as_str) == Some("9")));
            let pads: Vec<_> = hits
                .iter()
                .filter(|hit| hit.path == "/boxapi/getbg")
                .map(|hit| form_map(&hit.body))
                .collect();
            assert_eq!(pads.len(), 1);
            assert_eq!(pads[0].get("resolution").map(String::as_str), Some("pad"));
            assert_eq!(pads[0].get("conf_id").map(String::as_str), Some("9"));
            let tickets: Vec<_> = hits
                .iter()
                .filter(|hit| hit.path == "/boxapi/ticketbarcodes")
                .collect();
            assert_eq!(tickets.len(), 1);
            assert_eq!(
                form_map(&tickets[0].body)
                    .get("conf_id")
                    .map(String::as_str),
                Some("9")
            );
            assert!(hits.iter().all(|hit| {
                hit.path != "/boxapi/getbg"
                    || form_map(&hit.body).get("resolution").map(String::as_str) != Some("phone")
            }));
            stand.release(4);
            until(&stand, 5).await;
            let phone = stand
                .hits()
                .into_iter()
                .filter(|hit| hit.path == "/boxapi/getbg")
                .map(|hit| form_map(&hit.body))
                .find(|form| form.get("resolution").map(String::as_str) == Some("phone"));
            let phone = phone.unwrap();
            assert_eq!(phone.get("conf_id").map(String::as_str), Some("9"));
            stand.release(8);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn ticket_barcodes_land_in_the_file() {
            let stand = Stand::start(stand_bodies()).await;
            stand.release(20);
            let (venue, dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                vec![empty_getconf("EVT1")],
                4,
                Duration::from_secs(2),
            )
            .await;
            assert_eq!(ticket_rows(&dir), vec![("ABC".to_string(), 9)]);
            drain_queue(
                &venue.app,
                &remote,
                vec![ticket_job("EVT1", "9")],
                1,
                Duration::from_secs(2),
            )
            .await;
            assert_eq!(ticket_rows(&dir), vec![("ABC".to_string(), 9)]);
        }
    }

    mod codes {
        use super::support::venue;
        use super::*;
        use crate::registration_server::event_db::DB_FILE;
        use crate::registration_server::remote_server::RemoteServer;
        use axum::extract::State;
        use axum::routing::{get, post};
        use axum::Router;
        use rusqlite::OptionalExtension;
        use std::collections::HashMap;
        use std::path::Path;
        use std::sync::Mutex as StdMutex;

        #[derive(Clone)]
        struct Hit {
            method: String,
            path: String,
            body: String,
        }

        struct Stand {
            base: String,
            hits: Arc<StdMutex<Vec<Hit>>>,
            seq: watch::Sender<usize>,
            info: watch::Sender<bool>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Stand {
            async fn start(bodies: HashMap<String, String>, split_info: bool) -> Self {
                let hits = Arc::new(StdMutex::new(Vec::new()));
                let (seq, seq_gate) = watch::channel(0usize);
                let (info, info_gate) = watch::channel(false);
                let app = Router::new()
                    .route("/boxapi/barcodesinfo", get(answer))
                    .route("/boxapi/control_zones", post(answer))
                    .route("/boxapi/dbenums", post(answer))
                    .route("/boxapi/getconf", post(answer))
                    .route("/boxapi/getbadge", post(answer))
                    .route("/boxapi/barcodes", post(answer))
                    .route("/tracked/UniRegUser", post(answer))
                    .with_state((Arc::clone(&hits), seq_gate, info_gate, split_info, bodies));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let handle = tokio::spawn(async move {
                    axum::serve(listener, app).await.unwrap();
                });
                Self {
                    base: format!("http://{addr}/"),
                    hits,
                    seq,
                    info,
                    handle,
                }
            }

            fn release_seq(&self, n: usize) {
                let _ = self.seq.send(n);
            }

            fn release_info(&self) {
                let _ = self.info.send(true);
            }

            fn hits(&self) -> Vec<Hit> {
                self.hits.lock().unwrap().clone()
            }
        }

        impl Drop for Stand {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn answer(
            State((hits, mut seq, mut info, split_info, bodies)): State<(
                Arc<StdMutex<Vec<Hit>>>,
                watch::Receiver<usize>,
                watch::Receiver<bool>,
                bool,
                HashMap<String, String>,
            )>,
            req: axum::extract::Request,
        ) -> String {
            let method = req.method().as_str().to_string();
            let path = req.uri().path().to_string();
            let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                .await
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let hold_info = split_info && path == "/boxapi/barcodesinfo";
            let ticket = {
                let mut hits = hits.lock().unwrap();
                hits.push(Hit {
                    method,
                    path: path.clone(),
                    body,
                });
                hits.iter()
                    .filter(|hit| !(split_info && hit.path == "/boxapi/barcodesinfo"))
                    .count()
            };
            if hold_info {
                let _ = info.wait_for(|go| *go).await;
            } else {
                let _ = seq.wait_for(|done| *done >= ticket).await;
            }
            bodies.get(&path).cloned().unwrap_or_else(|| "{}".into())
        }

        pub(super) fn form_map(raw: &str) -> HashMap<String, String> {
            let mut pairs = HashMap::new();
            for pair in raw.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                pairs.insert(percent_decode(name), percent_decode(value));
            }
            pairs
        }

        fn percent_decode(raw: &str) -> String {
            let bytes = raw.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'+' {
                    out.push(b' ');
                    i += 1;
                } else if bytes[i] == b'%' && i + 2 < bytes.len() {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        out.push(byte);
                        i += 3;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }

        fn info_body() -> String {
            r#"{"info":[{"cat_id":3,"clean_ids":[{"bcType":0,"clean_id":9}]}]}"#.to_string()
        }

        fn cats_body() -> String {
            r#"{"status":"ok","last_synch":1,"cats":[{"cat_id":3},{"cat_id":7}],"forms":{"list":[],"change":1}}"#
                .to_string()
        }

        fn pool_body() -> String {
            "1\n3\n4\n1\n1\nABC\n".to_string()
        }

        async fn until(stand: &Stand, pred: impl Fn(&[Hit]) -> bool) {
            let started = tokio::time::Instant::now();
            loop {
                let hits = stand.hits();
                if pred(&hits) {
                    return;
                }
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("timed out, saw {} requests", hits.len());
                }
                tokio::task::yield_now().await;
            }
        }

        fn stored(dir: &Path, name: &str) -> Option<String> {
            let conn = rusqlite::Connection::open(dir.join("EVT1").join(DB_FILE)).unwrap();
            conn.query_row(
                "SELECT value FROM sync_cursor WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
        }

        fn barcode_rows(dir: &Path) -> Vec<(String, i64, i64)> {
            let conn = rusqlite::Connection::open(dir.join("EVT1").join(DB_FILE)).unwrap();
            let mut stmt = conn
                .prepare("SELECT barcode, type, clean_id FROM barcodes ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .map(|row| row.unwrap())
                .collect()
        }

        #[tokio::test]
        async fn info_is_not_the_probe() {
            let mut bodies = HashMap::new();
            bodies.insert("/boxapi/barcodesinfo".to_string(), info_body());
            bodies.insert(
                "/boxapi/control_zones".to_string(),
                r#"{"status":"ok","last_synch":4,"zones":[]}"#.to_string(),
            );
            bodies.insert(
                "/boxapi/dbenums".to_string(),
                r#"{"version":1}"#.to_string(),
            );
            bodies.insert(
                "/boxapi/getconf".to_string(),
                r#"{"status":"ok","forms":{"list":[],"change":0}}"#.to_string(),
            );
            bodies.insert("/boxapi/barcodes".to_string(), pool_body());
            let stand = Stand::start(bodies, false).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                let remote = remote.clone();
                tokio::spawn(async move {
                    sync_wake(&app, &remote, &[], Duration::from_secs(2), 8).await
                })
            };
            until(&stand, |hits| hits.len() >= 1).await;
            assert_eq!(stand.hits()[0].method, "GET");
            assert_eq!(stand.hits()[0].path, "/boxapi/barcodesinfo");
            assert!(stand
                .hits()
                .iter()
                .all(|hit| hit.path != "/boxapi/barcodes"));
            stand.release_seq(1);
            until(&stand, |hits| {
                hits.iter()
                    .filter(|hit| hit.path == "/boxapi/barcodesinfo")
                    .count()
                    >= 2
                    && hits.iter().any(|hit| hit.path == "/boxapi/control_zones")
            })
            .await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let hits = stand.hits();
            assert_eq!(
                hits.iter()
                    .filter(|hit| hit.path == "/boxapi/barcodesinfo")
                    .count(),
                2
            );
            assert!(hits
                .iter()
                .any(|hit| hit.method == "POST" && hit.path == "/boxapi/control_zones"));
            assert!(hits.iter().all(|hit| hit.path != "/boxapi/barcodes"));
            stand.release_seq(20);
            until(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/barcodes")
            })
            .await;
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn badges_follow_the_categories() {
            let mut bodies = HashMap::new();
            bodies.insert("/boxapi/getconf".to_string(), cats_body());
            bodies.insert("/boxapi/getbadge".to_string(), "PK".to_string());
            bodies.insert("/boxapi/barcodesinfo".to_string(), info_body());
            bodies.insert("/boxapi/barcodes".to_string(), pool_body());
            let stand = Stand::start(bodies, true).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![empty_getconf("EVT1"), info_job("EVT1")],
                        4,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/getconf")
            })
            .await;
            stand.release_seq(1);
            until(&stand, |hits| {
                hits.iter()
                    .filter(|hit| hit.path == "/boxapi/getbadge")
                    .count()
                    >= 1
            })
            .await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let badges: Vec<_> = stand
                .hits()
                .into_iter()
                .filter(|hit| hit.path == "/boxapi/getbadge")
                .map(|hit| form_map(&hit.body))
                .collect();
            assert_eq!(badges.len(), 1);
            assert_eq!(badges[0].get("cat_id").map(String::as_str), Some("3"));
            assert_eq!(badges[0].get("is_cert").map(String::as_str), Some("false"));
            assert!(stand
                .hits()
                .iter()
                .all(|hit| hit.path != "/boxapi/barcodes"));
            stand.release_seq(2);
            until(&stand, |hits| {
                hits.iter()
                    .filter(|hit| hit.path == "/boxapi/getbadge")
                    .count()
                    >= 2
            })
            .await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let badges: Vec<_> = stand
                .hits()
                .into_iter()
                .filter(|hit| hit.path == "/boxapi/getbadge")
                .map(|hit| form_map(&hit.body))
                .collect();
            assert_eq!(badges.len(), 2);
            assert_eq!(badges[1].get("cat_id").map(String::as_str), Some("7"));
            assert_eq!(badges[1].get("is_cert").map(String::as_str), Some("false"));
            assert!(stand
                .hits()
                .iter()
                .all(|hit| hit.path != "/boxapi/barcodes"));
            stand.release_info();
            until(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/barcodes")
            })
            .await;
            stand.release_seq(20);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn barcode_rows_land_in_the_file() {
            let mut bodies = HashMap::new();
            bodies.insert("/boxapi/barcodesinfo".to_string(), info_body());
            bodies.insert("/boxapi/barcodes".to_string(), pool_body());
            let stand = Stand::start(bodies, false).await;
            stand.release_seq(20);
            let (venue, dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                vec![info_job("EVT1")],
                2,
                Duration::from_secs(2),
            )
            .await;
            assert_eq!(barcode_rows(&dir), vec![("ABC".to_string(), 3, 4)]);
            assert_eq!(stored(&dir, "barcodes:3:0").as_deref(), Some("4"));
        }
    }

    mod visitors {
        use super::support::venue;
        use super::*;
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::{ListOrder, DB_FILE};
        use crate::registration_server::remote_server::RemoteServer;
        use axum::extract::State;
        use axum::routing::post;
        use axum::Router;
        use rusqlite::OptionalExtension;
        use std::path::Path;
        use std::sync::Mutex as StdMutex;

        #[derive(Clone)]
        struct Hit {
            body: String,
        }

        struct Stand {
            base: String,
            hits: Arc<StdMutex<Vec<Hit>>>,
            release: watch::Sender<usize>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Stand {
            async fn start(pages: Vec<String>) -> Self {
                let hits = Arc::new(StdMutex::new(Vec::new()));
                let (release, gate) = watch::channel(0usize);
                let app = Router::new()
                    .route("/tracked/UniRegUser", post(answer))
                    .with_state((Arc::clone(&hits), gate, pages));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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

            fn release(&self, n: usize) {
                let _ = self.release.send(n);
            }

            fn hits(&self) -> Vec<Hit> {
                self.hits.lock().unwrap().clone()
            }
        }

        impl Drop for Stand {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn answer(
            State((hits, mut gate, pages)): State<(
                Arc<StdMutex<Vec<Hit>>>,
                watch::Receiver<usize>,
                Vec<String>,
            )>,
            req: axum::extract::Request,
        ) -> String {
            let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                .await
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let ticket = {
                let mut hits = hits.lock().unwrap();
                hits.push(Hit { body });
                hits.len()
            };
            let _ = gate.wait_for(|done| *done >= ticket).await;
            pages.get(ticket - 1).cloned().unwrap_or_else(|| {
                pages
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "{\"arr\":[]}".into())
            })
        }

        fn form_map(raw: &str) -> std::collections::HashMap<String, String> {
            let mut pairs = std::collections::HashMap::new();
            for pair in raw.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                pairs.insert(percent_decode(name), percent_decode(value));
            }
            pairs
        }

        fn percent_decode(raw: &str) -> String {
            let bytes = raw.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'+' {
                    out.push(b' ');
                    i += 1;
                } else if bytes[i] == b'%' && i + 2 < bytes.len() {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        out.push(byte);
                        i += 3;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }

        fn page(count: usize, ts: i64, surname: &str) -> String {
            let arr = (0..count)
                .map(|i| {
                    format!(
                        r#"{{"ts":{ts},"obj":{{"uid":"{ts}-{i}","name":"ivan","surname":"{surname}","email":"u{i}@example.com"}}}}"#
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(r#"{{"arr":[{arr}]}}"#)
        }

        async fn until(stand: &Stand, n: usize) {
            let started = tokio::time::Instant::now();
            while stand.hits().len() < n {
                if started.elapsed() > Duration::from_secs(5) {
                    panic!(
                        "timed out waiting for {n} requests, saw {}",
                        stand.hits().len()
                    );
                }
                tokio::task::yield_now().await;
            }
        }

        fn stored(dir: &Path, event_id: &str, name: &str) -> Option<String> {
            let conn = rusqlite::Connection::open(dir.join(event_id).join(DB_FILE)).unwrap();
            conn.query_row(
                "SELECT value FROM sync_cursor WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
        }

        fn file_uids(dir: &Path, event_id: &str) -> Vec<String> {
            let conn = rusqlite::Connection::open(dir.join(event_id).join(DB_FILE)).unwrap();
            let mut stmt = conn.prepare("SELECT uid FROM mem_u ORDER BY uid").unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .map(|row| row.unwrap())
                .collect()
        }

        #[tokio::test]
        async fn visitors_are_pages_of_100() {
            let stand = Stand::start(vec![page(100, 500, "petrov"), page(40, 640, "petrov")]).await;
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(
                        &app,
                        &remote,
                        vec![visitors_job("EVT1")],
                        2,
                        Duration::from_secs(2),
                    )
                    .await
                })
            };
            until(&stand, 1).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(stand.hits().len(), 1);
            let form = form_map(&stand.hits()[0].body);
            assert_eq!(form.get("limit").map(String::as_str), Some("100"));
            assert_eq!(form.get("from_time").map(String::as_str), Some("0"));
            assert_eq!(
                form.get("json").map(String::as_str),
                Some("EVT1_reg_track_request")
            );
            stand.release(1);
            until(&stand, 2).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(stand.hits().len(), 2);
            let second = form_map(&stand.hits()[1].body);
            assert_eq!(second.get("from_time").map(String::as_str), Some("500"));
            assert_eq!(second.get("limit").map(String::as_str), Some("100"));
            stand.release(2);
            tokio::time::sleep(Duration::from_millis(80)).await;
            assert_eq!(stand.hits().len(), 2);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn the_file_is_written_before_memory() {
            let stand = Stand::start(vec![page(1, 20, "petrov")]).await;
            stand.release(4);
            let (venue, dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                vec![visitors_job("EVT1")],
                1,
                Duration::from_secs(2),
            )
            .await;
            assert_eq!(file_uids(&dir, "EVT1"), vec!["20-0".to_string()]);
            let row = CurrentEvent::lock(&venue.app.current)
                .full_row("EVT1", "20-0")
                .unwrap();
            assert!(row.value.unwrap().contains("petrov"));
            let page = CurrentEvent::lock(&venue.app.current)
                .search("EVT1", "petrov", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.value.total, 1);
            assert_eq!(page.value.rows[0].uid, "20-0");
            assert_eq!(
                CurrentEvent::lock(&venue.app.current)
                    .loaded_count()
                    .unwrap(),
                1
            );
        }

        #[tokio::test]
        async fn a_switched_event_does_not_take_the_page() {
            let stand = Stand::start(vec![page(1, 99, "petrov")]).await;
            stand.release(4);
            let (venue, dir) = venue("AAA");
            CurrentEvent::lock(&venue.app.current)
                .set_cursor("AAA", "visitors", "11")
                .unwrap();
            let remote = RemoteServer::new(&stand.base).unwrap();
            let log = Arc::new(StdMutex::new(Vec::new()));
            let (hold_tx, hold_rx) = watch::channel(false);
            let mut job = visitors_job("AAA");
            job.hold_parse = Some(hold_rx);
            job.log = Some(Arc::clone(&log));
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, vec![job], 1, Duration::from_secs(2)).await
                })
            };
            let started = tokio::time::Instant::now();
            loop {
                if log
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|event| event.starts_with("start "))
                {
                    break;
                }
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("parse did not start");
                }
                tokio::task::yield_now().await;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert_eq!(
                CurrentEvent::lock(&venue.app.current)
                    .switch_to("BBB")
                    .unwrap(),
                "BBB"
            );
            hold_tx.send(true).unwrap();
            pending.await.unwrap();
            assert!(log.lock().unwrap().iter().any(|event| event == "NotOpen"));
            assert_eq!(stored(&dir, "AAA", "visitors").as_deref(), Some("11"));
            assert!(file_uids(&dir, "BBB").is_empty());
            let page = CurrentEvent::lock(&venue.app.current)
                .search("BBB", "petrov", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.value.total, 0);
        }
    }

    mod rest {
        use super::support::venue;
        use super::*;
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::{VisitorWrite, DB_FILE};
        use crate::registration_server::remote_server::RemoteServer;
        use axum::extract::State;
        use axum::routing::any;
        use axum::Router;
        use std::collections::HashMap;
        use std::path::Path;
        use std::sync::Mutex as StdMutex;

        #[derive(Clone)]
        pub(super) struct Hit {
            pub(super) path: String,
            pub(super) body: String,
        }

        pub(super) struct Stand {
            pub(super) base: String,
            hits: Arc<StdMutex<Vec<Hit>>>,
            release: watch::Sender<usize>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Stand {
            pub(super) async fn start(bodies: HashMap<String, String>) -> Self {
                let hits = Arc::new(StdMutex::new(Vec::new()));
                let (release, gate) = watch::channel(0usize);
                let app = Router::new().fallback(any(answer)).with_state((
                    Arc::clone(&hits),
                    gate,
                    bodies,
                ));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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

            pub(super) fn release(&self, n: usize) {
                let _ = self.release.send(n);
            }

            pub(super) fn hits(&self) -> Vec<Hit> {
                self.hits.lock().unwrap().clone()
            }
        }

        impl Drop for Stand {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn answer(
            State((hits, mut gate, bodies)): State<(
                Arc<StdMutex<Vec<Hit>>>,
                watch::Receiver<usize>,
                HashMap<String, String>,
            )>,
            req: axum::extract::Request,
        ) -> String {
            let path = req.uri().path().to_string();
            let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                .await
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let ticket = {
                let mut hits = hits.lock().unwrap();
                hits.push(Hit {
                    path: path.clone(),
                    body,
                });
                hits.len()
            };
            let _ = gate.wait_for(|done| *done >= ticket).await;
            bodies.get(&path).cloned().unwrap_or_else(|| "{}".into())
        }

        pub(super) fn form_map(raw: &str) -> HashMap<String, String> {
            let mut pairs = HashMap::new();
            for pair in raw.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                pairs.insert(percent_decode(name), percent_decode(value));
            }
            pairs
        }

        fn percent_decode(raw: &str) -> String {
            let bytes = raw.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'+' {
                    out.push(b' ');
                    i += 1;
                } else if bytes[i] == b'%' && i + 2 < bytes.len() {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        out.push(byte);
                        i += 3;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }

        fn waiting(uid: &str) -> VisitorWrite {
            VisitorWrite {
                uid: uid.to_string(),
                data: "{}".to_string(),
                name: "ivan".to_string(),
                surname: "petrov".to_string(),
                c_name: String::new(),
                category: 0,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 1,
                org_id: None,
                email: String::new(),
            }
        }

        pub(super) async fn until(stand: &Stand, pred: impl Fn(&[Hit]) -> bool) {
            let started = tokio::time::Instant::now();
            loop {
                let hits = stand.hits();
                if pred(&hits) {
                    return;
                }
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("timed out, saw {} requests", hits.len());
                }
                tokio::task::yield_now().await;
            }
        }

        fn scan_rows(dir: &Path) -> Vec<(i64, i64, String)> {
            let conn = rusqlite::Connection::open(dir.join("EVT1").join(DB_FILE)).unwrap();
            let mut stmt = conn
                .prepare("SELECT scanid, zone_id, barcode FROM scans ORDER BY scanid")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .map(|row| row.unwrap())
                .collect()
        }

        #[tokio::test]
        async fn managers_are_skipped_without_an_org() {
            let stand = Stand::start(HashMap::new()).await;
            stand.release(40);
            let (venue, _dir) = venue("EVT1");
            let remote = RemoteServer::new(&stand.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                catalog(&venue.app),
                20,
                Duration::from_secs(2),
            )
            .await;
            let paths: Vec<_> = stand.hits().into_iter().map(|hit| hit.path).collect();
            assert!(
                paths.iter().all(|path| path != "/boxapi/getmanagers"),
                "{paths:?}"
            );
            assert!(
                paths.iter().all(|path| path != "/tracked/OrgRegUser"),
                "{paths:?}"
            );
        }

        #[tokio::test]
        async fn scans_follow_the_zone() {
            let mut bodies = HashMap::new();
            bodies.insert(
                "/tracked/ScanUserRef".to_string(),
                r#"{"arr":[{"ts":8,"obj":{"scanid":1,"zone_id":4,"barcode":"ABC","time":8}}]}"#
                    .to_string(),
            );
            let stand = Stand::start(bodies).await;
            stand.release(40);
            let (venue, dir) = venue("EVT1");
            CurrentEvent::lock(&venue.app.current)
                .set_cursor("EVT1", "zone", "4")
                .unwrap();
            let remote = RemoteServer::new(&stand.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                catalog(&venue.app),
                20,
                Duration::from_secs(2),
            )
            .await;
            let hit = stand
                .hits()
                .into_iter()
                .find(|hit| hit.path == "/tracked/ScanUserRef")
                .expect("scan download");
            let form = form_map(&hit.body);
            assert_eq!(form.get("extraId").map(String::as_str), Some("zone_4"));
            assert_eq!(form.get("limit").map(String::as_str), Some("100"));
            assert_eq!(
                form.get("json").map(String::as_str),
                Some("EVT1_regscan_track_request_short")
            );
            assert_eq!(scan_rows(&dir), vec![(1, 4, "ABC".to_string())]);
        }

        #[tokio::test]
        async fn org_users_are_taken_before_the_upload() {
            let stand = Stand::start(HashMap::new()).await;
            let (venue, _dir) = venue("EVT1");
            CurrentEvent::lock(&venue.app.current)
                .set_cursor("EVT1", "org_id", "55")
                .unwrap();
            CurrentEvent::lock(&venue.app.current)
                .write("EVT1", &waiting("1"))
                .unwrap();
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, catalog(&app), 1, Duration::from_secs(2)).await
                })
            };
            let started = tokio::time::Instant::now();
            loop {
                let hits = stand.hits();
                if hits.iter().any(|hit| hit.path == "/tracked/OrgRegUser") {
                    break;
                }
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("org users were not sent, saw {} requests", hits.len());
                }
                if !hits.is_empty() {
                    assert!(hits.iter().all(|hit| hit.path != "/boxapi/uploadreg"));
                    stand.release(hits.len());
                }
                tokio::task::yield_now().await;
            }
            assert!(stand
                .hits()
                .iter()
                .all(|hit| hit.path != "/boxapi/uploadreg"));
            stand.release(stand.hits().len());
            until(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/uploadreg")
            })
            .await;
            let hits = stand.hits();
            let org = hits
                .iter()
                .position(|hit| hit.path == "/tracked/OrgRegUser")
                .unwrap();
            let upload = hits
                .iter()
                .position(|hit| hit.path == "/boxapi/uploadreg")
                .unwrap();
            assert!(org < upload);
            let form = form_map(&hits[org].body);
            assert_eq!(form.get("extraId").map(String::as_str), Some("55"));
            assert_eq!(form.get("limit").map(String::as_str), Some("100"));
            stand.release(40);
            pending.await.unwrap();
        }
    }

    mod upload_support {
        pub(super) use super::rest::{form_map, until, Stand};
        use crate::registration_server::event_db::DB_FILE;
        use std::path::Path;

        fn db(dir: &Path) -> rusqlite::Connection {
            rusqlite::Connection::open(dir.join("EVT1").join(DB_FILE)).unwrap()
        }

        pub(super) fn insert_registration(
            dir: &Path,
            uid: &str,
            barcode: &str,
            on_server: Option<i64>,
            in_synch: Option<i64>,
        ) {
            db(dir)
                .execute(
                    "INSERT INTO mem_u (uid, data, ts, barcode, on_server, in_synch)
                     VALUES (?1, '{}', 1, ?2, ?3, ?4)",
                    rusqlite::params![uid, barcode, on_server, in_synch],
                )
                .unwrap();
        }

        pub(super) fn insert_scan(dir: &Path, scanid: i64, barcode: &str, synched: Option<i64>) {
            db(dir)
                .execute(
                    "INSERT INTO scans (scanid, zone_id, entrence_type, barcode, time, synched)
                     VALUES (?1, 1, 0, ?2, 1, ?3)",
                    rusqlite::params![scanid, barcode, synched],
                )
                .unwrap();
        }

        pub(super) fn insert_attachment(dir: &Path, man_id: i64, uid: i64, synched: Option<i64>) {
            db(dir)
                .execute(
                    "INSERT INTO man_to_user (man_id, uid, comment, zone, time, synched)
                     VALUES (?1, ?2, '', 1, 1, ?3)",
                    rusqlite::params![man_id, uid, synched],
                )
                .unwrap();
        }

        pub(super) fn in_synch(dir: &Path, uid: &str) -> Option<i64> {
            db(dir)
                .query_row("SELECT in_synch FROM mem_u WHERE uid = ?1", [uid], |row| {
                    row.get(0)
                })
                .unwrap()
        }

        pub(super) fn scan_flag(dir: &Path, scanid: i64) -> Option<i64> {
            db(dir)
                .query_row(
                    "SELECT synched FROM scans WHERE scanid = ?1",
                    [scanid],
                    |row| row.get(0),
                )
                .unwrap()
        }

        pub(super) fn vals_of(body: &str) -> Vec<serde_json::Value> {
            let form = form_map(body);
            let raw = form.get("vals").expect("vals");
            serde_json::from_str(raw).expect("vals json")
        }

        pub(super) fn field_of(body: &str, name: &str) -> Option<String> {
            form_map(body).get(name).cloned()
        }
    }

    mod upload_reg {
        use super::support::venue;
        use super::upload_support::{in_synch, insert_registration, until, vals_of, Stand};
        use super::*;
        use crate::registration_server::remote_server::RemoteServer;
        use std::collections::HashMap;

        fn synch_res(ids: &[i64]) -> String {
            let listed: Vec<_> = ids
                .iter()
                .map(|id| serde_json::json!({ "id": id.to_string() }))
                .collect();
            serde_json::json!({ "synch_res": listed }).to_string()
        }

        async fn past_downloads(stand: &Stand, pred: impl Fn(&[super::rest::Hit]) -> bool) {
            let started = tokio::time::Instant::now();
            loop {
                let hits = stand.hits();
                if pred(&hits) {
                    return;
                }
                if !hits.is_empty() && hits.iter().all(|hit| !hit.path.contains("upload")) {
                    stand.release(hits.len());
                }
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("timed out after {} requests", hits.len());
                }
                tokio::task::yield_now().await;
            }
        }

        #[tokio::test]
        async fn no_waiting_row_skips_uploadreg() {
            let stand = Stand::start(HashMap::new()).await;
            stand.release(40);
            let (venue, dir) = venue("EVT1");
            insert_registration(&dir, "1", "", Some(1), Some(1));
            let remote = RemoteServer::new(&stand.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                catalog(&venue.app),
                4,
                Duration::from_secs(2),
            )
            .await;
            assert!(stand
                .hits()
                .iter()
                .all(|hit| hit.path != "/boxapi/uploadreg"));
        }

        #[tokio::test]
        async fn twenty_waiting_rows_are_one_batch() {
            let mut bodies = HashMap::new();
            bodies.insert(
                "/boxapi/uploadreg".to_string(),
                synch_res(&(1..=21).collect::<Vec<_>>()),
            );
            let stand = Stand::start(bodies).await;
            let (venue, dir) = venue("EVT1");
            for id in 1..=21 {
                insert_registration(&dir, &id.to_string(), "", None, None);
            }
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, catalog(&app), 1, Duration::from_secs(2)).await
                })
            };
            past_downloads(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/uploadreg")
            })
            .await;
            let hits = stand.hits();
            let first = hits
                .iter()
                .position(|hit| hit.path == "/boxapi/uploadreg")
                .unwrap();
            assert_eq!(
                hits.iter()
                    .filter(|hit| hit.path == "/boxapi/uploadreg")
                    .count(),
                1
            );
            let vals = vals_of(&hits[first].body);
            assert_eq!(vals.len(), 20);
            let ids: Vec<_> = vals
                .iter()
                .filter_map(|row| row.get("id").and_then(|id| id.as_str()))
                .collect();
            assert_eq!(ids, (1..=20).map(|id| id.to_string()).collect::<Vec<_>>());
            assert!(in_synch(&dir, "1").is_none());
            stand.release(first + 1);
            until(&stand, |hits| {
                hits.iter()
                    .filter(|hit| hit.path == "/boxapi/uploadreg")
                    .count()
                    >= 2
            })
            .await;
            for id in 1..=20 {
                assert_eq!(in_synch(&dir, &id.to_string()), Some(1));
            }
            assert!(in_synch(&dir, "21").is_none());
            let hits = stand.hits();
            let second = hits
                .iter()
                .rposition(|hit| hit.path == "/boxapi/uploadreg")
                .unwrap();
            let vals = vals_of(&hits[second].body);
            assert_eq!(vals.len(), 1);
            assert_eq!(vals[0].get("id").and_then(|id| id.as_str()), Some("21"));
            stand.release(40);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn photos_follow_the_accepted_ids() {
            let mut bodies = HashMap::new();
            bodies.insert("/boxapi/uploadreg".to_string(), synch_res(&[1, 2, 3]));
            let stand = Stand::start(bodies).await;
            let (venue, dir) = venue("EVT1");
            for id in 1..=3 {
                insert_registration(&dir, &id.to_string(), "", None, None);
            }
            let photos = dir.join("EVT1").join("photos").join("uids");
            std::fs::create_dir_all(&photos).unwrap();
            std::fs::write(photos.join("1.png"), b"png-1").unwrap();
            std::fs::write(photos.join("2.png"), b"png-2").unwrap();
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, catalog(&app), 2, Duration::from_secs(2)).await
                })
            };
            past_downloads(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/uploadreg")
            })
            .await;
            let ticket = stand.hits().len();
            stand.release(ticket);
            until(&stand, |hits| {
                hits.iter()
                    .filter(|hit| hit.path == "/boxapi/uploadphoto")
                    .count()
                    >= 2
            })
            .await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let photos: Vec<_> = stand
                .hits()
                .into_iter()
                .filter(|hit| hit.path == "/boxapi/uploadphoto")
                .collect();
            assert_eq!(photos.len(), 2);
            assert!(photos.iter().any(|hit| hit.body.contains("name=\"f_1\"")));
            assert!(photos.iter().any(|hit| hit.body.contains("name=\"f_2\"")));
            assert!(photos.iter().all(|hit| !hit.body.contains("name=\"f_3\"")));
            stand.release(40);
            pending.await.unwrap();
        }
    }

    mod upload_rest {
        use super::support::venue;
        use super::upload_support::{
            field_of, in_synch, insert_attachment, insert_registration, insert_scan, scan_flag,
            until, vals_of, Stand,
        };
        use super::*;
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::remote_server::RemoteServer;
        use std::collections::HashMap;

        async fn past_downloads(stand: &Stand, pred: impl Fn(&[super::rest::Hit]) -> bool) {
            let started = tokio::time::Instant::now();
            loop {
                let hits = stand.hits();
                if pred(&hits) {
                    return;
                }
                if !hits.is_empty() && hits.iter().all(|hit| !hit.path.contains("upload")) {
                    stand.release(hits.len());
                }
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("timed out after {} requests", hits.len());
                }
                tokio::task::yield_now().await;
            }
        }

        #[tokio::test]
        async fn scans_go_up_in_batches_of_100() {
            let mut bodies = HashMap::new();
            bodies.insert(
                "/boxapi/uploadscans".to_string(),
                r#"{"status":"ok"}"#.to_string(),
            );
            let stand = Stand::start(bodies).await;
            let (venue, dir) = venue("EVT1");
            for id in 1..=101 {
                insert_scan(&dir, id, &format!("B{id}"), None);
            }
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, catalog(&app), 1, Duration::from_secs(2)).await
                })
            };
            past_downloads(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/uploadscans")
            })
            .await;
            let hits = stand.hits();
            let first = hits
                .iter()
                .position(|hit| hit.path == "/boxapi/uploadscans")
                .unwrap();
            let vals = vals_of(&hits[first].body);
            assert_eq!(vals.len(), 100);
            stand.release(first + 1);
            until(&stand, |hits| {
                hits.iter()
                    .filter(|hit| hit.path == "/boxapi/uploadscans")
                    .count()
                    >= 2
            })
            .await;
            for id in 1..=100 {
                assert_eq!(scan_flag(&dir, id), Some(1));
            }
            assert!(scan_flag(&dir, 101).is_none());
            let hits = stand.hits();
            let second = hits
                .iter()
                .rposition(|hit| hit.path == "/boxapi/uploadscans")
                .unwrap();
            let vals = vals_of(&hits[second].body);
            assert_eq!(vals.len(), 1);
            assert_eq!(vals[0].get("scan_id").and_then(|id| id.as_i64()), Some(101));
            stand.release(40);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn no_scans_skips_the_call() {
            let stand = Stand::start(HashMap::new()).await;
            stand.release(40);
            let (venue, dir) = venue("EVT1");
            insert_scan(&dir, 1, "DONE", Some(1));
            let remote = RemoteServer::new(&stand.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                catalog(&venue.app),
                4,
                Duration::from_secs(2),
            )
            .await;
            assert!(stand
                .hits()
                .iter()
                .all(|hit| hit.path != "/boxapi/uploadscans"));
        }

        #[tokio::test]
        async fn a_new_registration_holds_only_its_own_rows() {
            let mut bodies = HashMap::new();
            bodies.insert(
                "/boxapi/uploadreg".to_string(),
                r#"{"synch_res":[{"id":"1"}]}"#.to_string(),
            );
            bodies.insert(
                "/boxapi/uploadscans".to_string(),
                r#"{"status":"ok"}"#.to_string(),
            );
            bodies.insert(
                "/leadgenapi/upload".to_string(),
                r#"{"users":[]}"#.to_string(),
            );
            let stand = Stand::start(bodies).await;
            let (venue, dir) = venue("EVT1");
            insert_registration(&dir, "1", "NEW", None, None);
            insert_registration(&dir, "2", "DOWN", Some(1), None);
            insert_scan(&dir, 10, "NEW", None);
            insert_scan(&dir, 11, "DOWN", None);
            insert_attachment(&dir, 7, 1, None);
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, catalog(&app), 2, Duration::from_secs(2)).await
                })
            };
            past_downloads(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/uploadreg")
                    && hits.iter().any(|hit| hit.path == "/boxapi/uploadscans")
            })
            .await;
            let hits = stand.hits();
            let reg = hits
                .iter()
                .position(|hit| hit.path == "/boxapi/uploadreg")
                .unwrap();
            let scan = hits
                .iter()
                .position(|hit| hit.path == "/boxapi/uploadscans")
                .unwrap();
            assert!(reg < scan);
            let scans = vals_of(&hits[scan].body);
            assert!(scans
                .iter()
                .any(|row| row.get("barcode").and_then(|v| v.as_str()) == Some("DOWN")));
            assert!(scans
                .iter()
                .all(|row| row.get("barcode").and_then(|v| v.as_str()) != Some("NEW")));
            assert!(hits.iter().all(|hit| hit.path != "/leadgenapi/upload"));
            stand.release(reg + 1);
            until(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/leadgenapi/upload")
            })
            .await;
            assert_eq!(in_synch(&dir, "1"), Some(1));
            let held = stand
                .hits()
                .into_iter()
                .find(|hit| hit.path == "/boxapi/uploadscans")
                .unwrap();
            let scans = vals_of(&held.body);
            assert!(scans
                .iter()
                .all(|row| row.get("barcode").and_then(|v| v.as_str()) != Some("NEW")));
            let lead = stand
                .hits()
                .into_iter()
                .find(|hit| hit.path == "/leadgenapi/upload")
                .unwrap();
            let attached = vals_of(&lead.body);
            assert!(attached
                .iter()
                .any(|row| row.get("uid").and_then(|v| v.as_i64()) == Some(1)));
            stand.release(scan + 1);
            until(&stand, |hits| {
                hits.iter().any(|hit| {
                    hit.path == "/boxapi/uploadscans"
                        && vals_of(&hit.body)
                            .iter()
                            .any(|row| row.get("barcode").and_then(|v| v.as_str()) == Some("NEW"))
                })
            })
            .await;
            stand.release(40);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn attachments_send_the_event_id() {
            let mut bodies = HashMap::new();
            bodies.insert(
                "/boxapi/uploadscans".to_string(),
                r#"{"status":"ok"}"#.to_string(),
            );
            bodies.insert(
                "/leadgenapi/upload".to_string(),
                r#"{"users":[]}"#.to_string(),
            );
            let stand = Stand::start(bodies).await;
            let (venue, dir) = venue("EVT1");
            insert_scan(&dir, 1, "ORPHAN", None);
            insert_attachment(&dir, 3, 9, None);
            let remote = RemoteServer::new(&stand.base).unwrap();
            let pending = {
                let app = venue.app.clone();
                tokio::spawn(async move {
                    drain_queue(&app, &remote, catalog(&app), 2, Duration::from_secs(2)).await
                })
            };
            past_downloads(&stand, |hits| {
                hits.iter().any(|hit| hit.path == "/boxapi/uploadscans")
                    && hits.iter().any(|hit| hit.path == "/leadgenapi/upload")
            })
            .await;
            let lead = stand
                .hits()
                .into_iter()
                .find(|hit| hit.path == "/leadgenapi/upload")
                .unwrap();
            assert_eq!(
                field_of(&lead.body, "no_need_data").as_deref(),
                Some("true")
            );
            assert_eq!(field_of(&lead.body, "expoId").as_deref(), Some("EVT1"));
            stand.release(40);
            pending.await.unwrap();
        }

        #[tokio::test]
        async fn a_rejected_upload_leaves_the_rest_unsent() {
            let mut bodies = HashMap::new();
            bodies.insert(
                "/boxapi/uploadscans".to_string(),
                r#"{"error_msg":"not_logged_in"}"#.to_string(),
            );
            let stand = Stand::start(bodies).await;
            stand.release(40);
            let (venue, dir) = venue("EVT1");
            insert_scan(&dir, 1, "ORPHAN", None);
            insert_attachment(&dir, 3, 9, None);
            let remote = RemoteServer::new(&stand.base).unwrap();
            drain_queue(
                &venue.app,
                &remote,
                catalog(&venue.app),
                1,
                Duration::from_secs(2),
            )
            .await;
            assert!(stand
                .hits()
                .iter()
                .all(|hit| hit.path != "/leadgenapi/upload"));
            assert!(scan_flag(&dir, 1).is_none());
            assert!(CurrentEvent::lock(&venue.app.current).event_id().is_none());
            let stored = venue.app.operator.credential_file().load().unwrap();
            assert_eq!(stored.project_token(), None);
            assert_eq!(stored.event_id(), None);
        }
    }
}
