//! The release tag compiled into this binary, and the GitHub check for a newer one.
//!
//! `REGISTRATION_RELEASE_TAG` is read by `build.rs`. Unset bakes `v` plus the
//! Cargo version. A set value must be that same tag.
//!
//! The sync thread asks GitHub `releases/latest` once the listen socket is open
//! and again every six hours. That request is not a sync download and it does
//! not call the remote registration server.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::registration_server::gate::{Gate, RequestGuard};

#[cfg(test)]
#[path = "release_tag.rs"]
mod release_tag;

/// Tag baked at build time, `v` plus `MAJOR.MINOR.PATCH`.
pub fn release_tag() -> &'static str {
    env!("REGISTRATION_RELEASE_TAG")
}

/// Version from [`release_tag`], without the leading `v`. Version directories and asset names use this.
pub fn release_version() -> &'static str {
    release_tag()
        .strip_prefix('v')
        .expect("release tag starts with v")
}

/// One line for the `RELEASE` file in the package payload.
pub fn release_file_contents() -> String {
    format!("{}\n", release_tag())
}

/// GitHub repository the release check reads. `REGISTRATION_UPDATE_REPO` replaces it.
pub const UPDATE_REPO: &str = "OpenDOps/pdf-badges";
pub const UPDATE_REPO_ENV: &str = "REGISTRATION_UPDATE_REPO";
/// Set to `1` to offer pre-releases. Unset keeps `releases/latest`, which omits them.
pub const UPDATE_PRERELEASE_ENV: &str = "REGISTRATION_UPDATE_PRERELEASE";
pub const GITHUB_API: &str = "https://api.github.com";

/// How long the sync thread waits after a release check before the next one.
pub const UPDATE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// `tag` is `v` or `release-` plus a version. `running` is that version without the prefix.
/// A version is `MAJOR.MINOR.PATCH` or the same numbers plus `-alpha.N`, `-beta.N`, or `-rc.N`.
/// The result is the version without the prefix when it is newer in semver order.
pub fn newer(tag: &str, running: &str) -> Option<String> {
    let tag_version = parse_version(version_in_tag(tag)?)?;
    let running_version = parse_version(running)?;
    if tag_version > running_version {
        Some(tag_version.to_string())
    } else {
        None
    }
}

fn version_in_tag(tag: &str) -> Option<&str> {
    tag.strip_prefix("release-")
        .or_else(|| tag.strip_prefix('v'))
}

/// Empty or whitespace keeps [`UPDATE_REPO`].
pub fn update_repo(raw: Option<&str>) -> String {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(repo) => repo.to_string(),
        None => UPDATE_REPO.to_string(),
    }
}

pub fn update_repo_from_env() -> String {
    update_repo(std::env::var(UPDATE_REPO_ENV).ok().as_deref())
}

/// `1`, `true`, or `yes` lists GitHub releases and keeps pre-releases. Anything else does not.
pub fn update_prerelease(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim)
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

pub fn update_prerelease_from_env() -> bool {
    update_prerelease(std::env::var(UPDATE_PRERELEASE_ENV).ok().as_deref())
}

/// No previous attempt is due. An attempt is due again `UPDATE_EVERY` later.
pub fn check_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    match last {
        None => true,
        Some(last) => match now.duration_since(last) {
            Ok(elapsed) => elapsed >= UPDATE_EVERY,
            Err(_) => false,
        },
    }
}

/// `rejected` from `updates/state.yml`. A missing or unreadable file is an empty list.
pub fn read_rejected(path: &Path) -> Vec<String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };
    rejected_in(&text)
}

fn rejected_in(text: &str) -> Vec<String> {
    #[derive(serde::Deserialize, Default)]
    struct StateFile {
        #[serde(default)]
        rejected: Vec<String>,
    }
    serde_yaml::from_str::<StateFile>(text)
        .map(|state| state.rejected)
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PreChannel {
    Alpha,
    Beta,
    Rc,
}

struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<(PreChannel, u64)>,
}

impl std::fmt::Display for ReleaseVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match self.pre {
            Some((PreChannel::Alpha, n)) => write!(f, "-alpha.{n}"),
            Some((PreChannel::Beta, n)) => write!(f, "-beta.{n}"),
            Some((PreChannel::Rc, n)) => write!(f, "-rc.{n}"),
            None => Ok(()),
        }
    }
}

impl PartialEq for ReleaseVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for ReleaseVersion {}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre, other.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(left), Some(right)) => left.cmp(&right),
            })
    }
}

fn parse_version(raw: &str) -> Option<ReleaseVersion> {
    if raw.is_empty() || raw.contains('+') {
        return None;
    }
    let (numbers, pre) = match raw.split_once('-') {
        Some((numbers, pre)) => (numbers, Some(pre)),
        None => (raw, None),
    };
    let mut parts = numbers.split('.');
    let major = numeric_identifier(parts.next()?)?;
    let minor = numeric_identifier(parts.next()?)?;
    let patch = numeric_identifier(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(ReleaseVersion {
        major,
        minor,
        patch,
        pre: match pre {
            Some(pre) => Some(parse_pre(pre)?),
            None => None,
        },
    })
}

/// A semver numeric identifier: `0`, or a number that does not start with `0`.
fn numeric_identifier(raw: &str) -> Option<u64> {
    if raw.is_empty() || (raw.len() > 1 && raw.starts_with('0')) {
        return None;
    }
    raw.parse().ok()
}

fn parse_pre(raw: &str) -> Option<(PreChannel, u64)> {
    let (name, number) = raw.split_once('.')?;
    if number.contains('.') {
        return None;
    }
    let channel = match name {
        "alpha" => PreChannel::Alpha,
        "beta" => PreChannel::Beta,
        "rc" => PreChannel::Rc,
        _ => return None,
    };
    Some((channel, numeric_identifier(number)?))
}

pub(crate) fn github_client(timeout: Duration) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        "application/vnd.github+json"
            .parse()
            .expect("github accept header"),
    );
    reqwest::Client::builder()
        .user_agent("rust-reg")
        .default_headers(headers)
        .timeout(timeout)
        .build()
        .expect("github client")
}

/// The offer from the last GitHub release check, and when that check ran.
pub struct ReleaseCheck {
    inner: Mutex<OfferState>,
}

struct OfferState {
    offer: Option<String>,
    last_attempt: Option<SystemTime>,
    api_base: String,
    in_flight: bool,
    include_prerelease: bool,
    phase: String,
    received: u64,
    total: u64,
}

/// The GitHub call failed, or another check already holds it.
pub(crate) enum CheckError {
    Failed,
    InProgress,
}

struct InFlight<'a>(&'a ReleaseCheck);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.inner.lock().expect("release offer").in_flight = false;
    }
}

impl ReleaseCheck {
    pub fn new() -> Self {
        Self::with_prerelease(update_prerelease_from_env())
    }

    fn with_prerelease(include_prerelease: bool) -> Self {
        Self {
            inner: Mutex::new(OfferState {
                offer: None,
                last_attempt: None,
                api_base: GITHUB_API.to_string(),
                in_flight: false,
                include_prerelease,
                phase: "idle".to_string(),
                received: 0,
                total: 0,
            }),
        }
    }

    /// `true` when this call is the one that moved the install into `downloading`.
    pub(crate) fn begin_download(&self) -> bool {
        let mut state = self.inner.lock().expect("release offer");
        if state.phase != "idle" {
            return false;
        }
        state.phase = "downloading".to_string();
        state.received = 0;
        state.total = 0;
        true
    }

    pub(crate) fn note_download(&self, received: u64, total: u64) {
        let mut state = self.inner.lock().expect("release offer");
        if state.phase != "downloading" {
            return;
        }
        state.received = received;
        if total > 0 {
            state.total = total;
        }
    }

    /// Moves `downloading` to `extracting` and resets the bar to 0 of 1000.
    pub(crate) fn begin_extract(&self) {
        let mut state = self.inner.lock().expect("release offer");
        if state.phase != "downloading" {
            return;
        }
        state.phase = "extracting".to_string();
        state.received = 0;
        state.total = EXTRACT_UNITS + UNPACK_UNITS;
    }

    pub(crate) fn note_extract(&self, received: u64, total: u64) {
        let mut state = self.inner.lock().expect("release offer");
        if state.phase != "extracting" {
            return;
        }
        state.received = received;
        if total > 0 {
            state.total = total;
        }
    }

    pub(crate) fn begin_apply(&self) {
        let mut state = self.inner.lock().expect("release offer");
        if state.phase == "downloading" || state.phase == "extracting" {
            state.phase = "applying".to_string();
            state.received = 0;
            state.total = 0;
        }
    }

    pub(crate) fn finish_install(&self) {
        let mut state = self.inner.lock().expect("release offer");
        state.phase = "idle".to_string();
        state.received = 0;
        state.total = 0;
    }

    pub(crate) fn progress(&self) -> (String, u64, u64) {
        let state = self.inner.lock().expect("release offer");
        (state.phase.clone(), state.received, state.total)
    }

    /// Stored offer. `GET /api/update` returns it.
    pub fn offer(&self) -> Option<String> {
        self.inner.lock().expect("release offer").offer.clone()
    }

    pub fn last_attempt(&self) -> Option<SystemTime> {
        self.inner.lock().expect("release offer").last_attempt
    }

    pub fn due(&self, now: SystemTime) -> bool {
        check_due(self.last_attempt(), now)
    }

    fn api_base(&self) -> String {
        self.inner.lock().expect("release offer").api_base.clone()
    }

    #[cfg(test)]
    fn set_api_base(&self, base: &str) {
        self.inner.lock().expect("release offer").api_base = base.to_string();
    }

    #[cfg(test)]
    fn store_offer(&self, offer: Option<String>) {
        self.inner.lock().expect("release offer").offer = offer;
    }

    #[cfg(test)]
    fn note_attempt(&self, now: SystemTime) {
        self.inner.lock().expect("release offer").last_attempt = Some(now);
    }

    /// Asks GitHub when [`due`] is true. Waits until the gate is idle. Does not touch the sync queue.
    /// `true` means a check is already in flight and this call did not send.
    pub async fn scheduled(
        &self,
        gate: &Gate,
        client: &reqwest::Client,
        api_base: &str,
        repo: &str,
        running: &str,
        rejected: &[String],
        now: SystemTime,
    ) -> bool {
        if !self.due(now) {
            return false;
        }
        matches!(
            self.force(gate, client, api_base, repo, running, rejected, now)
                .await,
            Err(CheckError::InProgress)
        )
    }

    /// Asks GitHub even when the six-hour wait has not elapsed. The attempt time becomes `now`.
    /// [`CheckError::Failed`] leaves the previous offer. [`CheckError::InProgress`] does not send.
    pub async fn force(
        &self,
        gate: &Gate,
        client: &reqwest::Client,
        api_base: &str,
        repo: &str,
        running: &str,
        rejected: &[String],
        now: SystemTime,
    ) -> Result<Option<String>, CheckError> {
        self.force_at(gate, client, api_base, repo, running, rejected, now, false)
            .await
    }

    /// Same as [`force`]. `from_request` is the desk route: that call already holds the gate,
    /// so it waits until every other request and any print are idle.
    async fn force_at(
        &self,
        gate: &Gate,
        client: &reqwest::Client,
        api_base: &str,
        repo: &str,
        running: &str,
        rejected: &[String],
        now: SystemTime,
        from_request: bool,
    ) -> Result<Option<String>, CheckError> {
        let include_prerelease = {
            let mut state = self.inner.lock().expect("release offer");
            if state.in_flight {
                return Err(CheckError::InProgress);
            }
            state.in_flight = true;
            state.include_prerelease
        };
        let _flight = InFlight(self);
        if from_request {
            gate.wait_until_others_idle().await;
        } else {
            gate.wait_until_idle().await;
        }
        let fetched = fetch_offer(
            client,
            api_base,
            repo,
            running,
            rejected,
            include_prerelease,
        )
        .await;
        let mut state = self.inner.lock().expect("release offer");
        state.last_attempt = Some(now);
        match fetched {
            Ok(offer) => {
                state.offer = offer.clone();
                Ok(offer)
            }
            Err(()) => Err(CheckError::Failed),
        }
    }
}

/// `{ "version", "newer", "phase", "received", "total" }` for `GET /api/update`. Does not open a connection.
pub(crate) fn update_document(app: &crate::registration_server::App) -> serde_json::Value {
    let (phase, received, total) = app.updates.progress();
    let allowed = update_allowed(app);
    let failed = failed_upgrade(&app.prefix);
    serde_json::json!({
        "version": release_version(),
        "newer": app.updates.offer(),
        "supervised": allowed,
        "phase": phase,
        "received": received,
        "total": if total == 0 { serde_json::Value::Null } else { serde_json::json!(total) },
        "failed": failed,
    })
}

/// The version named by phase `rolled_back` in `<prefix>/updates/state.yml`.
fn failed_upgrade(prefix: &Path) -> Option<String> {
    let state = read_state(prefix)?;
    if state.phase == "rolled_back" && !state.to.is_empty() {
        Some(state.to)
    } else {
        None
    }
}

/// The step 3 check, run now. `Err` leaves the stored offer as [`ReleaseCheck::force`] left it.
pub(crate) async fn check_now(
    app: &crate::registration_server::App,
) -> Result<serde_json::Value, CheckError> {
    let client = github_client(Duration::from_secs(30));
    let rejected = read_rejected(Path::new("updates/state.yml"));
    let base = app.updates.api_base();
    app.updates
        .force_at(
            &app.gate,
            &client,
            &base,
            &update_repo_from_env(),
            release_version(),
            &rejected,
            SystemTime::now(),
            true,
        )
        .await?;
    Ok(update_document(app))
}

impl Default for ReleaseCheck {
    fn default() -> Self {
        Self::new()
    }
}

/// `SHA256SUMS` did not name this file with the digest of its bytes.
/// The download in step 7 is the first caller outside tests.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChecksumMismatch;

#[allow(dead_code)]
impl ChecksumMismatch {
    pub(crate) fn code(self) -> &'static str {
        "checksum_mismatch"
    }
}

/// `sums` is `SHA256SUMS`: lowercase hex, two spaces, the file name, one line per file.
/// Another file's line is ignored. One space before `name`, or no line for `name`, is a mismatch.
#[allow(dead_code)]
pub(crate) fn verify(bytes: &[u8], sums: &str, name: &str) -> Result<(), ChecksumMismatch> {
    let digest = sha256_hex(bytes);
    let mut found = false;
    for raw in sums.split('\n') {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        match line.split_once("  ") {
            Some((hex, file)) if file == name => {
                found = true;
                if hex != digest {
                    return Err(ChecksumMismatch);
                }
            }
            Some(_) => {}
            None if line.split_once(' ').is_some_and(|(_, file)| file == name) => {
                return Err(ChecksumMismatch);
            }
            None => {}
        }
    }
    if found {
        Ok(())
    } else {
        Err(ChecksumMismatch)
    }
}

/// Checks `asset` with [`verify`]. A mismatch deletes `asset` and does not write `updates/live`.
#[allow(dead_code)]
pub(crate) fn verify_asset(asset: &Path, sums: &str, name: &str) -> Result<(), ChecksumMismatch> {
    let bytes = std::fs::read(asset).map_err(|_| ChecksumMismatch)?;
    if let Err(err) = verify(&bytes, sums, name) {
        let _ = std::fs::remove_file(asset);
        return Err(err);
    }
    Ok(())
}

#[allow(dead_code)]
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Upgrade archive step 12 packs for this operating system. `version` is `MAJOR.MINOR.PATCH`.
/// On macOS the name follows the processor: Apple silicon uses `macos`, Intel uses `macos-intel`.
/// The pkg, the deb, and the exe stay the first-install packages.
#[allow(dead_code)]
pub(crate) fn asset_name(version: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        format!("rust-reg-{version}-linux.zip")
    }
    #[cfg(target_os = "macos")]
    {
        macos_upgrade_archive(version, host_is_apple_silicon())
    }
    #[cfg(target_os = "windows")]
    {
        format!("rust-reg-{version}-windows.zip")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        format!("rust-reg-{version}.zip")
    }
}

/// `rust-reg-<version>-macos.zip` on Apple silicon, `rust-reg-<version>-macos-intel.zip` on Intel.
pub(crate) fn macos_upgrade_archive(version: &str, apple_silicon: bool) -> String {
    if apple_silicon {
        format!("rust-reg-{version}-macos.zip")
    } else {
        format!("rust-reg-{version}-macos-intel.zip")
    }
}

/// The machine's processor. An Intel binary running under Rosetta on Apple silicon reports Apple silicon.
#[cfg(target_os = "macos")]
fn host_is_apple_silicon() -> bool {
    let mut value: i32 = 0;
    let mut len = std::mem::size_of_val(&value);
    let name = std::ffi::CString::new("hw.optional.arm64").expect("sysctl name");
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut i32 as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    rc == 0 && value != 0
}

/// `rust-reg` inside a version tree, or `rust-reg.exe` on Windows.
#[allow(dead_code)]
pub(crate) fn version_binary_name() -> &'static str {
    if cfg!(windows) {
        "rust-reg.exe"
    } else {
        "rust-reg"
    }
}

/// `rust-reg-run` inside a version tree, or `rust-reg-run.exe` on Windows.
pub(crate) fn wrapper_binary_name() -> &'static str {
    if cfg!(windows) {
        "rust-reg-run.exe"
    } else {
        "rust-reg-run"
    }
}

/// The direct parent executable is `rust-reg-run`. A renamed copy, a shell, and a missing parent are not.
pub(crate) fn parent_is_supervisor() -> bool {
    parent_executable().is_some_and(|path| is_supervisor_path(&path))
}

fn is_supervisor_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "rust-reg-run" || name == "rust-reg-run.exe"
}

/// `true` when this process may stage an update. Tests opt in. Production asks the live parent.
pub(crate) fn update_allowed(app: &crate::registration_server::App) -> bool {
    #[cfg(test)]
    if app.supervised {
        return true;
    }
    #[cfg(not(test))]
    let _ = app;
    parent_is_supervisor()
}

#[cfg(target_os = "linux")]
fn parent_executable() -> Option<PathBuf> {
    let pid = unsafe { libc::getppid() };
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[cfg(target_os = "macos")]
fn parent_executable() -> Option<PathBuf> {
    extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut libc::c_void, buffersize: u32) -> i32;
    }
    let mut buf = vec![0u8; 4096];
    let written = unsafe {
        proc_pidpath(
            libc::getppid(),
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as u32,
        )
    };
    if written <= 0 {
        return None;
    }
    let end = written as usize;
    let bytes = &buf[..end.min(buf.len())];
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let text = std::str::from_utf8(&bytes[..end]).ok()?;
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

#[cfg(windows)]
fn parent_executable() -> Option<PathBuf> {
    #[repr(C)]
    struct ProcessBasicInformation {
        reserved1: *mut std::ffi::c_void,
        peb: *mut std::ffi::c_void,
        reserved2: [*mut std::ffi::c_void; 2],
        pid: usize,
        parent_pid: usize,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            process: isize,
            class: u32,
            info: *mut ProcessBasicInformation,
            len: u32,
            returned: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn QueryFullProcessImageNameW(
            process: isize,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
    }
    unsafe {
        let mut info = std::mem::zeroed::<ProcessBasicInformation>();
        let status = NtQueryInformationProcess(
            GetCurrentProcess(),
            0,
            &mut info,
            std::mem::size_of::<ProcessBasicInformation>() as u32,
            std::ptr::null_mut(),
        );
        if status != 0 || info.parent_pid == 0 {
            return None;
        }
        let handle = OpenProcess(0x1000, 0, info.parent_pid as u32);
        if handle == 0 {
            return None;
        }
        let mut wide = vec![0u16; 32768];
        let mut len = wide.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, wide.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        wide.truncate(len as usize);
        use std::os::windows::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_wide(&wide)))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn parent_executable() -> Option<PathBuf> {
    None
}

/// Temporary asset, next to `versions/`, not inside it.
#[allow(dead_code)]
pub(crate) fn asset_temp_path(prefix: &Path, version: &str) -> PathBuf {
    prefix.join(format!("{}.partial", asset_name(version)))
}

/// The checked asset and, when this call dropped a request guard, the guard taken again.
#[allow(dead_code)]
pub(crate) struct Downloaded {
    pub path: PathBuf,
    /// Taken again after the download. Step 9 keeps it for the rest of the request.
    #[allow(dead_code)]
    pub held: Option<RequestGuard>,
}

#[allow(dead_code)]
pub(crate) struct DownloadFailed {
    kind: DownloadKind,
    /// Taken again after the download. Step 9 keeps it for the rest of the request.
    #[allow(dead_code)]
    pub held: Option<RequestGuard>,
}

#[allow(dead_code)]
enum DownloadKind {
    ChecksumMismatch,
    Failed,
}

impl DownloadFailed {
    #[allow(dead_code)]
    pub(crate) fn code(&self) -> &'static str {
        match self.kind {
            DownloadKind::ChecksumMismatch => ChecksumMismatch.code(),
            DownloadKind::Failed => "download_failed",
        }
    }
}

/// Downloads this machine's asset and `SHA256SUMS` from the same release.
/// Drops `held` before waiting, and takes a request guard again when the download returns.
/// Does not create a directory under `versions/` and does not exit.
#[allow(dead_code)]
pub(crate) async fn download_release(
    prefix: &Path,
    gate: &Arc<Gate>,
    held: Option<RequestGuard>,
    client: &reqwest::Client,
    base: &str,
    repo: &str,
    version: &str,
    progress: Option<&ReleaseCheck>,
) -> Result<Downloaded, DownloadFailed> {
    let resume = held.is_some();
    drop(held);
    gate.wait_until_idle().await;
    let fetched = fetch_asset(prefix, client, base, repo, version, progress).await;
    let held = resume.then(|| gate.enter_request());
    match fetched {
        Ok(path) => Ok(Downloaded { path, held }),
        Err(kind) => Err(DownloadFailed { kind, held }),
    }
}

#[allow(dead_code)]
async fn fetch_asset(
    prefix: &Path,
    client: &reqwest::Client,
    base: &str,
    repo: &str,
    version: &str,
    progress: Option<&ReleaseCheck>,
) -> Result<PathBuf, DownloadKind> {
    let name = asset_name(version);
    let path = asset_temp_path(prefix, version);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| DownloadKind::Failed)?;
    }
    let asset_url = release_file_url(base, repo, version, &name);
    if write_download(client, &asset_url, &path, progress)
        .await
        .is_err()
    {
        let _ = std::fs::remove_file(&path);
        return Err(DownloadKind::Failed);
    }
    let sums_url = release_file_url(base, repo, version, "SHA256SUMS");
    let sums = match read_download(client, &sums_url).await {
        Ok(text) => text,
        Err(()) => {
            let _ = std::fs::remove_file(&path);
            return Err(DownloadKind::Failed);
        }
    };
    if verify_asset(&path, &sums, &name).is_err() {
        return Err(DownloadKind::ChecksumMismatch);
    }
    Ok(path)
}

#[allow(dead_code)]
fn release_file_url(base: &str, repo: &str, version: &str, name: &str) -> String {
    let root = base.trim_end_matches('/');
    // `api.github.com/repos/.../releases/download/...` is not a file. The asset is on github.com.
    if api_host_is_github(root) {
        format!("https://github.com/{repo}/releases/download/release-{version}/{name}")
    } else {
        format!("{root}/repos/{repo}/releases/download/release-{version}/{name}")
    }
}

fn api_host_is_github(base: &str) -> bool {
    reqwest::Url::parse(base)
        .ok()
        .and_then(|url| url.host_str().map(|host| host == "api.github.com"))
        .unwrap_or(false)
}

#[allow(dead_code)]
async fn write_download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: Option<&ReleaseCheck>,
) -> Result<(), ()> {
    let mut response = client.get(url).send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let total = response.content_length().unwrap_or(0);
    let mut received = 0u64;
    if let Some(progress) = progress {
        progress.note_download(0, total);
    }
    let mut file = std::fs::File::create(dest).map_err(|_| ())?;
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        file.write_all(&chunk).map_err(|_| ())?;
        received += chunk.len() as u64;
        if let Some(progress) = progress {
            progress.note_download(received, total);
        }
        tokio::task::yield_now().await;
    }
    file.sync_all().map_err(|_| ())
}

#[allow(dead_code)]
async fn read_download(client: &reqwest::Client, url: &str) -> Result<String, ()> {
    let mut response = client.get(url).send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        body.extend_from_slice(&chunk);
        tokio::task::yield_now().await;
    }
    String::from_utf8(body).map_err(|_| ())
}

/// Copies a prepared payload into `versions/<version>.partial`, flushes that directory once, then renames.
/// An existing `versions/<version>` is removed before the rename.
/// The payload is the directory `dpkg-deb`, `pkgutil`, or the Windows staging run produces.
/// A failure removes `.partial`. `data/` and `bin/rust-reg-run` are not touched.
#[allow(dead_code)]
pub(crate) fn unpack(prefix: &Path, version: &str, payload: &Path) -> Result<(), UnpackError> {
    unpack_with(prefix, version, payload, &mut |_| {})
}

fn unpack_with(
    prefix: &Path,
    version: &str,
    payload: &Path,
    note: &mut dyn FnMut(u64),
) -> Result<(), UnpackError> {
    let versions = prefix.join("versions");
    let partial = versions.join(format!("{version}.partial"));
    let published = versions.join(version);
    if let Err(err) = std::fs::create_dir_all(&partial) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(UnpackError::from(err));
    }
    let copied = copy_payload(payload, &partial, note);
    if let Err(err) = copied {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(err);
    }
    if let Err(err) = sync_dir(&partial) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(UnpackError::from(err));
    }
    // A previous install of this same version leaves the destination in place.
    // Rename onto that directory fails, so the old tree goes away first.
    if published.exists() {
        let remove = if published.is_dir() {
            std::fs::remove_dir_all(&published)
        } else {
            std::fs::remove_file(&published)
        };
        if let Err(err) = remove {
            let _ = std::fs::remove_dir_all(&partial);
            return Err(UnpackError::from(err));
        }
    }
    if let Err(err) = std::fs::rename(&partial, &published) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(UnpackError::from(err));
    }
    Ok(())
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct UnpackError;

impl UnpackError {
    #[allow(dead_code)]
    fn from(_err: std::io::Error) -> Self {
        Self
    }
}

#[allow(dead_code)]
fn copy_payload(
    payload: &Path,
    partial: &Path,
    note: &mut dyn FnMut(u64),
) -> Result<(), UnpackError> {
    let mut copied = 0u64;
    let binary = version_binary_name();
    let binary_path = partial.join(binary);
    copy_file(&payload.join(binary), &binary_path, &mut copied, note)?;
    mark_executable(&binary_path)?;
    copy_tree(
        &payload.join("admin"),
        &partial.join("admin"),
        &mut copied,
        note,
    )?;
    copy_tree(
        &payload.join("form"),
        &partial.join("form"),
        &mut copied,
        note,
    )?;
    copy_file(
        &payload.join("RELEASE"),
        &partial.join("RELEASE"),
        &mut copied,
        note,
    )?;
    let wrapper = wrapper_binary_name();
    let wrapper_src = payload.join(wrapper);
    if wrapper_src.is_file() {
        let wrapper_path = partial.join(wrapper);
        copy_file(&wrapper_src, &wrapper_path, &mut copied, note)?;
        mark_executable(&wrapper_path)?;
    }
    Ok(())
}

#[allow(dead_code)]
fn copy_tree(
    from: &Path,
    to: &Path,
    copied: &mut u64,
    note: &mut dyn FnMut(u64),
) -> Result<(), UnpackError> {
    std::fs::create_dir_all(to).map_err(UnpackError::from)?;
    let entries = std::fs::read_dir(from).map_err(UnpackError::from)?;
    for entry in entries {
        let entry = entry.map_err(UnpackError::from)?;
        let dest = to.join(entry.file_name());
        let kind = entry.file_type().map_err(UnpackError::from)?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest, copied, note)?;
        } else {
            copy_file(&entry.path(), &dest, copied, note)?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
/// The version binary is what the supervisor spawns. A zip entry is written as a normal file.
fn mark_executable(path: &Path) -> Result<(), UnpackError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .map_err(UnpackError::from)?
            .permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(path, perms).map_err(UnpackError::from)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn copy_file(
    from: &Path,
    to: &Path,
    copied: &mut u64,
    note: &mut dyn FnMut(u64),
) -> Result<(), UnpackError> {
    let bytes = std::fs::read(from).map_err(UnpackError::from)?;
    let mut file = std::fs::File::create(to).map_err(UnpackError::from)?;
    file.write_all(&bytes).map_err(UnpackError::from)?;
    *copied = copied.saturating_add(bytes.len() as u64);
    note(*copied);
    Ok(())
}

#[allow(dead_code)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// `/opt/rust-reg`, `/Library/rust-reg`, or `C:\Program Files\rust-reg`.
pub(crate) fn default_prefix() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/opt/rust-reg")
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/Library/rust-reg")
    }
    #[cfg(target_os = "windows")]
    {
        PathBuf::from(r"C:\Program Files\rust-reg")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        PathBuf::from("/opt/rust-reg")
    }
}

/// `<prefix>/data/credentials.yml` → `<prefix>`. Any other path is not an install.
pub(crate) fn install_prefix(credentials: &Path) -> Option<PathBuf> {
    let data = credentials.parent()?;
    if data.file_name().and_then(|name| name.to_str()) != Some("data") {
        return None;
    }
    let prefix = data.parent()?;
    if prefix.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(prefix.to_path_buf())
    }
}

/// Binds the listen socket, then writes `updates/started`. A failed bind writes nothing.
pub async fn open_listen(addr: SocketAddr, prefix: Option<&Path>) -> io::Result<TcpListener> {
    let listener = TcpListener::bind(addr).await?;
    if let Some(prefix) = prefix {
        note_listening(prefix)?;
    }
    Ok(listener)
}

/// A job is waiting on the print channel, or the worker has started one.
/// The handler's own request guard does not count.
pub(crate) fn print_busy(app: &crate::registration_server::App) -> bool {
    app.prints.has_queued() || app.gate.is_printing()
}

pub(crate) struct InstallOutcome {
    pub held: Option<RequestGuard>,
    pub result: Result<String, StageFailure>,
}

pub(crate) enum StageFailure {
    Checksum,
    Failed,
}

/// Downloads the offer, checks it, unpacks it, and records phase `staged`.
/// Does not exit. The handler does that after the response is built.
pub(crate) async fn install(
    app: &crate::registration_server::App,
    held: Option<RequestGuard>,
) -> InstallOutcome {
    let Some(version) = app.updates.offer() else {
        return InstallOutcome {
            held,
            result: Err(StageFailure::Failed),
        };
    };
    if !app.updates.begin_download() {
        return InstallOutcome {
            held,
            result: Err(StageFailure::Failed),
        };
    }
    let client = github_client(Duration::from_secs(30));
    let base = app.updates.api_base();
    let repo = update_repo_from_env();
    eprintln!("registration-server update: download {version}");
    let download_started = Instant::now();
    let downloaded = match download_release(
        &app.prefix,
        &app.gate,
        held,
        &client,
        &base,
        &repo,
        &version,
        Some(&app.updates),
    )
    .await
    {
        Ok(downloaded) => downloaded,
        Err(err) => {
            eprintln!(
                "registration-server update: download failed {} {}ms",
                err.code(),
                download_started.elapsed().as_millis()
            );
            app.updates.finish_install();
            let failure = if err.code() == "checksum_mismatch" {
                StageFailure::Checksum
            } else {
                StageFailure::Failed
            };
            return InstallOutcome {
                held: err.held,
                result: Err(failure),
            };
        }
    };
    let bytes = std::fs::metadata(&downloaded.path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    eprintln!(
        "registration-server update: downloaded {bytes} bytes {}ms",
        download_started.elapsed().as_millis()
    );
    app.updates.begin_extract();
    let held = downloaded.held;
    let scratch = app.prefix.join("updates").join("payload");
    eprintln!("registration-server update: extract");
    let extract_started = Instant::now();
    let payload = match extract_payload(&downloaded.path, &scratch, Some(&app.updates)) {
        Ok(path) => path,
        Err(err) => {
            eprintln!(
                "registration-server update: extract failed {err} {}ms",
                extract_started.elapsed().as_millis()
            );
            let _ = std::fs::remove_dir_all(&scratch);
            let _ = std::fs::remove_file(&downloaded.path);
            app.updates.finish_install();
            return InstallOutcome {
                held,
                result: Err(StageFailure::Failed),
            };
        }
    };
    eprintln!(
        "registration-server update: extracted {}ms",
        extract_started.elapsed().as_millis()
    );
    eprintln!("registration-server update: unpack {version}");
    let unpack_started = Instant::now();
    let payload_len = tree_bytes(&payload).max(1);
    if let Err(err) = unpack_with(&app.prefix, &version, &payload, &mut |copied| {
        note_stage(
            &app.updates,
            copied,
            payload_len,
            EXTRACT_UNITS,
            UNPACK_UNITS,
        );
    }) {
        eprintln!(
            "registration-server update: unpack failed {err:?} {}ms",
            unpack_started.elapsed().as_millis()
        );
        let _ = std::fs::remove_dir_all(&scratch);
        let _ = std::fs::remove_file(&downloaded.path);
        app.updates.finish_install();
        return InstallOutcome {
            held,
            result: Err(StageFailure::Failed),
        };
    }
    eprintln!(
        "registration-server update: unpacked {}ms",
        unpack_started.elapsed().as_millis()
    );
    app.updates
        .note_extract(EXTRACT_UNITS + UNPACK_UNITS, EXTRACT_UNITS + UNPACK_UNITS);
    app.updates.begin_apply();
    let _ = std::fs::remove_dir_all(&scratch);
    let _ = std::fs::remove_file(&downloaded.path);
    eprintln!("registration-server update: stage");
    let stage_started = Instant::now();
    if let Err(err) = write_staged(
        &app.prefix,
        release_version(),
        &version,
        &app.dist,
        &app.form,
    ) {
        eprintln!(
            "registration-server update: stage failed {err} {}ms",
            stage_started.elapsed().as_millis()
        );
        app.updates.finish_install();
        return InstallOutcome {
            held,
            result: Err(StageFailure::Failed),
        };
    }
    eprintln!(
        "registration-server update: staged from {} to {version} {}ms",
        release_version(),
        stage_started.elapsed().as_millis()
    );
    InstallOutcome {
        held,
        result: Ok(version),
    }
}

/// Records the exit. After the response is sent, a non-test process leaves.
pub(crate) fn request_exit(app: &crate::registration_server::App) {
    eprintln!("registration-server update: exit; rust-reg-run starts the staged version");
    app.exit_requested.store(true, Ordering::Release);
    #[cfg(not(test))]
    {
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            std::process::exit(0);
        });
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct InstallState {
    phase: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    rejected: Vec<String>,
    /// Executable that was running when the upgrade was staged. Empty when unknown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    running: String,
    /// Admin static files the staged process was serving, when that directory is
    /// not already `versions/<from>/admin`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    admin: String,
    /// Operator screens the staged process was serving, when that directory is
    /// not already `versions/<from>/form`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    form: String,
}

#[derive(Clone, Copy)]
pub(crate) struct Day {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

fn write_staged(prefix: &Path, from: &str, to: &str, admin: &Path, form: &Path) -> io::Result<()> {
    let mut state = read_state(prefix).unwrap_or(InstallState {
        phase: String::new(),
        from: String::new(),
        to: String::new(),
        rejected: Vec::new(),
        running: String::new(),
        admin: String::new(),
        form: String::new(),
    });
    state.phase = "staged".into();
    state.from = from.into();
    state.to = to.into();
    let running = current_version_executable();
    if !running.is_empty() {
        state.running = running;
    }
    state.admin = recorded_static(prefix, from, "admin", admin);
    state.form = recorded_static(prefix, from, "form", form);
    write_state(prefix, &state)?;
    clear_started(prefix)?;
    Ok(())
}

/// Keep a static directory that is not already the installed version tree.
fn recorded_static(prefix: &Path, version: &str, name: &str, path: &Path) -> String {
    if !path.is_dir() {
        return String::new();
    }
    let installed = version_dir(prefix, version).join(name);
    if same_dir(path, &installed) {
        return String::new();
    }
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn same_dir(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn clear_started(prefix: &Path) -> io::Result<()> {
    let started = started_path(prefix);
    if started.exists() {
        std::fs::remove_file(&started)?;
        if let Some(parent) = started.parent() {
            sync_dir(parent)?;
        }
    }
    Ok(())
}

fn read_state(prefix: &Path) -> Option<InstallState> {
    let text = std::fs::read_to_string(prefix.join("updates").join("state.yml")).ok()?;
    serde_yaml::from_str(&text).ok()
}

fn write_state(prefix: &Path, state: &InstallState) -> io::Result<()> {
    let text = serde_yaml::to_string(state)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    replace_durable(&prefix.join("updates").join("state.yml"), text.as_bytes())
}

fn started_path(prefix: &Path) -> PathBuf {
    prefix.join("updates").join("started")
}

fn version_dir(prefix: &Path, version: &str) -> PathBuf {
    prefix.join("versions").join(version)
}

fn read_version_line(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

fn replace_durable(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = durable_parent(path) {
        std::fs::create_dir_all(parent)?;
        sync_dir(parent)?;
    }
    let tmp = temp_sibling(path);
    if let Err(err) = write_new(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    if let Some(parent) = durable_parent(path) {
        sync_dir(parent)?;
    }
    Ok(())
}

fn durable_parent(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn temp_sibling(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    name.push(format!(".tmp-{}-{nanos}", std::process::id()));
    path.with_file_name(name)
}

fn write_marker_if_absent(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    replace_durable(path, bytes)
}

/// First 800 units of the extract bar are the package expand. The last 200 are the copy into the version directory.
const EXTRACT_UNITS: u64 = 800;
const UNPACK_UNITS: u64 = 200;

fn note_stage(progress: &ReleaseCheck, done: u64, total: u64, start: u64, span: u64) {
    let portion = if total == 0 {
        span
    } else {
        done.saturating_mul(span) / total
    };
    progress.note_extract(
        start.saturating_add(portion.min(span)),
        EXTRACT_UNITS + UNPACK_UNITS,
    );
}

fn tree_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                total = total.saturating_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0));
            }
        }
    }
    total
}

fn extract_payload(
    asset: &Path,
    dest: &Path,
    progress: Option<&ReleaseCheck>,
) -> io::Result<PathBuf> {
    if is_zip(asset) {
        if dest.exists() {
            std::fs::remove_dir_all(dest)?;
        }
        std::fs::create_dir_all(dest)?;
        unzip(asset, dest, progress)?;
        return find_payload(dest)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "payload"));
    }
    if is_ustar(asset) {
        if dest.exists() {
            std::fs::remove_dir_all(dest)?;
        }
        std::fs::create_dir_all(dest)?;
        untar(asset, dest, progress)?;
        return Ok(dest.to_path_buf());
    }
    platform_expand(asset, dest, progress)?;
    find_payload(dest).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "payload"))
}

fn is_zip(path: &Path) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == [0x50, 0x4b, 0x03, 0x04]
}

fn unzip(asset: &Path, dest: &Path, progress: Option<&ReleaseCheck>) -> io::Result<()> {
    let file = std::fs::File::open(asset)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let total = archive.len().max(1);
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let Some(rel) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
            continue;
        };
        let path = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(path)?;
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&path)?;
            std::io::copy(&mut entry, &mut out)?;
            out.sync_all()?;
        }
        if let Some(progress) = progress {
            note_stage(progress, (index as u64) + 1, total as u64, 0, EXTRACT_UNITS);
        }
    }
    Ok(())
}

fn is_ustar(path: &Path) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut magic = [0u8; 6];
    if file.seek(SeekFrom::Start(257)).is_err() {
        return false;
    }
    file.read_exact(&mut magic).is_ok() && &magic == b"ustar\0"
}

fn untar(asset: &Path, dest: &Path, progress: Option<&ReleaseCheck>) -> io::Result<()> {
    let bytes = std::fs::read(asset)?;
    untar_reporting(&bytes, dest, progress)
}

pub(crate) fn untar_bytes(bytes: &[u8], dest: &Path) -> io::Result<()> {
    untar_reporting(bytes, dest, None)
}

fn untar_reporting(bytes: &[u8], dest: &Path, progress: Option<&ReleaseCheck>) -> io::Result<()> {
    let total = bytes.len() as u64;
    let mut offset = 0;
    while offset + 512 <= bytes.len() {
        let header = &bytes[offset..offset + 512];
        offset += 512;
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        if !tar_checksum_ok(header) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "tar checksum"));
        }
        let size = tar_octal(&header[124..136]).unwrap_or(0) as usize;
        let name = tar_name(header);
        let kind = header[156];
        if offset + size > bytes.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "tar"));
        }
        let data = &bytes[offset..offset + size];
        offset = (offset + size + 511) & !511;
        let Some(rel) = safe_tar_name(&name) else {
            continue;
        };
        let path = dest.join(rel);
        if kind == b'5' {
            std::fs::create_dir_all(path)?;
        } else if kind == b'0' || kind == 0 {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::File::create(&path)?;
            file.write_all(data)?;
            file.sync_all()?;
        }
        if let Some(progress) = progress {
            let shown = (offset as u64).min(total.saturating_sub(1));
            note_stage(progress, shown, total.max(1), 0, EXTRACT_UNITS);
        }
    }
    if let Some(progress) = progress {
        note_stage(progress, total.max(1), total.max(1), 0, EXTRACT_UNITS);
    }
    Ok(())
}

fn tar_name(header: &[u8]) -> String {
    let name = cstr(&header[0..100]);
    let prefix = cstr(&header[345..500]);
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn tar_octal(bytes: &[u8]) -> Option<u64> {
    let text = cstr(bytes);
    let text = text.trim();
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

fn tar_checksum_ok(header: &[u8]) -> bool {
    let Some(stored) = tar_octal(&header[148..156]) else {
        return false;
    };
    let mut sum = 0u64;
    for (index, byte) in header.iter().enumerate() {
        if (148..156).contains(&index) {
            sum += u64::from(b' ');
        } else {
            sum += u64::from(*byte);
        }
    }
    stored == sum
}

fn safe_tar_name(name: &str) -> Option<PathBuf> {
    let name = name.trim_matches('/');
    if name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn platform_expand(asset: &Path, dest: &Path, progress: Option<&ReleaseCheck>) -> io::Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    let total = std::fs::metadata(asset).map(|meta| meta.len()).unwrap_or(0);
    let mut child = platform_command(asset, dest)?.spawn()?;
    loop {
        if let Some(progress) = progress {
            let written = tree_bytes(dest);
            let shown = if total == 0 {
                0
            } else {
                written.min(total.saturating_sub(1))
            };
            note_stage(progress, shown, total.max(1), 0, EXTRACT_UNITS);
        }
        match child.try_wait()? {
            Some(status) if status.success() => {
                if let Some(progress) = progress {
                    note_stage(progress, total.max(1), total.max(1), 0, EXTRACT_UNITS);
                }
                return Ok(());
            }
            Some(_) => {
                return Err(io::Error::new(io::ErrorKind::Other, "extract failed"));
            }
            None => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn platform_command(asset: &Path, dest: &Path) -> io::Result<Command> {
    #[cfg(target_os = "linux")]
    {
        std::fs::create_dir_all(dest)?;
        let mut command = Command::new("dpkg-deb");
        command.arg("-x").arg(asset).arg(dest);
        return Ok(command);
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("pkgutil");
        command.arg("--expand-full").arg(asset).arg(dest);
        return Ok(command);
    }
    #[cfg(target_os = "windows")]
    {
        std::fs::create_dir_all(dest)?;
        let mut command = Command::new(asset);
        command.arg("--staging").arg(dest).arg("--no-service");
        return Ok(command);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (asset, dest);
        Err(io::Error::new(
            io::ErrorKind::Other,
            "no extract tool for this os",
        ))
    }
}

fn find_payload(root: &Path) -> Option<PathBuf> {
    if is_payload(root) {
        return Some(root.to_path_buf());
    }
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_payload(&path) {
                return Some(found);
            }
        }
    }
    None
}

fn is_payload(dir: &Path) -> bool {
    dir.join(version_binary_name()).is_file()
        && dir.join("admin").is_dir()
        && dir.join("form").is_dir()
        && dir.join("RELEASE").is_file()
}

#[cfg(test)]
pub(crate) fn payload_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in files {
            writer.start_file(*name, options).expect("zip name");
            writer.write_all(bytes).expect("zip bytes");
        }
        writer.finish().expect("zip");
    }
    cursor.into_inner()
}

#[cfg(test)]
pub(crate) fn payload_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        out.extend_from_slice(&tar_header(name, bytes.len() as u64));
        out.extend_from_slice(bytes);
        let pad = (512 - (bytes.len() % 512)) % 512;
        out.extend(std::iter::repeat(0).take(pad));
    }
    out.extend(std::iter::repeat(0).take(1024));
    out
}

#[cfg(test)]
fn tar_header(name: &str, size: u64) -> [u8; 512] {
    let mut header = [0u8; 512];
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len().min(100);
    header[..name_len].copy_from_slice(&name_bytes[..name_len]);
    write_tar_octal(&mut header[100..108], 0o644);
    write_tar_octal(&mut header[108..116], 0);
    write_tar_octal(&mut header[116..124], 0);
    write_tar_octal(&mut header[124..136], size);
    write_tar_octal(&mut header[136..148], 0);
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[148..156].fill(b' ');
    let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
    let rendered = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(rendered.as_bytes());
    header
}

#[cfg(test)]
fn write_tar_octal(dst: &mut [u8], value: u64) {
    let digits = dst.len() - 1;
    let text = format!("{value:0digits$o}");
    let bytes = text.as_bytes();
    let n = bytes.len().min(digits);
    dst[..n].copy_from_slice(&bytes[..n]);
    dst[digits] = 0;
}

/// One decision. Writes the phase, then calls `start` with the version directory.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resume(prefix: &Path, start: impl FnOnce(&Path)) -> io::Result<()> {
    let path = decide(prefix)?;
    start(&path);
    Ok(())
}

pub(crate) fn supervise(
    prefix: &Path,
    mut spawn: impl FnMut(&Path) -> io::Result<Child>,
    stop: &AtomicBool,
) -> io::Result<()> {
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let path = decide(prefix)?;
        let mut child = match spawn(&path) {
            Ok(child) => child,
            Err(err) => {
                eprintln!("rust-reg-run: {path:?}: {err}");
                if roll_back_failed_try(prefix)? {
                    continue;
                }
                return Err(err);
            }
        };
        let mut wrapper_checked = false;
        loop {
            if stop.load(Ordering::Acquire) {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(());
            }
            if !wrapper_checked && phase_is(prefix, "current") {
                wrapper_checked = true;
                if let Err(err) = replace_wrapper(prefix, own_wrapper_path().as_deref()) {
                    eprintln!("rust-reg-run: wrapper {err}");
                }
            }
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(err) => {
                    let _ = child.kill();
                    return Err(err);
                }
            }
        }
        roll_back_failed_try(prefix)?;
    }
}

/// The phase is still `trying` and this attempt never bound a port.
/// Record `rolled_back` and point `updates/live` back at the previous version.
fn roll_back_failed_try(prefix: &Path) -> io::Result<bool> {
    let Some(state) = read_state(prefix) else {
        return Ok(false);
    };
    if state.phase != "trying" || started_path(prefix).exists() {
        return Ok(false);
    }
    roll_back(prefix, &state)?;
    Ok(true)
}

pub(crate) fn run_supervisor(prefix: &Path) -> io::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    arm_stop(Arc::clone(&stop));
    supervise(prefix, |dir| spawn_registration(prefix, dir), &stop)
}

fn spawn_registration(prefix: &Path, version_dir: &Path) -> io::Result<Child> {
    Command::new(version_dir.join(version_binary_name()))
        .arg("registration-server")
        .arg("--credentials")
        .arg(prefix.join("data").join("credentials.yml"))
        .arg("--dist")
        .arg(version_dir.join("admin"))
        .arg("--form")
        .arg(version_dir.join("form"))
        .spawn()
}

#[cfg(unix)]
fn arm_stop(flag: Arc<AtomicBool>) {
    unsafe {
        let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        let mut set = set.assume_init();
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
    std::thread::spawn(move || unsafe {
        let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        let mut set = set.assume_init();
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        let mut sig: libc::c_int = 0;
        if libc::sigwait(&set, &mut sig) == 0 {
            flag.store(true, Ordering::Release);
        }
    });
}

#[cfg(not(unix))]
fn arm_stop(flag: Arc<AtomicBool>) {
    let _ = flag;
}

fn decide(prefix: &Path) -> io::Result<PathBuf> {
    let state = read_state(prefix);
    match state.as_ref().map(|state| state.phase.as_str()) {
        Some("staged") => stage_child(prefix, state.expect("staged")),
        Some("trying") => {
            let state = state.expect("trying");
            if state.to.is_empty() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "to"));
            }
            if started_path(prefix).exists() {
                let to = state.to.clone();
                settle(prefix, local_today())?;
                Ok(version_dir(prefix, &to))
            } else {
                backup_running_executable(prefix, &state, local_today())?;
                Ok(version_dir(prefix, &state.to))
            }
        }
        _ => {
            let live = read_version_line(&prefix.join("updates").join("live"))
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "updates/live"))?;
            Ok(version_dir(prefix, &live))
        }
    }
}

fn stage_child(prefix: &Path, mut state: InstallState) -> io::Result<PathBuf> {
    if state.to.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "to"));
    }
    if let Some(current) = tree_to_retire(prefix, &state) {
        state.from = current;
    }
    backup_running_executable(prefix, &state, local_today())?;
    clear_started(prefix)?;
    replace_durable(
        &prefix.join("updates").join("live"),
        format!("{}\n", state.to).as_bytes(),
    )?;
    state.phase = "trying".into();
    write_state(prefix, &state)?;
    Ok(version_dir(prefix, &state.to))
}

fn roll_back(prefix: &Path, state: &InstallState) -> io::Result<PathBuf> {
    if state.from.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "from"));
    }
    replace_durable(
        &prefix.join("updates").join("live"),
        format!("{}\n", state.from).as_bytes(),
    )?;
    restore_backed_up_version(prefix, &state.from)?;
    let to_dir = version_dir(prefix, &state.to);
    if to_dir.exists() {
        std::fs::remove_dir_all(to_dir)?;
    }
    let mut rejected = state.rejected.clone();
    if !state.to.is_empty() && !rejected.iter().any(|version| version == &state.to) {
        rejected.push(state.to.clone());
    }
    write_state(
        prefix,
        &InstallState {
            phase: "rolled_back".into(),
            from: state.from.clone(),
            to: state.to.clone(),
            rejected,
            running: state.running.clone(),
            admin: state.admin.clone(),
            form: state.form.clone(),
        },
    )?;
    Ok(version_dir(prefix, &state.from))
}

fn current_version_executable() -> String {
    let Ok(path) = std::env::current_exe() else {
        return String::new();
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return String::new();
    };
    if name == version_binary_name() {
        path.to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

/// Copy the staged process into `backup/<from>-<date>/` before the new version starts.
/// An executable that is not already `versions/<from>/` is copied there. Admin and form
/// static trees are copied from the directories the staged process was serving, from
/// siblings of that executable, or from `versions/<from>/`. The sources stay in place.
/// A backup that already holds the executable still receives static trees it is missing.
fn backup_running_executable(prefix: &Path, state: &InstallState, today: Day) -> io::Result<()> {
    if state.from.is_empty() {
        return Ok(());
    }
    let dest_root = prefix
        .join("backup")
        .join(backup_dir_name(&state.from, today));
    let mut wrote = false;
    let installed = version_dir(prefix, &state.from).join(version_binary_name());
    let source = PathBuf::from(&state.running);
    if source.is_file() && source != installed {
        let dest_bin = dest_root.join(version_binary_name());
        if !dest_bin.is_file() {
            std::fs::create_dir_all(&dest_root)?;
            std::fs::copy(&source, &dest_bin)?;
            wrote = true;
        }
    }
    for (name, recorded) in [
        ("admin", state.admin.as_str()),
        ("form", state.form.as_str()),
    ] {
        let dest = dest_root.join(name);
        if dest.exists() {
            continue;
        }
        let Some(from) = static_tree_source(prefix, state, name, recorded) else {
            continue;
        };
        if same_dir(&from, &dest) {
            continue;
        }
        std::fs::create_dir_all(&dest_root)?;
        copy_version_tree(&from, &dest)?;
        wrote = true;
    }
    if wrote {
        sync_dir(&dest_root)?;
        if let Some(backup) = dest_root.parent() {
            sync_dir(backup)?;
        }
    }
    Ok(())
}

/// Where the static tree for `name` (`admin` or `form`) is read from.
/// A directory recorded at stage time wins. Otherwise a sibling of the running
/// executable, when that sibling is not already `versions/<from>/<name>`.
/// Otherwise `versions/<from>/<name>`.
fn static_tree_source(
    prefix: &Path,
    state: &InstallState,
    name: &str,
    recorded: &str,
) -> Option<PathBuf> {
    if !recorded.is_empty() {
        let path = PathBuf::from(recorded);
        if path.is_dir() {
            return Some(path);
        }
    }
    let installed = version_dir(prefix, &state.from).join(name);
    if !state.running.is_empty() {
        if let Some(parent) = Path::new(&state.running).parent() {
            let sibling = parent.join(name);
            if sibling.is_dir() && !same_dir(&sibling, &installed) {
                return Some(sibling);
            }
        }
    }
    installed.is_dir().then_some(installed)
}

/// Put the newest `backup/<version>-*` tree back at `versions/<version>/`.
/// A missing binary restores the whole tree. A binary that is already there still
/// receives `admin/` and `form/` when those directories are missing.
fn restore_backed_up_version(prefix: &Path, version: &str) -> io::Result<()> {
    let dest = version_dir(prefix, version);
    let Some(backup) = latest_backup(prefix, version) else {
        return Ok(());
    };
    let binary = dest.join(version_binary_name());
    if !binary.is_file() {
        copy_version_tree(&backup, &dest)?;
    } else {
        for name in ["admin", "form"] {
            let from = backup.join(name);
            let to = dest.join(name);
            if from.is_dir() && !to.exists() {
                copy_version_tree(&from, &to)?;
            }
        }
    }
    if let Some(versions) = dest.parent() {
        sync_dir(versions)?;
    }
    Ok(())
}

fn latest_backup(prefix: &Path, version: &str) -> Option<PathBuf> {
    let prefix_name = format!("{version}-");
    let mut best: Option<(i64, PathBuf)> = None;
    for entry in std::fs::read_dir(prefix.join("backup")).ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        let name = entry.file_name().into_string().ok()?;
        if !name.starts_with(&prefix_name) {
            continue;
        }
        let day = backup_day(&name)?;
        let ordinal = ymd_to_days(day);
        if best
            .as_ref()
            .map(|(have, _)| ordinal >= *have)
            .unwrap_or(true)
        {
            best = Some((ordinal, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

fn copy_version_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_version_tree(&entry.path(), &dest)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

/// Move entries from `from` into `to`. A name that is already in `to` stays as it is.
fn merge_into(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if dest.exists() {
                merge_into(&entry.path(), &dest)?;
            } else {
                std::fs::rename(entry.path(), dest)?;
            }
        } else if kind.is_file() && !dest.exists() {
            std::fs::rename(entry.path(), dest)?;
        }
    }
    Ok(())
}

/// A version directory that holds a tree. A missing directory and an empty one do not.
fn version_occupied(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}

/// The tree to move aside. `versions/<from>/` when it has files. Otherwise the
/// version named in `updates/live`, when that directory has files and is not `to`.
fn tree_to_retire(prefix: &Path, state: &InstallState) -> Option<String> {
    if version_occupied(&version_dir(prefix, &state.from)) {
        return Some(state.from.clone());
    }
    let live = read_version_line(&prefix.join("updates").join("live"))?;
    if live.is_empty() || live == state.to || live == state.from {
        return None;
    }
    version_occupied(&version_dir(prefix, &live)).then_some(live)
}

/// While the phase is `trying`, move the previous tree to a dated backup and set `current`.
/// A missing or empty previous tree, with no current tree either, leaves `backup/` unchanged.
pub(crate) fn settle(prefix: &Path, today: Day) -> io::Result<()> {
    let Some(mut state) = read_state(prefix) else {
        return Ok(());
    };
    if state.phase != "trying" {
        return Ok(());
    }
    if state.from.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "from"));
    }
    let retire = tree_to_retire(prefix, &state);
    let from_name = retire.clone().unwrap_or_else(|| state.from.clone());
    let from_dir = version_dir(prefix, &from_name);
    let backup_root = prefix.join("backup");
    let backup = backup_root.join(backup_dir_name(&from_name, today));
    if version_occupied(&from_dir) {
        state.from = from_name;
        std::fs::create_dir_all(&backup_root)?;
        if backup.exists() {
            merge_into(&from_dir, &backup)?;
            std::fs::remove_dir_all(&from_dir)?;
        } else {
            std::fs::rename(&from_dir, &backup)?;
        }
        sync_dir(&backup_root)?;
        if let Some(versions) = from_dir.parent() {
            sync_dir(versions)?;
        }
    }
    prune_backups(prefix, today)?;
    state.phase = "current".into();
    write_state(prefix, &state)?;
    Ok(())
}

fn backup_dir_name(version: &str, day: Day) -> String {
    format!("{version}-{:04}{:02}{:02}", day.year, day.month, day.day)
}

fn backup_day(name: &str) -> Option<Day> {
    let (_version, date) = name.rsplit_once('-')?;
    if date.len() != 8 || !date.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(Day {
        year: date[0..4].parse().ok()?,
        month: date[4..6].parse().ok()?,
        day: date[6..8].parse().ok()?,
    })
}

fn prune_backups(prefix: &Path, today: Day) -> io::Result<()> {
    let backup = prefix.join("backup");
    let entries = match std::fs::read_dir(&backup) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(day) = backup_day(&name) else {
            continue;
        };
        if ymd_to_days(today) - ymd_to_days(day) >= 7 {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    if backup.exists() {
        sync_dir(&backup)?;
    }
    Ok(())
}

fn ymd_to_days(day: Day) -> i64 {
    let mut year = day.year as i64;
    let mut month = day.month as i64;
    if month <= 2 {
        year -= 1;
        month += 12;
    }
    365 * year + year / 4 - year / 100 + year / 400 + (153 * month - 457) / 5 + day.day as i64 - 306
}

fn phase_is(prefix: &Path, phase: &str) -> bool {
    read_state(prefix).is_some_and(|state| state.phase == phase)
}

/// The running wrapper, when this process is `rust-reg-run`.
fn own_wrapper_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .filter(|path| is_supervisor_path(path))
}

/// The parent executable, when that file is `rust-reg-run` or `rust-reg-run.exe`.
fn server_wrapper_path() -> Option<PathBuf> {
    parent_executable().filter(|path| is_supervisor_path(path))
}

/// After the phase is `current`, copy `versions/<to>/rust-reg-run` onto `detected` when the
/// SHA-256 differs. `detected` is the path the wrapper was started from. A missing path does
/// not fall back to `<prefix>/bin/`. Any other phase leaves that path alone.
pub(crate) fn replace_wrapper(prefix: &Path, detected: Option<&Path>) -> io::Result<()> {
    let Some(state) = read_state(prefix) else {
        return Ok(());
    };
    if state.phase != "current" || state.to.is_empty() {
        return Ok(());
    }
    let Some(detected) = detected.filter(|path| !path.as_os_str().is_empty()) else {
        return Ok(());
    };
    let source = version_dir(prefix, &state.to).join(wrapper_binary_name());
    if !source.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "wrapper"));
    }
    if !detected.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "wrapper"));
    }
    let new_bytes = std::fs::read(&source)?;
    let old_bytes = std::fs::read(detected)?;
    if sha256_hex(&new_bytes) == sha256_hex(&old_bytes) {
        return Ok(());
    }
    write_replaced(detected, &new_bytes)
}

/// Write `bytes` beside `dest`, fsync that file, rename it onto `dest`, then fsync the directory.
fn write_replaced(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = match dest.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = dest
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "wrapper"))?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(".new");
    let tmp = parent.join(tmp_name);
    let write = (|| -> io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata()?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(&tmp, perms)?;
        }
        std::fs::rename(&tmp, dest)?;
        sync_dir(&parent)?;
        Ok(())
    })();
    if write.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write
}

fn note_listening(prefix: &Path) -> io::Result<()> {
    note_listening_on(prefix, local_today())
}

pub(crate) fn note_listening_on(prefix: &Path, today: Day) -> io::Result<()> {
    let phase = read_state(prefix)
        .map(|state| state.phase)
        .unwrap_or_default();
    write_marker_if_absent(&started_path(prefix), b"\n")?;
    if phase == "trying" {
        settle(prefix, today)?;
    }
    if let Err(err) = replace_wrapper(prefix, server_wrapper_path().as_deref()) {
        eprintln!("registration-server update: wrapper {err}");
    }
    Ok(())
}

#[cfg(unix)]
fn local_today() -> Day {
    use std::mem::MaybeUninit;
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm = MaybeUninit::<libc::tm>::uninit();
        if libc::localtime_r(&now, tm.as_mut_ptr()).is_null() {
            return Day {
                year: 1970,
                month: 1,
                day: 1,
            };
        }
        let tm = tm.assume_init();
        Day {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
        }
    }
}

#[cfg(not(unix))]
fn local_today() -> Day {
    let days = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    civil_from_epoch_days(days)
}

#[cfg(not(unix))]
fn civil_from_epoch_days(days: i64) -> Day {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    Day {
        year: year as i32,
        month: month as u32,
        day: day as u32,
    }
}

/// `Ok(None)` clears the offer. `Err` leaves the previous offer in place.
async fn fetch_offer(
    client: &reqwest::Client,
    api_base: &str,
    repo: &str,
    running: &str,
    rejected: &[String],
    include_prerelease: bool,
) -> Result<Option<String>, ()> {
    if include_prerelease {
        fetch_release_list(client, api_base, repo, running, rejected).await
    } else {
        fetch_latest(client, api_base, repo, running, rejected).await
    }
}

/// `Ok(None)` clears the offer. `Err` leaves the previous offer in place.
async fn fetch_latest(
    client: &reqwest::Client,
    api_base: &str,
    repo: &str,
    running: &str,
    rejected: &[String],
) -> Result<Option<String>, ()> {
    let url = format!(
        "{}/repos/{repo}/releases/latest",
        api_base.trim_end_matches('/')
    );
    let response = client.get(&url).send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let body: LatestRelease = response.json().await.map_err(|_| ())?;
    let Some(tag_name) = body.tag_name else {
        return Err(());
    };
    if body.draft.unwrap_or(false) || body.prerelease.unwrap_or(false) {
        return Ok(None);
    }
    match newer(&tag_name, running) {
        Some(version) if !rejected.iter().any(|item| item == &version) => Ok(Some(version)),
        _ => Ok(None),
    }
}

async fn fetch_release_list(
    client: &reqwest::Client,
    api_base: &str,
    repo: &str,
    running: &str,
    rejected: &[String],
) -> Result<Option<String>, ()> {
    let url = format!(
        "{}/repos/{repo}/releases?per_page=100",
        api_base.trim_end_matches('/')
    );
    let response = client.get(&url).send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let bodies: Vec<LatestRelease> = response.json().await.map_err(|_| ())?;
    Ok(newest_offer(&bodies, running, rejected))
}

fn newest_offer(bodies: &[LatestRelease], running: &str, rejected: &[String]) -> Option<String> {
    let mut best: Option<String> = None;
    for body in bodies {
        if body.draft.unwrap_or(false) {
            continue;
        }
        let Some(tag_name) = body.tag_name.as_deref() else {
            continue;
        };
        let Some(version) = newer(tag_name, running) else {
            continue;
        };
        if rejected.iter().any(|item| item == &version) {
            continue;
        }
        best = Some(match best {
            Some(current) if newer(&format!("v{version}"), &current).is_none() => current,
            _ => version,
        });
    }
    best
}

#[derive(serde::Deserialize)]
struct LatestRelease {
    tag_name: Option<String>,
    draft: Option<bool>,
    prerelease: Option<bool>,
}

/// Runs on the sync thread after the listen socket is open. Not a sync download.
pub(crate) async fn update_loop(
    app: crate::registration_server::App,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = github_client(Duration::from_secs(30));
    loop {
        if *shutdown.borrow() {
            return;
        }
        let now = SystemTime::now();
        if app.updates.due(now) {
            let rejected = read_rejected(Path::new("updates/state.yml"));
            let busy = app
                .updates
                .scheduled(
                    &app.gate,
                    &client,
                    GITHUB_API,
                    &update_repo_from_env(),
                    release_version(),
                    &rejected,
                    now,
                )
                .await;
            if busy {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    result = shutdown.changed() => {
                        if result.is_err() || *shutdown.borrow() {
                            return;
                        }
                    }
                }
                continue;
            }
        }
        let wait = match app.updates.last_attempt() {
            Some(last) => last
                .checked_add(UPDATE_EVERY)
                .and_then(|due| due.duration_since(SystemTime::now()).ok())
                .unwrap_or(UPDATE_EVERY),
            None => Duration::from_secs(0),
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn extract_progress_follows_the_download() {
        let check = super::ReleaseCheck::with_prerelease(false);
        assert!(check.begin_download());
        check.note_download(4, 4);
        check.begin_extract();
        let (phase, received, total) = check.progress();
        assert_eq!(phase, "extracting");
        assert_eq!(received, 0);
        assert_eq!(total, super::EXTRACT_UNITS + super::UNPACK_UNITS);
        check.note_extract(400, 1000);
        let (phase, received, total) = check.progress();
        assert_eq!(phase, "extracting");
        assert_eq!(received, 400);
        assert_eq!(total, 1000);
        check.begin_apply();
        assert_eq!(check.progress().0, "applying");
    }

    #[test]
    fn the_wrapper_name_is_rust_reg_run() {
        assert!(super::is_supervisor_path(std::path::Path::new(
            "rust-reg-run"
        )));
        assert!(super::is_supervisor_path(std::path::Path::new(
            "rust-reg-run.exe"
        )));
        assert!(super::is_supervisor_path(std::path::Path::new(
            "/Library/rust-reg/bin/rust-reg-run"
        )));
        assert!(!super::is_supervisor_path(std::path::Path::new(
            "/Library/rust-reg/versions/2.0.0-alpha.1/rust-reg"
        )));
        assert!(!super::is_supervisor_path(std::path::Path::new(
            "rust-reg-run-extra"
        )));
    }

    #[test]
    fn this_process_is_not_under_the_wrapper() {
        assert!(!super::parent_is_supervisor());
    }

    mod download_url {
        use super::super::{asset_name, release_file_url, GITHUB_API, UPDATE_REPO};

        #[test]
        fn github_download_uses_the_release_host() {
            let name = asset_name("2.0.0-alpha.1");
            assert_eq!(
                release_file_url(GITHUB_API, UPDATE_REPO, "2.0.0-alpha.1", &name),
                format!("https://github.com/{UPDATE_REPO}/releases/download/release-2.0.0-alpha.1/{name}")
            );
            assert_eq!(
                release_file_url("http://127.0.0.1:9/", "owner/repo", "0.2.0", "SHA256SUMS"),
                "http://127.0.0.1:9/repos/owner/repo/releases/download/release-0.2.0/SHA256SUMS"
            );
        }

        #[test]
        fn the_mac_upgrade_follows_the_processor() {
            assert_eq!(
                super::super::macos_upgrade_archive("2.0.0-alpha.2", true),
                "rust-reg-2.0.0-alpha.2-macos.zip"
            );
            assert_eq!(
                super::super::macos_upgrade_archive("2.0.0-alpha.2", false),
                "rust-reg-2.0.0-alpha.2-macos-intel.zip"
            );
        }
    }

    mod stamp {
        use super::super::release_tag::{bake_release_tag, parse_release_tag};
        use super::super::{release_file_contents, release_tag, release_version};

        #[test]
        fn a_release_tag_is_baked() {
            assert_eq!(parse_release_tag("v0.2.0", "0.2.0").unwrap(), "v0.2.0");
            let baked = concat!("v", env!("CARGO_PKG_VERSION"));
            assert_eq!(release_tag(), baked);
            assert_eq!(release_version(), env!("CARGO_PKG_VERSION"));
            assert_eq!(release_file_contents(), format!("{baked}\n"));
        }

        #[test]
        fn a_tag_must_match_cargo() {
            assert!(parse_release_tag("v0.2.0", "0.1.0").is_err());
            assert!(parse_release_tag("0.2.0", "0.2.0").is_err());
            assert_eq!(bake_release_tag(None, "0.1.0").unwrap(), "v0.1.0");
            assert_eq!(release_tag(), concat!("v", env!("CARGO_PKG_VERSION")));
        }
    }

    mod compare {
        use super::super::newer;

        #[test]
        fn a_greater_tag_is_newer() {
            assert_eq!(newer("v0.2.0", "0.1.0").as_deref(), Some("0.2.0"));
            assert_eq!(newer("v0.1.1", "0.1.0").as_deref(), Some("0.1.1"));
            assert_eq!(newer("v1.0.0", "0.9.9").as_deref(), Some("1.0.0"));
        }

        #[test]
        fn the_same_or_older_tag_is_not_offered() {
            assert_eq!(newer("v0.1.0", "0.1.0"), None);
            assert_eq!(newer("v0.0.9", "0.1.0"), None);
        }

        #[test]
        fn a_tag_that_is_not_a_release_is_rejected() {
            assert_eq!(newer("0.2.0", "0.1.0"), None);
            assert_eq!(newer("v0.2", "0.1.0"), None);
            assert_eq!(newer("v0.2.0.1", "0.1.0"), None);
            assert_eq!(newer("v1.2.3-b1", "0.1.0"), None);
            assert_eq!(newer("v1.2.3-alpha", "0.1.0"), None);
            assert_eq!(newer("v1.2.3-alpha.01", "0.1.0"), None);
            assert_eq!(newer("v1.2.3+sha", "0.1.0"), None);
        }

        #[test]
        fn an_alpha_beta_or_rc_is_ordered() {
            assert_eq!(
                newer("v1.2.3-alpha.2", "1.2.3-alpha.1").as_deref(),
                Some("1.2.3-alpha.2")
            );
            assert_eq!(
                newer("v1.2.3-alpha.10", "1.2.3-alpha.2").as_deref(),
                Some("1.2.3-alpha.10")
            );
            assert_eq!(
                newer("v1.2.3-beta.1", "1.2.3-alpha.10").as_deref(),
                Some("1.2.3-beta.1")
            );
            assert_eq!(
                newer("v1.2.3-beta.2", "1.2.3-beta.1").as_deref(),
                Some("1.2.3-beta.2")
            );
            assert_eq!(
                newer("v1.2.3-rc.1", "1.2.3-beta.2").as_deref(),
                Some("1.2.3-rc.1")
            );
            assert_eq!(newer("v1.2.3", "1.2.3-rc.1").as_deref(), Some("1.2.3"));
            assert_eq!(
                newer("v1.2.4-alpha.1", "1.2.3").as_deref(),
                Some("1.2.4-alpha.1")
            );
            assert_eq!(newer("v1.2.3-rc.1", "0.1.0").as_deref(), Some("1.2.3-rc.1"));
            assert_eq!(newer("v1.2.3-alpha.1", "1.2.3"), None);
            assert_eq!(newer("v1.2.3-alpha.1", "1.2.3-beta.1"), None);
            assert_eq!(newer("v1.2.3-rc.1", "1.2.3-rc.1"), None);
            assert_eq!(
                newer("release-2.0.0-alpha.1", "0.1.0").as_deref(),
                Some("2.0.0-alpha.1")
            );
            assert_eq!(newer("release-2.0.0-alpha.1", "2.0.0-beta.1"), None);
        }
    }

    mod look {
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, SystemTime};

        use axum::extract::Request;
        use axum::http::StatusCode;
        use axum::routing::get;
        use axum::Router;
        use tokio::net::TcpListener;

        use crate::registration_server::gate::Gate;

        use super::super::{
            check_due, github_client, read_rejected, update_repo, ReleaseCheck, UPDATE_EVERY,
            UPDATE_REPO,
        };

        struct Hit {
            path: String,
            query: String,
            authorization: Option<String>,
            user_agent: Option<String>,
            accept: Option<String>,
        }

        async fn stand_in(
            bodies: Vec<(StatusCode, &'static str)>,
        ) -> (String, Arc<Mutex<Vec<Hit>>>) {
            let hits = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&hits);
            let left = Arc::new(Mutex::new(bodies));
            let app = Router::new().route(
                "/repos/{owner}/{repo}/releases/latest",
                get(move |request: Request| {
                    let recorded = Arc::clone(&recorded);
                    let left = Arc::clone(&left);
                    async move {
                        recorded.lock().expect("hits").push(Hit {
                            path: request.uri().path().to_string(),
                            query: request.uri().query().unwrap_or("").to_string(),
                            authorization: request
                                .headers()
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_string),
                            user_agent: request
                                .headers()
                                .get("user-agent")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_string),
                            accept: request
                                .headers()
                                .get("accept")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_string),
                        });
                        let mut queue = left.lock().expect("bodies");
                        let (status, body) = if queue.len() > 1 {
                            queue.remove(0)
                        } else {
                            *queue.first().expect("a response")
                        };
                        (status, body)
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            (format!("http://{addr}"), hits)
        }

        fn client() -> reqwest::Client {
            github_client(Duration::from_secs(5))
        }

        fn no_token(hit: &Hit) {
            assert!(hit.authorization.is_none());
            assert!(!hit.path.contains("token"));
            assert!(!hit.query.contains("token"));
            assert_eq!(hit.user_agent.as_deref(), Some("rust-reg"));
            assert_eq!(hit.accept.as_deref(), Some("application/vnd.github+json"));
        }

        #[tokio::test]
        async fn latest_is_kept_when_it_is_newer() {
            let (base, hits) = stand_in(vec![(
                StatusCode::OK,
                r#"{"tag_name":"v0.2.0","draft":false,"prerelease":false}"#,
            )])
            .await;
            let look = ReleaseCheck::new();
            let gate = Arc::new(Gate::new());
            let guard = gate.enter_print();
            let waiting = {
                let gate = Arc::clone(&gate);
                let base = base.clone();
                let client = client();
                tokio::spawn(async move {
                    look.scheduled(
                        &gate,
                        &client,
                        &base,
                        UPDATE_REPO,
                        "0.1.0",
                        &[],
                        SystemTime::UNIX_EPOCH,
                    )
                    .await;
                    look
                })
            };
            tokio::task::yield_now().await;
            assert!(hits.lock().expect("hits").is_empty());
            assert!(!waiting.is_finished());
            drop(guard);
            let look = waiting.await.unwrap();
            let seen = hits.lock().expect("hits");
            assert_eq!(seen.len(), 1);
            assert!(seen[0].path.contains("OpenDOps/pdf-badges"));
            no_token(&seen[0]);
            assert_eq!(look.offer().as_deref(), Some("0.2.0"));
        }

        #[tokio::test]
        async fn draft_prerelease_and_rejected_are_skipped() {
            let (base, _) = stand_in(vec![
                (
                    StatusCode::OK,
                    r#"{"tag_name":"v0.2.0","draft":true,"prerelease":false}"#,
                ),
                (
                    StatusCode::OK,
                    r#"{"tag_name":"v0.2.0","draft":false,"prerelease":true}"#,
                ),
                (
                    StatusCode::OK,
                    r#"{"tag_name":"v0.2.0","draft":false,"prerelease":false}"#,
                ),
            ])
            .await;
            let client = client();
            let gate = Gate::new();
            let now = SystemTime::UNIX_EPOCH;
            let draft = ReleaseCheck::new();
            let _ = draft
                .force(&gate, &client, &base, UPDATE_REPO, "0.1.0", &[], now)
                .await;
            assert_eq!(draft.offer(), None);
            let preview = ReleaseCheck::new();
            let _ = preview
                .force(&gate, &client, &base, UPDATE_REPO, "0.1.0", &[], now)
                .await;
            assert_eq!(preview.offer(), None);

            let path = std::env::temp_dir().join(format!(
                "rust-reg-state-{}-rejected.yml",
                std::process::id()
            ));
            std::fs::write(
                &path,
                "phase: rolled_back\nfrom: \"0.1.0\"\nto: \"0.2.0\"\nrejected:\n  - \"0.2.0\"\n",
            )
            .unwrap();
            let rejected = read_rejected(&path);
            let _ = std::fs::remove_file(&path);
            assert_eq!(rejected, vec!["0.2.0".to_string()]);
            let skipped = ReleaseCheck::new();
            let _ = skipped
                .force(&gate, &client, &base, UPDATE_REPO, "0.1.0", &rejected, now)
                .await;
            assert_eq!(skipped.offer(), None);
        }

        #[tokio::test]
        async fn development_keeps_the_newest_prerelease() {
            use super::super::update_prerelease;

            assert!(!update_prerelease(None));
            assert!(!update_prerelease(Some("0")));
            assert!(update_prerelease(Some("1")));
            assert!(update_prerelease(Some("true")));

            let hits = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&hits);
            let app = Router::new().route(
                "/repos/{owner}/{repo}/releases",
                get(move |request: Request| {
                    let recorded = Arc::clone(&recorded);
                    async move {
                        recorded
                            .lock()
                            .expect("hits")
                            .push(request.uri().path().to_string());
                        (
                            StatusCode::OK,
                            r#"[
                                {"tag_name":"release-1.2.3","draft":false,"prerelease":false},
                                {"tag_name":"release-2.0.0-alpha.1","draft":false,"prerelease":true},
                                {"tag_name":"release-9.0.0","draft":true,"prerelease":false},
                                {"tag_name":"release-2.0.0-beta.1","draft":false,"prerelease":true}
                            ]"#,
                        )
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let look = ReleaseCheck::with_prerelease(true);
            let gate = Gate::new();
            let offer = look
                .force(
                    &gate,
                    &client(),
                    &format!("http://{addr}"),
                    UPDATE_REPO,
                    "0.1.0",
                    &[],
                    SystemTime::UNIX_EPOCH,
                )
                .await
                .unwrap_or_else(|_| panic!("list"));
            assert_eq!(offer.as_deref(), Some("2.0.0-beta.1"));
            let seen = hits.lock().expect("hits");
            assert_eq!(seen.len(), 1);
            assert!(seen[0].ends_with("/releases"));
            assert!(!seen[0].contains("/latest"));
        }

        #[tokio::test]
        async fn a_failed_check_keeps_the_offer() {
            let (base, _) = stand_in(vec![
                (
                    StatusCode::OK,
                    r#"{"tag_name":"v0.2.0","draft":false,"prerelease":false}"#,
                ),
                (StatusCode::INTERNAL_SERVER_ERROR, "no"),
                (
                    StatusCode::OK,
                    r#"{"tag_name":"v0.1.0","draft":false,"prerelease":false}"#,
                ),
            ])
            .await;
            let look = ReleaseCheck::new();
            let client = client();
            let gate = Gate::new();
            let now = SystemTime::UNIX_EPOCH;
            let _ = look
                .force(&gate, &client, &base, UPDATE_REPO, "0.1.0", &[], now)
                .await;
            assert_eq!(look.offer().as_deref(), Some("0.2.0"));
            let _ = look
                .force(&gate, &client, &base, UPDATE_REPO, "0.1.0", &[], now)
                .await;
            assert_eq!(look.offer().as_deref(), Some("0.2.0"));
            let _ = look
                .force(&gate, &client, &base, UPDATE_REPO, "0.1.0", &[], now)
                .await;
            assert_eq!(look.offer(), None);
        }

        #[tokio::test]
        async fn the_repo_override_is_the_path() {
            assert_eq!(update_repo(None), UPDATE_REPO);
            assert_eq!(update_repo(Some("")), UPDATE_REPO);
            assert_eq!(update_repo(Some("other/name")), "other/name");
            let (base, hits) = stand_in(vec![(
                StatusCode::OK,
                r#"{"tag_name":"v0.2.0","draft":false,"prerelease":false}"#,
            )])
            .await;
            let look = ReleaseCheck::new();
            let _ = look
                .force(
                    &Gate::new(),
                    &client(),
                    &base,
                    &update_repo(Some("other/name")),
                    "0.1.0",
                    &[],
                    SystemTime::UNIX_EPOCH,
                )
                .await;
            let seen = hits.lock().expect("hits");
            assert_eq!(seen.len(), 1);
            assert!(seen[0].path.contains("/repos/other/name/releases/latest"));
            no_token(&seen[0]);
        }

        #[test]
        fn the_check_is_due_at_open_and_six_hours() {
            let opened = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
            assert!(check_due(None, opened));
            let attempt = opened;
            let five_hours = attempt + Duration::from_secs(5 * 60 * 60);
            let six_hours = attempt + UPDATE_EVERY;
            assert!(!check_due(Some(attempt), five_hours));
            assert!(check_due(Some(attempt), six_hours));
        }

        #[tokio::test]
        async fn a_forced_check_ignores_the_wait() {
            let (base, hits) = stand_in(vec![(
                StatusCode::OK,
                r#"{"tag_name":"v0.2.0","draft":false,"prerelease":false}"#,
            )])
            .await;
            let look = ReleaseCheck::new();
            let client = client();
            let gate = Gate::new();
            let opened = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
            let _ = look
                .force(&gate, &client, &base, UPDATE_REPO, "0.1.0", &[], opened)
                .await;
            hits.lock().expect("hits").clear();
            let five_hours = opened + Duration::from_secs(5 * 60 * 60);
            look.scheduled(&gate, &client, &base, UPDATE_REPO, "0.1.0", &[], five_hours)
                .await;
            assert!(hits.lock().expect("hits").is_empty());
            let _ = look
                .force(&gate, &client, &base, UPDATE_REPO, "0.1.0", &[], five_hours)
                .await;
            assert_eq!(hits.lock().expect("hits").len(), 1);
            assert!(!look.due(five_hours + Duration::from_secs(5 * 60 * 60)));
            assert!(look.due(five_hours + UPDATE_EVERY));
        }
    }

    mod ask {
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, SystemTime};

        use axum::body::{to_bytes, Body};
        use axum::extract::Request;
        use axum::http::StatusCode;
        use axum::routing::get;
        use axum::Router;
        use serde_json::Value;
        use tokio::net::TcpListener;
        use tokio::sync::Notify;
        use tower::ServiceExt;

        use crate::registration_server::credentials::{CredentialFile, Credentials};
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::App;

        use super::super::{github_client, release_version, UPDATE_REPO};

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "reg-update-{}-{}",
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

        struct Desk {
            _dir: TempDir,
            _file: CredentialFile,
            _worker: crate::registration_server::print_queue::PrintWorker,
            app: App,
        }

        fn desk() -> Desk {
            let dir = TempDir::new();
            let file = CredentialFile::new(dir.0.join("credentials.yml"));
            let mut credentials = Credentials::default();
            credentials.set_device_id("device-1").unwrap();
            credentials
                .bind("EVT", "token-value", Some("Event".into()))
                .unwrap();
            file.store(&credentials).unwrap();
            let current = Arc::new(Mutex::new(CurrentEvent::new(&dir.0)));
            CurrentEvent::lock(&current).open_bound(&file).unwrap();
            let (mut app, worker) = App::new();
            app.current = current;
            Desk {
                _dir: dir,
                _file: file,
                _worker: worker,
                app,
            }
        }

        fn admin_cookie(app: &App) -> String {
            let key = CurrentEvent::lock(&app.current)
                .with_db(|db| {
                    db.file()
                        .query_row("SELECT key FROM keys WHERE is_admin = 1", [], |row| {
                            row.get::<_, String>(0)
                        })
                        .unwrap()
                })
                .unwrap();
            format!("desk={key}")
        }

        async fn send(
            app: &App,
            method: &str,
            uri: &str,
            cookie: Option<&str>,
        ) -> (StatusCode, Value) {
            let mut builder = Request::builder().method(method).uri(uri);
            if method != "GET" {
                builder = builder.header("content-type", "application/json");
            }
            if let Some(cookie) = cookie {
                builder = builder.header("cookie", cookie);
            }
            let response = crate::registration_server::http::router(app.clone())
                .oneshot(builder.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap_or(Value::Null)
            };
            (status, body)
        }

        struct Hit {
            path: String,
        }

        async fn stand_in(
            status: StatusCode,
            body: &'static str,
        ) -> (String, Arc<Mutex<Vec<Hit>>>) {
            let hits = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&hits);
            let app = Router::new().route(
                "/repos/{owner}/{repo}/releases/latest",
                get(move |request: Request| {
                    let recorded = Arc::clone(&recorded);
                    async move {
                        recorded.lock().expect("hits").push(Hit {
                            path: request.uri().path().to_string(),
                        });
                        (status, body)
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            (format!("http://{addr}"), hits)
        }

        fn updates_absent() {
            assert!(!std::path::Path::new("updates").exists());
        }

        #[tokio::test]
        async fn update_names_the_newer_version() {
            let venue = desk();
            let cookie = admin_cookie(&venue.app);
            let (base, hits) = stand_in(
                StatusCode::OK,
                r#"{"tag_name":"v999.0.0","draft":false,"prerelease":false}"#,
            )
            .await;
            venue.app.updates.set_api_base(&base);
            let _ = venue
                .app
                .updates
                .force(
                    &venue.app.gate,
                    &github_client(Duration::from_secs(5)),
                    &base,
                    UPDATE_REPO,
                    release_version(),
                    &[],
                    SystemTime::UNIX_EPOCH,
                )
                .await;
            assert_eq!(hits.lock().expect("hits").len(), 1);

            let (status, body) = send(&venue.app, "GET", "/api/update", Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["ok"], true);
            assert_eq!(body["data"]["version"], release_version());
            assert_eq!(body["data"]["newer"], "999.0.0");
            assert_eq!(hits.lock().expect("hits").len(), 1);
            updates_absent();
        }

        #[tokio::test]
        async fn update_without_an_offer() {
            let venue = desk();
            let cookie = admin_cookie(&venue.app);
            let (base, hits) = stand_in(StatusCode::OK, r#"{"tag_name":"v0.2.0"}"#).await;
            venue.app.updates.set_api_base(&base);
            let (status, body) = send(&venue.app, "GET", "/api/update", Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["data"]["version"], release_version());
            assert!(body["data"]["newer"].is_null());
            assert!(hits.lock().expect("hits").is_empty());
            updates_absent();
        }

        #[tokio::test]
        async fn update_requires_the_desk_cookie() {
            let venue = desk();
            let (base, hits) = stand_in(
                StatusCode::OK,
                r#"{"tag_name":"v999.0.0","draft":false,"prerelease":false}"#,
            )
            .await;
            venue.app.updates.set_api_base(&base);
            let (status, body) = send(&venue.app, "GET", "/api/update", None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body["error"]["code"], "unauthorized");
            let (status, body) = send(&venue.app, "POST", "/api/update/check", None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body["error"]["code"], "unauthorized");
            let (status, body) = send(&venue.app, "POST", "/api/update", None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body["error"]["code"], "unauthorized");
            assert!(hits.lock().expect("hits").is_empty());
            assert!(!venue
                .app
                .exit_requested
                .load(std::sync::atomic::Ordering::Acquire));
        }

        #[tokio::test]
        async fn check_asks_github_now() {
            let venue = desk();
            let cookie = admin_cookie(&venue.app);
            let (base, hits) = stand_in(
                StatusCode::OK,
                r#"{"tag_name":"v999.0.0","draft":false,"prerelease":false}"#,
            )
            .await;
            venue.app.updates.set_api_base(&base);
            let earlier = SystemTime::now()
                .checked_sub(Duration::from_secs(5 * 60 * 60))
                .unwrap();
            venue.app.updates.note_attempt(earlier);
            assert!(!venue.app.updates.due(SystemTime::now()));

            let (status, body) = send(&venue.app, "POST", "/api/update/check", Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["data"]["version"], release_version());
            assert_eq!(body["data"]["newer"], "999.0.0");
            let seen = hits.lock().expect("hits");
            assert_eq!(seen.len(), 1);
            assert!(seen[0]
                .path
                .contains("/repos/OpenDOps/pdf-badges/releases/latest"));
            drop(seen);
            updates_absent();

            let (status, _) = send(&venue.app, "GET", "/health", None).await;
            assert_eq!(status, StatusCode::OK);
        }

        #[tokio::test]
        async fn check_failure_keeps_the_offer() {
            let venue = desk();
            let cookie = admin_cookie(&venue.app);
            venue.app.updates.store_offer(Some("0.2.0".to_string()));
            let (base, _) = stand_in(StatusCode::INTERNAL_SERVER_ERROR, "no").await;
            venue.app.updates.set_api_base(&base);
            let (status, body) = send(&venue.app, "POST", "/api/update/check", Some(&cookie)).await;
            assert_eq!(status, StatusCode::BAD_GATEWAY);
            assert_eq!(body["error"]["code"], "update_check_failed");
            let (status, body) = send(&venue.app, "GET", "/api/update", Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["data"]["newer"], "0.2.0");
        }

        #[tokio::test]
        async fn a_second_check_waits() {
            let venue = desk();
            let cookie = admin_cookie(&venue.app);
            let hits = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&hits);
            let arrived = Arc::new(Notify::new());
            let release = Arc::new(Notify::new());
            let hold = ReleaseOnDrop(Arc::clone(&release));
            let arrived_for_route = Arc::clone(&arrived);
            let release_for_route = Arc::clone(&release);
            let app = Router::new().route(
                "/repos/{owner}/{repo}/releases/latest",
                get(move |request: Request| {
                    let recorded = Arc::clone(&recorded);
                    let arrived = Arc::clone(&arrived_for_route);
                    let release = Arc::clone(&release_for_route);
                    async move {
                        recorded
                            .lock()
                            .expect("hits")
                            .push(request.uri().path().to_string());
                        arrived.notify_one();
                        release.notified().await;
                        (
                            StatusCode::OK,
                            r#"{"tag_name":"v999.0.0","draft":false,"prerelease":false}"#,
                        )
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            venue.app.updates.set_api_base(&format!("http://{addr}"));
            let app_for_first = venue.app.clone();
            let cookie_for_first = cookie.clone();
            let first = tokio::spawn(async move {
                send(
                    &app_for_first,
                    "POST",
                    "/api/update/check",
                    Some(&cookie_for_first),
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(5), arrived.notified())
                .await
                .unwrap();
            let (status, body) = send(&venue.app, "POST", "/api/update/check", Some(&cookie)).await;
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(body["error"]["code"], "check_in_progress");
            assert_eq!(hits.lock().expect("hits").len(), 1);
            hold.0.notify_one();
            let (status, body) = tokio::time::timeout(Duration::from_secs(5), first)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["data"]["newer"], "999.0.0");
            assert_eq!(hits.lock().expect("hits").len(), 1);
        }

        #[tokio::test]
        async fn the_scala_update_paths_stay_unserved() {
            let (app, _worker) = App::new();
            for path in ["/updatecheck", "/version", "/update", "/need_update"] {
                let (status, _) = send(&app, "GET", path, None).await;
                assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            }
        }

        struct ReleaseOnDrop(Arc<Notify>);

        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                self.0.notify_one();
            }
        }
    }

    mod checksum {
        use std::path::PathBuf;

        use super::super::{verify, verify_asset};

        const ASSET: &[u8] = b"rust-reg-asset";
        const DIGEST: &str = "74df8ebebf381ce387cbafe365f4a8ac43d67ce1b3dbbe8b027abf94250b4ce4";
        const NAME: &str = "rust-reg-0.2.0.pkg";

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "reg-sum-{}-{}",
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

        #[test]
        fn the_digest_matches_the_named_line() {
            let sums = format!(
                "{DIGEST}  {NAME}\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  rust-reg-0.1.0.pkg\n"
            );
            assert!(verify(ASSET, &sums, NAME).is_ok());
        }

        #[test]
        fn a_mismatch_deletes_the_file() {
            let dir = TempDir::new();
            let updates = dir.0.join("updates");
            std::fs::create_dir_all(&updates).unwrap();
            std::fs::write(updates.join("live"), "0.1.0\n").unwrap();
            let asset = dir.0.join(NAME);
            std::fs::write(&asset, b"not-the-asset").unwrap();
            let sums = format!("{DIGEST}  {NAME}\n");
            let err = verify_asset(&asset, &sums, NAME).unwrap_err();
            assert_eq!(err.code(), "checksum_mismatch");
            assert!(!asset.exists());
            assert_eq!(
                std::fs::read_to_string(updates.join("live")).unwrap(),
                "0.1.0\n"
            );
        }

        #[test]
        fn a_broken_sums_line_is_a_mismatch() {
            let one_space = format!("{DIGEST} {NAME}\n");
            assert_eq!(
                verify(ASSET, &one_space, NAME).unwrap_err().code(),
                "checksum_mismatch"
            );
            let absent = format!("{DIGEST}  other.pkg\n");
            assert_eq!(
                verify(ASSET, &absent, NAME).unwrap_err().code(),
                "checksum_mismatch"
            );
        }
    }

    mod download {
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use axum::extract::Request;
        use axum::routing::get;
        use axum::Router;
        use sha2::{Digest, Sha256};
        use tokio::net::TcpListener;

        use crate::registration_server::gate::Gate;

        use super::super::{
            asset_name, asset_temp_path, download_release, github_client, UPDATE_REPO,
        };

        const VERSION: &str = "0.2.0";
        const BYTES: &[u8] = b"rust-reg-asset";
        const DIGEST: &str = "74df8ebebf381ce387cbafe365f4a8ac43d67ce1b3dbbe8b027abf94250b4ce4";

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "reg-download-{}-{}",
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

        fn hex(bytes: &[u8]) -> String {
            let digest = Sha256::digest(bytes);
            let mut out = String::with_capacity(digest.len() * 2);
            for byte in digest {
                out.push_str(&format!("{byte:02x}"));
            }
            out
        }

        async fn stand_in(sums: String) -> (String, Arc<Mutex<Vec<String>>>) {
            let hits = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&hits);
            let sums = Arc::new(sums);
            let app = Router::new().route(
                "/repos/{owner}/{repo}/releases/download/{tag}/{file}",
                get(move |request: Request| {
                    let recorded = Arc::clone(&recorded);
                    let sums = Arc::clone(&sums);
                    async move {
                        let path = request.uri().path().to_string();
                        recorded.lock().expect("hits").push(path.clone());
                        if path.ends_with("/SHA256SUMS") {
                            (axum::http::StatusCode::OK, sums.as_bytes().to_vec())
                        } else {
                            (axum::http::StatusCode::OK, BYTES.to_vec())
                        }
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            (format!("http://{addr}"), hits)
        }

        #[tokio::test]
        async fn the_asset_for_this_os_is_downloaded() {
            let name = asset_name(VERSION);
            let (base, hits) = stand_in(format!("{DIGEST}  {name}\n")).await;
            let dir = TempDir::new();
            let versions = dir.0.join("versions");
            std::fs::create_dir_all(&versions).unwrap();
            let loaded = download_release(
                &dir.0,
                &Arc::new(Gate::new()),
                None,
                &github_client(Duration::from_secs(5)),
                &base,
                UPDATE_REPO,
                VERSION,
                None,
            )
            .await
            .unwrap_or_else(|err| panic!("{}", err.code()));
            let seen = hits.lock().expect("hits");
            assert!(seen.iter().any(|path| path.contains(&name)));
            assert!(seen.iter().any(|path| path.ends_with("/SHA256SUMS")));
            drop(seen);
            let bytes = std::fs::read(&loaded.path).unwrap();
            assert_eq!(hex(&bytes), DIGEST);
            assert!(std::fs::read_dir(&versions).unwrap().next().is_none());
        }

        #[tokio::test]
        async fn the_download_waits_out_a_print() {
            let name = asset_name(VERSION);
            let (base, hits) = stand_in(format!("{DIGEST}  {name}\n")).await;
            let dir = TempDir::new();
            let gate = Arc::new(Gate::new());
            let guard = gate.enter_print();
            let waiting = {
                let gate = Arc::clone(&gate);
                let base = base.clone();
                let prefix = dir.0.clone();
                tokio::spawn(async move {
                    download_release(
                        &prefix,
                        &gate,
                        None,
                        &github_client(Duration::from_secs(5)),
                        &base,
                        UPDATE_REPO,
                        VERSION,
                        None,
                    )
                    .await
                })
            };
            tokio::task::yield_now().await;
            assert!(hits.lock().expect("hits").is_empty());
            assert!(!waiting.is_finished());
            drop(guard);
            waiting
                .await
                .unwrap()
                .unwrap_or_else(|err| panic!("{}", err.code()));
            let seen = hits.lock().expect("hits");
            assert!(seen.iter().any(|path| path.contains(&name)));
            assert!(seen.iter().any(|path| path.ends_with("/SHA256SUMS")));
        }

        #[tokio::test]
        async fn a_bad_digest_leaves_no_file() {
            let name = asset_name(VERSION);
            let wrong = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
            let (base, _) = stand_in(format!("{wrong}  {name}\n")).await;
            let dir = TempDir::new();
            let updates = dir.0.join("updates");
            std::fs::create_dir_all(&updates).unwrap();
            std::fs::write(updates.join("live"), "0.1.0\n").unwrap();
            let err = download_release(
                &dir.0,
                &Arc::new(Gate::new()),
                None,
                &github_client(Duration::from_secs(5)),
                &base,
                UPDATE_REPO,
                VERSION,
                None,
            )
            .await
            .err()
            .unwrap_or_else(|| panic!("download succeeded"));
            assert_eq!(err.code(), "checksum_mismatch");
            assert!(!asset_temp_path(&dir.0, VERSION).exists());
            assert_eq!(
                std::fs::read_to_string(updates.join("live")).unwrap(),
                "0.1.0\n"
            );
        }
    }

    mod unpack {
        use std::path::PathBuf;

        use super::super::{unpack, version_binary_name};

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "reg-unpack-{}-{}",
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

        fn write_payload(dir: &std::path::Path, release: bool) -> PathBuf {
            let payload = dir.join("payload");
            std::fs::create_dir_all(payload.join("admin")).unwrap();
            std::fs::create_dir_all(payload.join("form").join("assets")).unwrap();
            std::fs::write(payload.join("admin").join("index.html"), b"admin").unwrap();
            std::fs::write(payload.join("form").join("index.html"), b"form").unwrap();
            std::fs::write(payload.join("form").join("assets").join("app.js"), b"js").unwrap();
            std::fs::write(payload.join(version_binary_name()), b"binary").unwrap();
            if release {
                std::fs::write(payload.join("RELEASE"), b"v0.2.0\n").unwrap();
            }
            payload
        }

        #[test]
        fn the_rename_publishes_the_tree() {
            let dir = TempDir::new();
            let payload = write_payload(&dir.0, true);
            unpack(&dir.0, "0.2.0", &payload).unwrap();
            let tree = dir.0.join("versions").join("0.2.0");
            assert_eq!(
                std::fs::read(tree.join(version_binary_name())).unwrap(),
                b"binary"
            );
            assert!(tree.join("admin").is_dir());
            assert!(tree.join("form").is_dir());
            assert_eq!(
                std::fs::read(tree.join("admin").join("index.html")).unwrap(),
                b"admin"
            );
            assert_eq!(
                std::fs::read(tree.join("form").join("assets").join("app.js")).unwrap(),
                b"js"
            );
            assert_eq!(
                std::fs::read_to_string(tree.join("RELEASE")).unwrap(),
                "v0.2.0\n"
            );
            assert!(!dir.0.join("versions").join("0.2.0.partial").exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(tree.join(version_binary_name()))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_ne!(mode & 0o111, 0, "the unpacked server must be executable");
            }
        }

        #[test]
        fn an_existing_tree_is_replaced() {
            let dir = TempDir::new();
            let tree = dir.0.join("versions").join("0.2.0");
            std::fs::create_dir_all(&tree).unwrap();
            std::fs::write(tree.join("stale.txt"), b"old").unwrap();
            std::fs::write(tree.join(version_binary_name()), b"old-binary").unwrap();
            let payload = write_payload(&dir.0, true);
            unpack(&dir.0, "0.2.0", &payload).unwrap();
            assert_eq!(
                std::fs::read(tree.join(version_binary_name())).unwrap(),
                b"binary"
            );
            assert!(!tree.join("stale.txt").exists());
            assert!(!dir.0.join("versions").join("0.2.0.partial").exists());
        }

        #[test]
        fn a_failed_extract_leaves_the_partial() {
            let dir = TempDir::new();
            let payload = write_payload(&dir.0, false);
            let updates = dir.0.join("updates");
            std::fs::create_dir_all(&updates).unwrap();
            std::fs::write(updates.join("live"), "0.1.0\n").unwrap();
            assert!(unpack(&dir.0, "0.2.0", &payload).is_err());
            assert!(!dir.0.join("versions").join("0.2.0").exists());
            assert!(!dir.0.join("versions").join("0.2.0.partial").exists());
            assert_eq!(
                std::fs::read_to_string(updates.join("live")).unwrap(),
                "0.1.0\n"
            );
        }

        #[test]
        fn data_stays_put() {
            let dir = TempDir::new();
            let payload = write_payload(&dir.0, true);
            let credentials = b"device-id: desk\n";
            let database = b"sqlite-bytes";
            let supervisor = b"supervisor-bytes";
            let cred = dir.0.join("data").join("credentials.yml");
            let db = dir.0.join("data").join("EVT").join("db.sqlite");
            let bin = dir.0.join("bin").join("rust-reg-run");
            std::fs::create_dir_all(cred.parent().unwrap()).unwrap();
            std::fs::create_dir_all(db.parent().unwrap()).unwrap();
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(&cred, credentials).unwrap();
            std::fs::write(&db, database).unwrap();
            std::fs::write(&bin, supervisor).unwrap();
            unpack(&dir.0, "0.2.0", &payload).unwrap();
            assert_eq!(std::fs::read(&cred).unwrap(), credentials);
            assert_eq!(std::fs::read(&db).unwrap(), database);
            assert_eq!(std::fs::read(&bin).unwrap(), supervisor);
        }
    }

    mod stage {
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};

        use axum::body::{to_bytes, Body};
        use axum::extract::Request;
        use axum::http::StatusCode;
        use axum::routing::get;
        use axum::Router;
        use serde_json::Value;
        use tokio::net::TcpListener;
        use tower::ServiceExt;

        use crate::registration_server::credentials::{CredentialFile, Credentials};
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::print_queue::PrintJob;
        use crate::registration_server::App;

        use super::super::{asset_name, release_version, sha256_hex, version_binary_name};

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "reg-stage-{}-{}",
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

        struct Desk {
            _dir: TempDir,
            _file: CredentialFile,
            _worker: crate::registration_server::print_queue::PrintWorker,
            app: App,
        }

        fn desk() -> Desk {
            let dir = TempDir::new();
            let file = CredentialFile::new(dir.0.join("credentials.yml"));
            let mut credentials = Credentials::default();
            credentials.set_device_id("device-1").unwrap();
            credentials
                .bind("EVT", "token-value", Some("Event".into()))
                .unwrap();
            file.store(&credentials).unwrap();
            let current = Arc::new(Mutex::new(CurrentEvent::new(&dir.0)));
            CurrentEvent::lock(&current).open_bound(&file).unwrap();
            let (mut app, worker) = App::new();
            app.current = current;
            Desk {
                _dir: dir,
                _file: file,
                _worker: worker,
                app,
            }
        }

        fn operator_cookie(app: &App) -> String {
            let session = crate::registration_server::OperatorSession::resume(
                "http://127.0.0.1:9/",
                "device-1",
                "jsess",
            )
            .expect("session");
            app.operator
                .keep_session("admin-session".to_string(), session);
            "operator=admin-session".to_string()
        }

        async fn send(app: &App, cookie: Option<&str>) -> (StatusCode, Value) {
            let mut builder = Request::builder()
                .method("POST")
                .uri("/api/update")
                .header("content-type", "application/json");
            if let Some(cookie) = cookie {
                builder = builder.header("cookie", cookie);
            }
            let response = crate::registration_server::http::router(app.clone())
                .oneshot(builder.body(Body::from("{}")).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            (status, body)
        }

        fn archive() -> Vec<u8> {
            super::super::payload_zip(&[
                (version_binary_name(), b"new-server"),
                ("admin/index.html", b"admin"),
                ("form/index.html", b"form"),
                ("RELEASE", b"v0.2.0\n"),
            ])
        }

        async fn stand_in(asset: Vec<u8>, sums: String) -> (String, Arc<Mutex<Vec<String>>>) {
            let hits = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&hits);
            let asset = Arc::new(asset);
            let sums = Arc::new(sums);
            let app = Router::new().route(
                "/repos/{owner}/{repo}/releases/download/{tag}/{file}",
                get(move |request: Request| {
                    let recorded = Arc::clone(&recorded);
                    let asset = Arc::clone(&asset);
                    let sums = Arc::clone(&sums);
                    async move {
                        let path = request.uri().path().to_string();
                        recorded.lock().expect("hits").push(path.clone());
                        if path.ends_with("/SHA256SUMS") {
                            sums.as_bytes().to_vec()
                        } else {
                            asset.as_ref().clone()
                        }
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            (format!("http://{addr}"), hits)
        }

        fn plant(prefix: &PathBuf) {
            let updates = prefix.join("updates");
            std::fs::create_dir_all(&updates).unwrap();
            std::fs::write(updates.join("live"), "0.1.0\n").unwrap();
            std::fs::write(updates.join("started"), "open\n").unwrap();
            let credentials = prefix.join("data").join("credentials.yml");
            std::fs::create_dir_all(credentials.parent().unwrap()).unwrap();
            std::fs::write(&credentials, b"device-id: desk\n").unwrap();
        }

        #[tokio::test]
        async fn a_queued_job_is_print_busy() {
            let mut venue = desk();
            let prefix = TempDir::new();
            venue.app.prefix = prefix.0.clone();
            plant(&prefix.0);
            let (base, hits) = stand_in(archive(), "unused\n".into()).await;
            venue.app.updates.set_api_base(&base);
            venue.app.updates.store_offer(Some("0.2.0".into()));
            venue
                .app
                .prints
                .enqueue(PrintJob {
                    id: "job-1".into(),
                    pages: 1,
                    zone: None,
                    name: None,
                    certificate: None,
                    category: None,
                })
                .unwrap();
            let cookie = operator_cookie(&venue.app);
            let (status, body) = send(&venue.app, Some(&cookie)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["error"]["code"], "print_busy");
            assert!(hits.lock().expect("hits").is_empty());
            assert!(!venue
                .app
                .exit_requested
                .load(std::sync::atomic::Ordering::Acquire));
            assert_eq!(
                std::fs::read_to_string(prefix.0.join("updates").join("live")).unwrap(),
                "0.1.0\n"
            );
        }

        #[tokio::test]
        async fn a_running_print_is_print_busy() {
            let mut venue = desk();
            let prefix = TempDir::new();
            venue.app.prefix = prefix.0.clone();
            plant(&prefix.0);
            let (base, hits) = stand_in(archive(), "unused\n".into()).await;
            venue.app.updates.set_api_base(&base);
            venue.app.updates.store_offer(Some("0.2.0".into()));
            let printing = venue.app.gate.enter_print();
            let cookie = operator_cookie(&venue.app);
            let (status, body) = send(&venue.app, Some(&cookie)).await;
            drop(printing);
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["error"]["code"], "print_busy");
            assert!(hits.lock().expect("hits").is_empty());
            assert!(!venue
                .app
                .exit_requested
                .load(std::sync::atomic::Ordering::Acquire));
        }

        #[tokio::test]
        async fn an_unsupervised_server_refuses_the_install() {
            let mut venue = desk();
            venue.app.supervised = false;
            venue.app.updates.store_offer(Some("0.2.0".into()));
            let cookie = operator_cookie(&venue.app);
            let (status, body) = send(&venue.app, Some(&cookie)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["error"]["code"], "no_supervisor");
            assert!(!venue
                .app
                .exit_requested
                .load(std::sync::atomic::Ordering::Acquire));
            let doc = super::super::update_document(&venue.app);
            assert_eq!(doc["newer"], "0.2.0");
            assert_eq!(doc["supervised"], false);
            assert_eq!(doc["version"], release_version());
        }

        #[tokio::test]
        async fn stage_keeps_live_and_exits() {
            let bytes = archive();
            let name = asset_name("0.2.0");
            let sums = format!("{}  {name}\n", sha256_hex(&bytes));
            let (base, _hits) = stand_in(bytes.clone(), sums).await;
            let mut venue = desk();
            let prefix = TempDir::new();
            venue.app.prefix = prefix.0.clone();
            plant(&prefix.0);
            venue.app.updates.set_api_base(&base);
            venue.app.updates.store_offer(Some("0.2.0".into()));
            let cookie = operator_cookie(&venue.app);
            let (status, body) = send(&venue.app, Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["ok"], true);
            assert_eq!(body["data"]["version"], "0.2.0");
            let state: Value = serde_yaml::from_str(
                &std::fs::read_to_string(prefix.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"], "staged");
            assert_eq!(state["from"], release_version());
            assert_eq!(state["to"], "0.2.0");
            assert_eq!(
                std::fs::read_to_string(prefix.0.join("updates").join("live")).unwrap(),
                "0.1.0\n"
            );
            assert!(!prefix.0.join("updates").join("started").exists());
            assert_eq!(
                std::fs::read(
                    prefix
                        .0
                        .join("versions")
                        .join("0.2.0")
                        .join(version_binary_name())
                )
                .unwrap(),
                b"new-server"
            );
            assert!(venue
                .app
                .exit_requested
                .load(std::sync::atomic::Ordering::Acquire));
            assert_eq!(
                std::fs::read(prefix.0.join("data").join("credentials.yml")).unwrap(),
                b"device-id: desk\n"
            );
        }
    }

    mod supervisor {
        use std::path::{Path, PathBuf};
        use std::process::Command;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        use super::super::{resume, supervise};

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::AtomicU64;
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "reg-supervisor-{}-{}",
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

        fn write_phase(dir: &Path, phase: &str, from: &str, to: &str, rejected: &[&str]) {
            let rejected = if rejected.is_empty() {
                "rejected: []\n".to_string()
            } else {
                let lines = rejected
                    .iter()
                    .map(|version| format!("  - \"{version}\"\n"))
                    .collect::<String>();
                format!("rejected:\n{lines}")
            };
            let text = format!("phase: {phase}\nfrom: \"{from}\"\nto: \"{to}\"\n{rejected}");
            std::fs::create_dir_all(dir.join("updates")).unwrap();
            std::fs::write(dir.join("updates").join("state.yml"), text).unwrap();
        }

        fn plant_tree(dir: &Path, version: &str, bytes: &[u8]) {
            let tree = dir.join("versions").join(version);
            std::fs::create_dir_all(&tree).unwrap();
            std::fs::write(tree.join("marker"), bytes).unwrap();
        }

        fn idle_child() -> std::io::Result<std::process::Child> {
            #[cfg(unix)]
            {
                Command::new("/bin/sleep").arg("30").spawn()
            }
            #[cfg(windows)]
            {
                Command::new("ping").args(["-n", "30", "127.0.0.1"]).spawn()
            }
        }

        #[test]
        fn staged_points_live_at_the_new_tree() {
            let dir = TempDir::new();
            write_phase(&dir.0, "staged", "0.1.0", "0.2.0", &[]);
            std::fs::write(dir.0.join("updates").join("live"), "0.1.0\n").unwrap();
            plant_tree(&dir.0, "0.2.0", b"new");
            std::fs::write(dir.0.join("updates").join("started"), "stale\n").unwrap();
            let mut started = None;
            resume(&dir.0, |path| {
                let state: serde_yaml::Value = serde_yaml::from_str(
                    &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
                )
                .unwrap();
                assert_eq!(state["phase"].as_str(), Some("trying"));
                started = Some(path.to_path_buf());
            })
            .unwrap();
            assert_eq!(started.unwrap(), dir.0.join("versions").join("0.2.0"));
            assert_eq!(
                std::fs::read_to_string(dir.0.join("updates").join("live")).unwrap(),
                "0.2.0\n"
            );
            assert!(!dir.0.join("updates").join("started").exists());
        }

        #[test]
        fn staged_uses_the_live_tree_when_the_previous_directory_is_empty() {
            let dir = TempDir::new();
            write_phase(&dir.0, "staged", "0.1.0", "0.2.0", &[]);
            std::fs::write(dir.0.join("updates").join("live"), "0.0.9\n").unwrap();
            std::fs::create_dir_all(dir.0.join("versions").join("0.1.0")).unwrap();
            plant_tree(&dir.0, "0.0.9", b"current-tree");
            plant_tree(&dir.0, "0.2.0", b"new");
            resume(&dir.0, |_| {}).unwrap();
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("trying"));
            assert_eq!(state["from"].as_str(), Some("0.0.9"));
            assert_eq!(
                std::fs::read_to_string(dir.0.join("updates").join("live")).unwrap(),
                "0.2.0\n"
            );
            assert_eq!(
                std::fs::read(dir.0.join("versions").join("0.0.9").join("marker")).unwrap(),
                b"current-tree"
            );
        }

        fn exiting_child() -> std::io::Result<std::process::Child> {
            #[cfg(unix)]
            {
                Command::new("true").spawn()
            }
            #[cfg(windows)]
            {
                Command::new("cmd").args(["/c", "exit", "0"]).spawn()
            }
        }

        fn assert_rolled_back(dir: &Path) {
            assert_eq!(
                std::fs::read_to_string(dir.join("updates").join("live")).unwrap(),
                "0.1.0\n"
            );
            assert!(!dir.join("versions").join("0.2.0").exists());
            assert_eq!(
                std::fs::read(dir.join("versions").join("0.1.0").join("marker")).unwrap(),
                b"from-tree"
            );
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("rolled_back"));
            let rejected = state["rejected"]
                .as_sequence()
                .unwrap()
                .iter()
                .map(|item| item.as_str().unwrap().to_string())
                .collect::<Vec<_>>();
            assert!(rejected.iter().any(|version| version == "0.2.0"));
            assert!(rejected.iter().any(|version| version == "0.0.9"));
            assert_eq!(super::super::failed_upgrade(dir).as_deref(), Some("0.2.0"));
        }

        #[test]
        fn trying_without_started_rolls_back() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0", &["0.0.9"]);
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            plant_tree(&dir.0, "0.1.0", b"from-tree");
            plant_tree(&dir.0, "0.2.0", b"new-tree");
            let seen = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let prefix = dir.0.clone();
            let seen_child = Arc::clone(&seen);
            let stop_child = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                supervise(
                    &prefix,
                    move |path| {
                        seen_child.lock().expect("path").push(path.to_path_buf());
                        if path.ends_with("0.2.0") {
                            exiting_child()
                        } else {
                            idle_child()
                        }
                    },
                    &stop_child,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let restored = seen
                    .lock()
                    .expect("path")
                    .iter()
                    .any(|path| path.ends_with("0.1.0"));
                if restored {
                    break;
                }
                assert!(Instant::now() < deadline, "previous version did not start");
                std::thread::sleep(Duration::from_millis(5));
            }
            stop.store(true, Ordering::Release);
            handle.join().unwrap().unwrap();
            assert_rolled_back(&dir.0);
        }

        #[test]
        fn a_failed_spawn_while_trying_rolls_back() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0", &["0.0.9"]);
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            plant_tree(&dir.0, "0.1.0", b"from-tree");
            plant_tree(&dir.0, "0.2.0", b"new-tree");
            let seen = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let prefix = dir.0.clone();
            let seen_child = Arc::clone(&seen);
            let stop_child = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                supervise(
                    &prefix,
                    move |path| {
                        seen_child.lock().expect("path").push(path.to_path_buf());
                        if path.ends_with("0.2.0") {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::PermissionDenied,
                                "not executable",
                            ))
                        } else {
                            idle_child()
                        }
                    },
                    &stop_child,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let restored = seen
                    .lock()
                    .expect("path")
                    .iter()
                    .any(|path| path.ends_with("0.1.0"));
                if restored {
                    break;
                }
                assert!(Instant::now() < deadline, "previous version did not start");
                std::thread::sleep(Duration::from_millis(5));
            }
            stop.store(true, Ordering::Release);
            handle.join().unwrap().unwrap();
            assert_rolled_back(&dir.0);
        }

        #[test]
        fn an_outside_binary_is_backed_up_before_the_new_version_starts() {
            let dir = TempDir::new();
            let running = dir.0.join("elsewhere").join("rust-reg");
            std::fs::create_dir_all(running.parent().unwrap()).unwrap();
            std::fs::write(&running, b"old-bin").unwrap();
            std::fs::create_dir_all(dir.0.join("updates")).unwrap();
            std::fs::write(
                dir.0.join("updates").join("state.yml"),
                format!(
                    "phase: staged\nfrom: \"0.1.0\"\nto: \"0.2.0\"\nrejected: []\nrunning: \"{}\"\n",
                    running.display()
                ),
            )
            .unwrap();
            std::fs::write(dir.0.join("updates").join("live"), "0.1.0\n").unwrap();
            plant_tree(&dir.0, "0.2.0", b"new");
            resume(&dir.0, |_| {}).unwrap();
            let saved = std::fs::read_dir(dir.0.join("backup"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .find(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("0.1.0-"))
                })
                .expect("backup directory");
            assert_eq!(
                std::fs::read(saved.path().join("rust-reg")).unwrap(),
                b"old-bin"
            );
            assert_eq!(std::fs::read(&running).unwrap(), b"old-bin");
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("trying"));
        }

        #[test]
        fn rollback_restores_the_backed_up_binary() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0", &["0.0.9"]);
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            let saved = dir.0.join("backup").join("0.1.0-20261024");
            std::fs::create_dir_all(&saved).unwrap();
            std::fs::write(saved.join("rust-reg"), b"old-bin").unwrap();
            plant_tree(&dir.0, "0.2.0", b"new-tree");
            let seen = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let prefix = dir.0.clone();
            let seen_child = Arc::clone(&seen);
            let stop_child = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                supervise(
                    &prefix,
                    move |path| {
                        seen_child.lock().expect("path").push(path.to_path_buf());
                        if path.ends_with("0.2.0") {
                            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
                        } else {
                            idle_child()
                        }
                    },
                    &stop_child,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let restored = seen
                    .lock()
                    .expect("path")
                    .iter()
                    .any(|path| path.ends_with("0.1.0"));
                if restored {
                    break;
                }
                assert!(Instant::now() < deadline, "backed up version did not start");
                std::thread::sleep(Duration::from_millis(5));
            }
            stop.store(true, Ordering::Release);
            handle.join().unwrap().unwrap();
            assert_eq!(
                std::fs::read(dir.0.join("versions").join("0.1.0").join("rust-reg")).unwrap(),
                b"old-bin"
            );
            assert_eq!(std::fs::read(saved.join("rust-reg")).unwrap(), b"old-bin");
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("rolled_back"));
            assert_eq!(
                std::fs::read_to_string(dir.0.join("updates").join("live")).unwrap(),
                "0.1.0\n"
            );
        }

        fn version_binary() -> &'static str {
            super::super::version_binary_name()
        }

        fn run_until(
            dir: &Path,
            stop_when: impl Fn(&[PathBuf]) -> bool + Send + 'static,
            spawn_at: impl Fn(&Path) -> std::io::Result<std::process::Child> + Send + 'static,
        ) -> std::io::Result<()> {
            let seen = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let prefix = dir.to_path_buf();
            let seen_child = Arc::clone(&seen);
            let stop_child = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                supervise(
                    &prefix,
                    move |path| {
                        seen_child.lock().expect("path").push(path.to_path_buf());
                        spawn_at(path)
                    },
                    &stop_child,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if stop_when(&seen.lock().expect("path")) {
                    stop.store(true, Ordering::Release);
                    return handle.join().unwrap();
                }
                if handle.is_finished() {
                    return handle.join().unwrap();
                }
                assert!(
                    Instant::now() < deadline,
                    "supervisor did not reach the expected version"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        #[test]
        fn a_failed_upgrade_restores_the_outside_binary() {
            let dir = TempDir::new();
            let home = dir.0.join("elsewhere");
            let binary = version_binary();
            std::fs::create_dir_all(home.join("admin")).unwrap();
            std::fs::create_dir_all(home.join("form")).unwrap();
            std::fs::write(home.join(binary), b"old-bin").unwrap();
            std::fs::write(home.join("admin").join("index.html"), b"admin").unwrap();
            std::fs::write(home.join("form").join("index.html"), b"form").unwrap();
            std::fs::write(home.join("other.bin"), b"not-a-version-file").unwrap();
            std::fs::create_dir_all(dir.0.join("updates")).unwrap();
            std::fs::write(
                dir.0.join("updates").join("state.yml"),
                format!(
                    "phase: staged\nfrom: \"0.1.0\"\nto: \"0.2.0\"\nrejected: []\nrunning: \"{}\"\n",
                    home.join(binary).display()
                ),
            )
            .unwrap();
            std::fs::write(dir.0.join("updates").join("live"), "0.1.0\n").unwrap();
            plant_tree(&dir.0, "0.2.0", b"new");
            run_until(
                &dir.0,
                |paths| paths.iter().any(|path| path.ends_with("0.1.0")),
                |path| {
                    if path.ends_with("0.2.0") {
                        exiting_child()
                    } else {
                        idle_child()
                    }
                },
            )
            .unwrap();
            let restored = dir.0.join("versions").join("0.1.0");
            assert_eq!(std::fs::read(restored.join(binary)).unwrap(), b"old-bin");
            assert_eq!(
                std::fs::read(restored.join("admin").join("index.html")).unwrap(),
                b"admin"
            );
            assert_eq!(
                std::fs::read(restored.join("form").join("index.html")).unwrap(),
                b"form"
            );
            assert!(!restored.join("other.bin").exists());
            assert_eq!(std::fs::read(home.join(binary)).unwrap(), b"old-bin");
            assert_eq!(
                std::fs::read(home.join("admin").join("index.html")).unwrap(),
                b"admin"
            );
            let saved = std::fs::read_dir(dir.0.join("backup"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .find(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("0.1.0-"))
                })
                .expect("backup");
            assert_eq!(
                std::fs::read(saved.path().join(binary)).unwrap(),
                b"old-bin"
            );
            assert!(!dir.0.join("versions").join("0.2.0").exists());
            assert_eq!(
                std::fs::read_to_string(dir.0.join("updates").join("live")).unwrap(),
                "0.1.0\n"
            );
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("rolled_back"));
            assert_eq!(
                super::super::failed_upgrade(&dir.0).as_deref(),
                Some("0.2.0")
            );
        }

        #[test]
        fn a_bind_keeps_the_new_version_and_moves_the_old_tree() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0", &[]);
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            plant_tree(&dir.0, "0.1.0", b"from-tree");
            plant_tree(&dir.0, "0.2.0", b"new-tree");
            let prefix = dir.0.clone();
            let starts = Arc::new(Mutex::new(0u32));
            let starts_child = Arc::clone(&starts);
            run_until(
                &dir.0,
                |paths| paths.iter().filter(|path| path.ends_with("0.2.0")).count() >= 2,
                move |path| {
                    assert!(path.ends_with("0.2.0"));
                    let mut count = starts_child.lock().expect("starts");
                    *count += 1;
                    if *count == 1 {
                        std::fs::write(prefix.join("updates").join("started"), "\n").unwrap();
                        exiting_child()
                    } else {
                        idle_child()
                    }
                },
            )
            .unwrap();
            assert!(!dir.0.join("versions").join("0.1.0").exists());
            assert_eq!(
                std::fs::read(dir.0.join("versions").join("0.2.0").join("marker")).unwrap(),
                b"new-tree"
            );
            let saved = std::fs::read_dir(dir.0.join("backup"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .find(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("0.1.0-"))
                })
                .expect("backup of the previous tree");
            assert_eq!(
                std::fs::read(saved.path().join("marker")).unwrap(),
                b"from-tree"
            );
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("current"));
            assert_eq!(
                std::fs::read_to_string(dir.0.join("updates").join("live")).unwrap(),
                "0.2.0\n"
            );
        }

        #[test]
        fn the_newer_backup_is_the_one_restored() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0", &[]);
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            let older = dir.0.join("backup").join("0.1.0-20261001");
            let newer = dir.0.join("backup").join("0.1.0-20261024");
            std::fs::create_dir_all(&older).unwrap();
            std::fs::create_dir_all(&newer).unwrap();
            std::fs::write(older.join(version_binary()), b"older-bin").unwrap();
            std::fs::write(newer.join(version_binary()), b"newer-bin").unwrap();
            run_until(
                &dir.0,
                |paths| paths.iter().any(|path| path.ends_with("0.1.0")),
                |path| {
                    if path.ends_with("0.2.0") {
                        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
                    } else {
                        idle_child()
                    }
                },
            )
            .unwrap();
            assert_eq!(
                std::fs::read(dir.0.join("versions").join("0.1.0").join(version_binary())).unwrap(),
                b"newer-bin"
            );
            assert_eq!(
                std::fs::read(older.join(version_binary())).unwrap(),
                b"older-bin"
            );
            assert_eq!(
                std::fs::read(newer.join(version_binary())).unwrap(),
                b"newer-bin"
            );
        }

        #[test]
        fn an_installed_tree_is_not_replaced_by_its_backup() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0", &[]);
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            let installed = dir.0.join("versions").join("0.1.0");
            std::fs::create_dir_all(&installed).unwrap();
            std::fs::write(installed.join(version_binary()), b"kept").unwrap();
            let saved = dir.0.join("backup").join("0.1.0-20261024");
            std::fs::create_dir_all(&saved).unwrap();
            std::fs::write(saved.join(version_binary()), b"older-bin").unwrap();
            run_until(
                &dir.0,
                |paths| paths.iter().any(|path| path.ends_with("0.1.0")),
                |path| {
                    if path.ends_with("0.2.0") {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "not executable",
                        ))
                    } else {
                        idle_child()
                    }
                },
            )
            .unwrap();
            assert_eq!(
                std::fs::read(installed.join(version_binary())).unwrap(),
                b"kept"
            );
        }

        #[test]
        fn a_version_inside_the_prefix_is_not_copied_before_start() {
            let dir = TempDir::new();
            let binary = dir.0.join("versions").join("0.1.0").join(version_binary());
            std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
            std::fs::write(&binary, b"already-installed").unwrap();
            std::fs::create_dir_all(dir.0.join("updates")).unwrap();
            std::fs::write(
                dir.0.join("updates").join("state.yml"),
                format!(
                    "phase: staged\nfrom: \"0.1.0\"\nto: \"0.2.0\"\nrejected: []\nrunning: \"{}\"\n",
                    binary.display()
                ),
            )
            .unwrap();
            std::fs::write(dir.0.join("updates").join("live"), "0.1.0\n").unwrap();
            plant_tree(&dir.0, "0.2.0", b"new");
            resume(&dir.0, |_| {}).unwrap();
            assert!(!dir.0.join("backup").exists());
            assert_eq!(std::fs::read(&binary).unwrap(), b"already-installed");
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("trying"));
        }

        #[test]
        fn a_missing_tree_and_backup_records_the_rollback_and_stops() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0", &[]);
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            let error = run_until(
                &dir.0,
                |_| false,
                |_path| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing")),
            )
            .unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("rolled_back"));
            assert_eq!(
                super::super::failed_upgrade(&dir.0).as_deref(),
                Some("0.2.0")
            );
            assert_eq!(
                std::fs::read_to_string(dir.0.join("updates").join("live")).unwrap(),
                "0.1.0\n"
            );
            assert!(!dir
                .0
                .join("versions")
                .join("0.1.0")
                .join(version_binary())
                .exists());
        }

        #[test]
        fn static_files_from_the_previous_version_are_restored() {
            let dir = TempDir::new();
            let binary = version_binary();
            let installed = dir.0.join("versions").join("0.1.0");
            std::fs::create_dir_all(&installed).unwrap();
            std::fs::write(installed.join(binary), b"kept").unwrap();
            let screens = dir.0.join("screens");
            std::fs::create_dir_all(screens.join("admin").join("assets")).unwrap();
            std::fs::create_dir_all(screens.join("form")).unwrap();
            std::fs::write(screens.join("admin").join("index.html"), b"admin").unwrap();
            std::fs::write(screens.join("admin").join("assets").join("app.js"), b"js").unwrap();
            std::fs::write(screens.join("form").join("index.html"), b"form").unwrap();
            let home = dir.0.join("elsewhere");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(home.join(binary), b"old-bin").unwrap();
            std::fs::create_dir_all(dir.0.join("updates")).unwrap();
            std::fs::write(
                dir.0.join("updates").join("state.yml"),
                format!(
                    "phase: trying\nfrom: \"0.1.0\"\nto: \"0.2.0\"\nrejected: []\nrunning: \"{}\"\nadmin: \"{}\"\nform: \"{}\"\n",
                    home.join(binary).display(),
                    screens.join("admin").display(),
                    screens.join("form").display()
                ),
            )
            .unwrap();
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            run_until(
                &dir.0,
                |paths| paths.iter().any(|path| path.ends_with("0.1.0")),
                |path| {
                    if path.ends_with("0.2.0") {
                        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
                    } else {
                        idle_child()
                    }
                },
            )
            .unwrap();
            assert_eq!(std::fs::read(installed.join(binary)).unwrap(), b"kept");
            assert_eq!(
                std::fs::read(installed.join("admin").join("index.html")).unwrap(),
                b"admin"
            );
            assert_eq!(
                std::fs::read(installed.join("admin").join("assets").join("app.js")).unwrap(),
                b"js"
            );
            assert_eq!(
                std::fs::read(installed.join("form").join("index.html")).unwrap(),
                b"form"
            );
            assert_eq!(
                std::fs::read(screens.join("admin").join("assets").join("app.js")).unwrap(),
                b"js"
            );
            let saved = std::fs::read_dir(dir.0.join("backup"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .find(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("0.1.0-"))
                })
                .expect("backup");
            assert_eq!(
                std::fs::read(saved.path().join("admin").join("assets").join("app.js")).unwrap(),
                b"js"
            );
            assert_eq!(
                std::fs::read(saved.path().join("form").join("index.html")).unwrap(),
                b"form"
            );
            assert!(!dir.0.join("versions").join("0.2.0").exists());
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("rolled_back"));
        }

        #[test]
        fn the_from_directory_static_files_are_backed_up() {
            let dir = TempDir::new();
            let binary = version_binary();
            let installed = dir.0.join("versions").join("0.1.0");
            std::fs::create_dir_all(installed.join("admin").join("assets")).unwrap();
            std::fs::create_dir_all(installed.join("form")).unwrap();
            std::fs::write(installed.join("admin").join("index.html"), b"admin").unwrap();
            std::fs::write(installed.join("admin").join("assets").join("app.js"), b"js").unwrap();
            std::fs::write(installed.join("form").join("index.html"), b"form").unwrap();
            let home = dir.0.join("elsewhere");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(home.join(binary), b"old-bin").unwrap();
            std::fs::create_dir_all(dir.0.join("updates")).unwrap();
            std::fs::write(
                dir.0.join("updates").join("state.yml"),
                format!(
                    "phase: trying\nfrom: \"0.1.0\"\nto: \"0.2.0\"\nrejected: []\nrunning: \"{}\"\n",
                    home.join(binary).display()
                ),
            )
            .unwrap();
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            run_until(
                &dir.0,
                |paths| paths.iter().any(|path| path.ends_with("0.1.0")),
                |path| {
                    if path.ends_with("0.2.0") {
                        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
                    } else {
                        idle_child()
                    }
                },
            )
            .unwrap();
            assert_eq!(std::fs::read(installed.join(binary)).unwrap(), b"old-bin");
            assert_eq!(
                std::fs::read(installed.join("admin").join("assets").join("app.js")).unwrap(),
                b"js"
            );
            assert_eq!(
                std::fs::read(installed.join("form").join("index.html")).unwrap(),
                b"form"
            );
            let saved = std::fs::read_dir(dir.0.join("backup"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .find(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("0.1.0-"))
                })
                .expect("backup");
            assert_eq!(
                std::fs::read(saved.path().join("admin").join("index.html")).unwrap(),
                b"admin"
            );
            assert_eq!(
                std::fs::read(saved.path().join(binary)).unwrap(),
                b"old-bin"
            );
        }

        #[test]
        fn trying_with_started_keeps_the_new_tree() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0", &[]);
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            std::fs::write(dir.0.join("updates").join("started"), "\n").unwrap();
            plant_tree(&dir.0, "0.1.0", b"from-tree");
            plant_tree(&dir.0, "0.2.0", b"new-tree");
            let mut started = None;
            resume(&dir.0, |path| {
                started = Some(path.to_path_buf());
            })
            .unwrap();
            assert_eq!(started.unwrap(), dir.0.join("versions").join("0.2.0"));
            assert_eq!(
                std::fs::read(dir.0.join("versions").join("0.2.0").join("marker")).unwrap(),
                b"new-tree"
            );
        }

        #[test]
        fn current_starts_live() {
            let dir = TempDir::new();
            write_phase(&dir.0, "current", "0.1.0", "0.2.0", &[]);
            let before = std::fs::read(dir.0.join("updates").join("state.yml")).unwrap();
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            plant_tree(&dir.0, "0.2.0", b"live-tree");
            let mut started = None;
            resume(&dir.0, |path| {
                started = Some(path.to_path_buf());
            })
            .unwrap();
            assert_eq!(started.unwrap(), dir.0.join("versions").join("0.2.0"));
            assert_eq!(
                std::fs::read(dir.0.join("updates").join("state.yml")).unwrap(),
                before
            );
        }

        #[test]
        fn a_service_stop_leaves_the_phase() {
            let dir = TempDir::new();
            write_phase(&dir.0, "current", "0.1.0", "0.2.0", &[]);
            let before = std::fs::read(dir.0.join("updates").join("state.yml")).unwrap();
            std::fs::write(dir.0.join("updates").join("live"), "0.2.0\n").unwrap();
            plant_tree(&dir.0, "0.1.0", b"from-tree");
            plant_tree(&dir.0, "0.2.0", b"live-tree");
            let seen = Arc::new(Mutex::new(None));
            let started = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let prefix = dir.0.clone();
            let seen_child = Arc::clone(&seen);
            let started_child = Arc::clone(&started);
            let stop_child = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                supervise(
                    &prefix,
                    move |path| {
                        *seen_child.lock().expect("path") = Some(path.to_path_buf());
                        started_child.store(true, Ordering::Release);
                        idle_child()
                    },
                    &stop_child,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !started.load(Ordering::Acquire) {
                assert!(Instant::now() < deadline, "child did not start");
                std::thread::sleep(Duration::from_millis(5));
            }
            stop.store(true, Ordering::Release);
            handle.join().unwrap().unwrap();
            assert_eq!(
                seen.lock().expect("path").as_deref(),
                Some(dir.0.join("versions").join("0.2.0").as_path())
            );
            assert_eq!(
                std::fs::read(dir.0.join("updates").join("state.yml")).unwrap(),
                before
            );
            assert_eq!(
                std::fs::read(dir.0.join("versions").join("0.1.0").join("marker")).unwrap(),
                b"from-tree"
            );
            assert_eq!(
                std::fs::read(dir.0.join("versions").join("0.2.0").join("marker")).unwrap(),
                b"live-tree"
            );
        }
    }

    mod started {
        use std::net::SocketAddr;
        use std::path::{Path, PathBuf};

        use super::super::{note_listening_on, open_listen, settle, Day};

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "reg-started-{}-{}",
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

        fn write_phase(dir: &Path, phase: &str, from: &str, to: &str) {
            let text = format!("phase: {phase}\nfrom: \"{from}\"\nto: \"{to}\"\nrejected: []\n");
            std::fs::create_dir_all(dir.join("updates")).unwrap();
            std::fs::write(dir.join("updates").join("state.yml"), text).unwrap();
        }

        fn day() -> Day {
            Day {
                year: 2026,
                month: 10,
                day: 6,
            }
        }

        #[tokio::test]
        async fn the_open_socket_writes_started() {
            let dir = TempDir::new();
            let listener = open_listen("127.0.0.1:0".parse().unwrap(), Some(&dir.0))
                .await
                .unwrap();
            drop(listener);
            assert!(dir.0.join("updates").join("started").is_file());

            let dir = TempDir::new();
            let failed = open_listen(SocketAddr::from(([192, 0, 2, 1], 9)), Some(&dir.0)).await;
            assert!(failed.is_err());
            assert!(!dir.0.join("updates").join("started").exists());
        }

        #[test]
        fn trying_moves_the_old_tree() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0");
            let old = dir.0.join("versions").join("0.1.0");
            let new = dir.0.join("versions").join("0.2.0");
            std::fs::create_dir_all(&old).unwrap();
            std::fs::create_dir_all(&new).unwrap();
            std::fs::write(old.join("marker"), b"old-tree").unwrap();
            std::fs::write(new.join("marker"), b"new-tree").unwrap();
            settle(&dir.0, day()).unwrap();
            assert!(!old.exists());
            assert_eq!(
                std::fs::read(dir.0.join("backup").join("0.1.0-20261006").join("marker")).unwrap(),
                b"old-tree"
            );
            assert_eq!(std::fs::read(new.join("marker")).unwrap(), b"new-tree");
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("current"));
        }

        #[test]
        fn an_existing_backup_keeps_static_files_already_copied() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0");
            let old = dir.0.join("versions").join("0.1.0");
            let saved = dir.0.join("backup").join("0.1.0-20261006");
            std::fs::create_dir_all(old.join("admin")).unwrap();
            std::fs::create_dir_all(saved.join("admin")).unwrap();
            std::fs::write(old.join("rust-reg"), b"from-bin").unwrap();
            std::fs::write(old.join("admin").join("index.html"), b"from-admin").unwrap();
            std::fs::write(saved.join("admin").join("index.html"), b"backed-admin").unwrap();
            std::fs::write(saved.join("rust-reg"), b"backed-bin").unwrap();
            settle(&dir.0, day()).unwrap();
            assert!(!old.exists());
            assert_eq!(
                std::fs::read(saved.join("admin").join("index.html")).unwrap(),
                b"backed-admin"
            );
            assert_eq!(
                std::fs::read(saved.join("rust-reg")).unwrap(),
                b"backed-bin"
            );
        }

        #[test]
        fn a_missing_previous_tree_finalizes_without_a_backup() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0");
            let new = dir.0.join("versions").join("0.2.0");
            std::fs::create_dir_all(&new).unwrap();
            std::fs::write(new.join("marker"), b"new-tree").unwrap();
            settle(&dir.0, day()).unwrap();
            assert!(!dir.0.join("versions").join("0.1.0").exists());
            assert!(!dir.0.join("backup").exists());
            assert_eq!(std::fs::read(new.join("marker")).unwrap(), b"new-tree");
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("current"));
            assert_eq!(state["from"].as_str(), Some("0.1.0"));
        }

        #[test]
        fn an_empty_previous_tree_backs_up_the_live_version() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0");
            std::fs::create_dir_all(dir.0.join("versions").join("0.1.0")).unwrap();
            std::fs::write(dir.0.join("updates").join("live"), "0.0.9\n").unwrap();
            let current = dir.0.join("versions").join("0.0.9");
            let new = dir.0.join("versions").join("0.2.0");
            std::fs::create_dir_all(&current).unwrap();
            std::fs::create_dir_all(&new).unwrap();
            std::fs::write(current.join("marker"), b"current-tree").unwrap();
            std::fs::write(new.join("marker"), b"new-tree").unwrap();
            settle(&dir.0, day()).unwrap();
            assert!(!current.exists());
            assert!(dir.0.join("versions").join("0.1.0").is_dir());
            assert_eq!(
                std::fs::read(dir.0.join("backup").join("0.0.9-20261006").join("marker")).unwrap(),
                b"current-tree"
            );
            assert_eq!(std::fs::read(new.join("marker")).unwrap(), b"new-tree");
            let state: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(dir.0.join("updates").join("state.yml")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["phase"].as_str(), Some("current"));
            assert_eq!(state["from"].as_str(), Some("0.0.9"));
        }

        #[test]
        fn a_backup_older_than_seven_days_is_removed() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0");
            std::fs::create_dir_all(dir.0.join("versions").join("0.1.0")).unwrap();
            let gone = dir.0.join("backup").join("0.1.0-20260929");
            let stays = dir.0.join("backup").join("0.1.0-20260930");
            std::fs::create_dir_all(&gone).unwrap();
            std::fs::create_dir_all(&stays).unwrap();
            std::fs::write(gone.join("marker"), b"old").unwrap();
            std::fs::write(stays.join("marker"), b"keep").unwrap();
            settle(&dir.0, day()).unwrap();
            assert!(!gone.exists());
            assert_eq!(std::fs::read(stays.join("marker")).unwrap(), b"keep");
        }

        #[test]
        fn current_does_not_move_again() {
            let dir = TempDir::new();
            write_phase(&dir.0, "current", "0.1.0", "0.2.0");
            let tree = dir.0.join("versions").join("0.2.0");
            let backup = dir.0.join("backup").join("0.1.0-20261006");
            std::fs::create_dir_all(&tree).unwrap();
            std::fs::create_dir_all(&backup).unwrap();
            std::fs::write(tree.join("marker"), b"live-tree").unwrap();
            std::fs::write(backup.join("marker"), b"backed-up").unwrap();
            let credentials = dir.0.join("data").join("credentials.yml");
            std::fs::create_dir_all(credentials.parent().unwrap()).unwrap();
            std::fs::write(&credentials, b"device-id: desk\n").unwrap();
            let started = dir.0.join("updates").join("started");
            std::fs::write(&started, b"keep\n").unwrap();
            note_listening_on(&dir.0, day()).unwrap();
            assert_eq!(std::fs::read(tree.join("marker")).unwrap(), b"live-tree");
            assert_eq!(std::fs::read(backup.join("marker")).unwrap(), b"backed-up");
            assert_eq!(std::fs::read(&credentials).unwrap(), b"device-id: desk\n");
            assert_eq!(std::fs::read(&started).unwrap(), b"keep\n");
        }
    }

    mod wrapper {
        use std::path::{Path, PathBuf};

        use super::super::{replace_wrapper, unpack, version_binary_name, wrapper_binary_name};

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "reg-wrapper-{}-{}",
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

        fn write_phase(dir: &Path, phase: &str, from: &str, to: &str) {
            std::fs::create_dir_all(dir.join("updates")).unwrap();
            std::fs::write(
                dir.join("updates").join("state.yml"),
                format!("phase: {phase}\nfrom: \"{from}\"\nto: \"{to}\"\nrejected: []\n"),
            )
            .unwrap();
        }

        fn plant_wrapper(dir: &Path, version: &str, bytes: &[u8]) {
            let tree = dir.join("versions").join(version);
            std::fs::create_dir_all(&tree).unwrap();
            std::fs::write(tree.join(wrapper_binary_name()), bytes).unwrap();
        }

        fn phase_of(dir: &Path) -> String {
            let text = std::fs::read_to_string(dir.join("updates").join("state.yml")).unwrap();
            let state: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
            state["phase"].as_str().unwrap().to_string()
        }

        #[test]
        fn the_version_directory_contains_the_wrapper() {
            let dir = TempDir::new();
            let payload = dir.0.join("payload");
            std::fs::create_dir_all(payload.join("admin")).unwrap();
            std::fs::create_dir_all(payload.join("form")).unwrap();
            std::fs::write(payload.join("admin").join("index.html"), b"admin").unwrap();
            std::fs::write(payload.join("form").join("index.html"), b"form").unwrap();
            std::fs::write(payload.join(version_binary_name()), b"server").unwrap();
            std::fs::write(payload.join("RELEASE"), b"v0.2.0\n").unwrap();
            std::fs::write(payload.join(wrapper_binary_name()), b"wrapper-bytes").unwrap();
            let running = dir.0.join("opt").join(wrapper_binary_name());
            std::fs::create_dir_all(running.parent().unwrap()).unwrap();
            std::fs::write(&running, b"previous").unwrap();
            unpack(&dir.0, "0.2.0", &payload).unwrap();
            let installed = dir
                .0
                .join("versions")
                .join("0.2.0")
                .join(wrapper_binary_name());
            assert_eq!(std::fs::read(&installed).unwrap(), b"wrapper-bytes");
            assert_eq!(std::fs::read(&running).unwrap(), b"previous");
            assert!(!dir.0.join("bin").join(wrapper_binary_name()).exists());
        }

        #[test]
        fn current_replaces_the_wrapper() {
            let dir = TempDir::new();
            write_phase(&dir.0, "current", "0.1.0", "0.2.0");
            plant_wrapper(&dir.0, "0.2.0", b"new-wrapper");
            let detected = dir.0.join("opt").join(wrapper_binary_name());
            std::fs::create_dir_all(detected.parent().unwrap()).unwrap();
            std::fs::write(&detected, b"old-wrapper").unwrap();
            let service = dir.0.join("rust-reg.service");
            std::fs::write(&service, b"unit\n").unwrap();
            let credentials = dir.0.join("data").join("credentials.yml");
            let event = dir.0.join("data").join("EVT").join("db.sqlite");
            std::fs::create_dir_all(event.parent().unwrap()).unwrap();
            std::fs::write(&credentials, b"device-id: desk\n").unwrap();
            std::fs::write(&event, b"sqlite").unwrap();
            replace_wrapper(&dir.0, Some(&detected)).unwrap();
            assert_eq!(std::fs::read(&detected).unwrap(), b"new-wrapper");
            assert_eq!(
                std::fs::read(
                    dir.0
                        .join("versions")
                        .join("0.2.0")
                        .join(wrapper_binary_name())
                )
                .unwrap(),
                b"new-wrapper"
            );
            assert!(!dir.0.join("bin").join(wrapper_binary_name()).exists());
            assert_eq!(std::fs::read(&service).unwrap(), b"unit\n");
            assert_eq!(std::fs::read(&credentials).unwrap(), b"device-id: desk\n");
            assert_eq!(std::fs::read(&event).unwrap(), b"sqlite");
            assert_eq!(phase_of(&dir.0), "current");
        }

        #[test]
        fn the_same_sha_leaves_the_wrapper() {
            let dir = TempDir::new();
            write_phase(&dir.0, "current", "0.1.0", "0.2.0");
            plant_wrapper(&dir.0, "0.2.0", b"same-wrapper");
            let detected = dir.0.join("opt").join(wrapper_binary_name());
            std::fs::create_dir_all(detected.parent().unwrap()).unwrap();
            std::fs::write(&detected, b"same-wrapper").unwrap();
            #[cfg(unix)]
            let inode = {
                use std::os::unix::fs::MetadataExt;
                std::fs::metadata(&detected).unwrap().ino()
            };
            replace_wrapper(&dir.0, Some(&detected)).unwrap();
            assert_eq!(std::fs::read(&detected).unwrap(), b"same-wrapper");
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(std::fs::metadata(&detected).unwrap().ino(), inode);
            }
            assert!(!dir.0.join("bin").join(wrapper_binary_name()).exists());
        }

        #[test]
        fn trying_does_not_replace_the_wrapper() {
            let dir = TempDir::new();
            write_phase(&dir.0, "trying", "0.1.0", "0.2.0");
            plant_wrapper(&dir.0, "0.2.0", b"new-wrapper");
            let detected = dir.0.join("opt").join(wrapper_binary_name());
            std::fs::create_dir_all(detected.parent().unwrap()).unwrap();
            std::fs::write(&detected, b"installed-0.1.0").unwrap();
            replace_wrapper(&dir.0, Some(&detected)).unwrap();
            assert_eq!(std::fs::read(&detected).unwrap(), b"installed-0.1.0");
            assert_eq!(phase_of(&dir.0), "trying");
            assert!(!dir.0.join("bin").join(wrapper_binary_name()).exists());
        }

        #[test]
        fn a_failed_copy_leaves_the_old_wrapper() {
            let dir = TempDir::new();
            write_phase(&dir.0, "current", "0.1.0", "0.2.0");
            plant_wrapper(&dir.0, "0.2.0", b"new-wrapper");
            let detected = dir.0.join("opt").join(wrapper_binary_name());
            std::fs::create_dir_all(detected.parent().unwrap()).unwrap();
            std::fs::write(&detected, b"old-wrapper").unwrap();
            std::fs::create_dir(
                detected
                    .parent()
                    .unwrap()
                    .join(format!("{}.new", wrapper_binary_name())),
            )
            .unwrap();
            assert!(replace_wrapper(&dir.0, Some(&detected)).is_err());
            assert_eq!(std::fs::read(&detected).unwrap(), b"old-wrapper");
            assert_eq!(
                std::fs::read(
                    dir.0
                        .join("versions")
                        .join("0.2.0")
                        .join(wrapper_binary_name())
                )
                .unwrap(),
                b"new-wrapper"
            );
            assert_eq!(phase_of(&dir.0), "current");
        }

        #[test]
        fn a_missing_wrapper_path_writes_nothing() {
            let dir = TempDir::new();
            write_phase(&dir.0, "current", "0.1.0", "0.2.0");
            plant_wrapper(&dir.0, "0.2.0", b"new-wrapper");
            replace_wrapper(&dir.0, None).unwrap();
            assert!(!dir.0.join("bin").join(wrapper_binary_name()).exists());
            assert_eq!(phase_of(&dir.0), "current");
            assert_eq!(
                std::fs::read(
                    dir.0
                        .join("versions")
                        .join("0.2.0")
                        .join(wrapper_binary_name())
                )
                .unwrap(),
                b"new-wrapper"
            );
        }
    }
}
