//! Credential file for the venue device.
//!
//! The live file is replaced by renaming a fully synced temporary file in the
//! same directory. Readers see the previous document or the new one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// File name under the device base directory.
pub const FILE_NAME: &str = "credentials.json";

const VERSION: u32 = 1;

/// In-memory credential document.
///
/// Logged in means both the exhibition id and the project token are set.
/// Empty strings are unset. The exhibition id and the project token are
/// stored together.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Credentials {
    device_id: Option<String>,
    base_url: Option<String>,
    expo_id: Option<String>,
    expo_name: Option<String>,
    project_token: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("device_id", &self.device_id)
            .field("base_url", &self.base_url)
            .field("expo_id", &self.expo_id)
            .field("expo_name", &self.expo_name)
            .field("project_token", &self.project_token.as_ref().map(|_| "set"))
            .finish()
    }
}

impl Credentials {
    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }

    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    pub fn expo_id(&self) -> Option<&str> {
        self.expo_id.as_deref()
    }

    pub fn expo_name(&self) -> Option<&str> {
        self.expo_name.as_deref()
    }

    pub fn project_token(&self) -> Option<&str> {
        self.project_token.as_deref()
    }

    pub fn is_logged_in(&self) -> bool {
        self.expo_id.is_some() && self.project_token.is_some()
    }

    pub fn set_device_id(&mut self, id: impl Into<String>) -> Result<(), CredentialError> {
        self.device_id = Some(require_text("device_id", id.into())?);
        Ok(())
    }

    pub fn set_base_url(&mut self, url: impl Into<String>) -> Result<(), CredentialError> {
        self.base_url = Some(require_text("base_url", url.into())?);
        Ok(())
    }

    /// Record the exhibition chosen at bind.
    ///
    /// `expo_name` is the display label. Pass `None` when the bind response
    /// has no name.
    pub fn bind(
        &mut self,
        expo_id: impl Into<String>,
        project_token: impl Into<String>,
        expo_name: Option<String>,
    ) -> Result<(), CredentialError> {
        let expo_id = require_text("expo_id", expo_id.into())?;
        let project_token = require_text("project_token", project_token.into())?;
        let expo_name = match expo_name {
            Some(name) => Some(require_text("expo_name", name)?),
            None => None,
        };
        self.expo_id = Some(expo_id);
        self.project_token = Some(project_token);
        self.expo_name = expo_name;
        Ok(())
    }

    /// Remove the exhibition id, its name, and the project token.
    pub fn clear_binding(&mut self) {
        self.expo_id = None;
        self.expo_name = None;
        self.project_token = None;
    }
}

/// Failure while reading or replacing the credential file.
#[derive(Debug)]
pub enum CredentialError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Corrupt {
        path: PathBuf,
        detail: String,
    },
    UnsupportedVersion {
        path: PathBuf,
        version: u32,
    },
    IncompleteBinding {
        path: PathBuf,
        missing: &'static str,
    },
    EmptyField(&'static str),
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredentialError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            CredentialError::Corrupt { path, detail } => {
                write!(f, "credential file {} is corrupt: {detail}", path.display())
            }
            CredentialError::UnsupportedVersion { path, version } => write!(
                f,
                "credential file {} has version {version}; this library reads version {VERSION}",
                path.display()
            ),
            CredentialError::IncompleteBinding { path, missing } => {
                write!(f, "credential file {} is missing {missing}", path.display())
            }
            CredentialError::EmptyField(field) => write!(f, "{field} must be a non-empty string"),
        }
    }
}

impl std::error::Error for CredentialError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CredentialError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Credential document at `path`.
///
/// `update` is the read-modify-write. It holds the lock from the load through
/// the store. The closure must not call `load`, `store`, or `update` on this
/// same file.
pub struct CredentialFile {
    path: PathBuf,
    write: std::sync::Mutex<()>,
}

impl std::fmt::Debug for CredentialFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialFile")
            .field("path", &self.path)
            .finish()
    }
}

impl CredentialFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write: std::sync::Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the document. A missing or empty file is an empty [`Credentials`].
    pub fn load(&self) -> Result<Credentials, CredentialError> {
        let _guard = self.lock();
        load_path(&self.path)
    }

    /// Replace the file with `credentials`.
    pub fn store(&self, credentials: &Credentials) -> Result<(), CredentialError> {
        let _guard = self.lock();
        store_path(&self.path, credentials)
    }

    /// Load, apply `f`, and store when `f` returns `Ok`.
    pub fn update<T>(
        &self,
        f: impl FnOnce(&mut Credentials) -> Result<T, CredentialError>,
    ) -> Result<T, CredentialError> {
        let _guard = self.lock();
        let mut credentials = load_path(&self.path)?;
        let value = f(&mut credentials)?;
        store_path(&self.path, &credentials)?;
        Ok(value)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write.lock().unwrap_or_else(|err| err.into_inner())
    }
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    expo_id: Option<String>,
    #[serde(default)]
    expo_name: Option<String>,
    #[serde(default)]
    project_token: Option<String>,
}

impl From<&Credentials> for Document {
    fn from(credentials: &Credentials) -> Self {
        Self {
            version: VERSION,
            device_id: credentials.device_id.clone(),
            base_url: credentials.base_url.clone(),
            expo_id: credentials.expo_id.clone(),
            expo_name: credentials.expo_name.clone(),
            project_token: credentials.project_token.clone(),
        }
    }
}

fn load_path(path: &Path) -> Result<Credentials, CredentialError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Credentials::default()),
        Err(err) => return Err(io_at(path, err)),
    };
    if bytes.is_empty() {
        return Ok(Credentials::default());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| CredentialError::Corrupt {
        path: path.to_path_buf(),
        detail: "file is not utf-8".into(),
    })?;
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Ok(Credentials::default());
    }
    if text.starts_with('{') {
        parse_json(path, text)
    } else {
        parse_properties(path, text)
    }
}

fn store_path(path: &Path, credentials: &Credentials) -> Result<(), CredentialError> {
    let document = Document::from(credentials);
    let mut bytes =
        serde_json::to_vec_pretty(&document).map_err(|err| CredentialError::Corrupt {
            path: path.to_path_buf(),
            detail: format!("could not encode credentials: {err}"),
        })?;
    bytes.push(b'\n');
    replace_file(path, &bytes)
}

fn parse_json(path: &Path, text: &str) -> Result<Credentials, CredentialError> {
    let document: Document =
        serde_json::from_str(text).map_err(|err| CredentialError::Corrupt {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?;
    if document.version != VERSION {
        return Err(CredentialError::UnsupportedVersion {
            path: path.to_path_buf(),
            version: document.version,
        });
    }
    from_parts(
        path,
        document.device_id,
        document.base_url,
        document.expo_id,
        document.expo_name,
        document.project_token,
    )
}

fn parse_properties(path: &Path, text: &str) -> Result<Credentials, CredentialError> {
    let mut device_id = None;
    let mut base_url = None;
    let mut expo_id = None;
    let mut expo_name = None;
    let mut project_token = None;
    for line in logical_lines(text) {
        let Some((key, value)) = parse_entry(path, &line)? else {
            continue;
        };
        match key.as_str() {
            "device_id" => device_id = Some(value),
            "base_url" => base_url = Some(value),
            "current_expo_uid" => expo_id = Some(value),
            "current_expo_name" => expo_name = Some(value),
            "curentr_expo_token" | "project_token" => project_token = Some(value),
            _ => {}
        }
    }
    from_parts(path, device_id, base_url, expo_id, expo_name, project_token)
}

fn from_parts(
    path: &Path,
    device_id: Option<String>,
    base_url: Option<String>,
    expo_id: Option<String>,
    expo_name: Option<String>,
    project_token: Option<String>,
) -> Result<Credentials, CredentialError> {
    let device_id = blank_to_none(device_id);
    let base_url = blank_to_none(base_url);
    let expo_id = blank_to_none(expo_id);
    let mut expo_name = blank_to_none(expo_name);
    let project_token = blank_to_none(project_token);
    match (&expo_id, &project_token) {
        (Some(_), Some(_)) => {}
        (None, None) => expo_name = None,
        (Some(_), None) => {
            return Err(CredentialError::IncompleteBinding {
                path: path.to_path_buf(),
                missing: "project_token",
            });
        }
        (None, Some(_)) => {
            return Err(CredentialError::IncompleteBinding {
                path: path.to_path_buf(),
                missing: "expo_id",
            });
        }
    }
    Ok(Credentials {
        device_id,
        base_url,
        expo_id,
        expo_name,
        project_token,
    })
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), CredentialError> {
    let dir = parent_dir(path);
    let created_dir = !dir.exists();
    fs::create_dir_all(&dir).map_err(|err| io_at(&dir, err))?;
    sync_dir(&dir)?;
    if created_dir {
        if let Some(grand) = dir.parent() {
            if !grand.as_os_str().is_empty() {
                sync_dir(grand)?;
            }
        }
    }

    let tmp = temp_path(path);
    if let Err(err) = write_temp(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(io_at(path, err));
    }
    sync_dir(&dir)?;
    Ok(())
}

fn write_temp(tmp: &Path, bytes: &[u8]) -> Result<(), CredentialError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(tmp).map_err(|err| io_at(tmp, err))?;
    file.write_all(bytes).map_err(|err| io_at(tmp, err))?;
    file.sync_all().map_err(|err| io_at(tmp, err))?;
    Ok(())
}

fn sync_dir(dir: &Path) -> Result<(), CredentialError> {
    let file = File::open(dir).map_err(|err| io_at(dir, err))?;
    file.sync_all().map_err(|err| io_at(dir, err))?;
    Ok(())
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    name.push(format!(".tmp-{}-{nanos}", std::process::id()));
    path.with_file_name(name)
}

fn io_at(path: &Path, source: io::Error) -> CredentialError {
    CredentialError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn require_text(field: &'static str, value: String) -> Result<String, CredentialError> {
    let value = value.trim();
    if value.is_empty() {
        Err(CredentialError::EmptyField(field))
    } else {
        Ok(value.to_string())
    }
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    })
}

fn logical_lines(input: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut continuing = false;
    for raw in input.split('\n') {
        let raw = raw.trim_end_matches('\r');
        let piece = if continuing {
            raw.trim_start_matches([' ', '\t', '\u{c}'])
        } else {
            raw
        };
        let (content, cont) = strip_continuation(piece);
        current.push_str(content);
        if cont {
            continuing = true;
        } else {
            lines.push(std::mem::take(&mut current));
            continuing = false;
        }
    }
    if continuing || !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn strip_continuation(line: &str) -> (&str, bool) {
    let bytes = line.as_bytes();
    let mut count = 0;
    while count < bytes.len() && bytes[bytes.len() - 1 - count] == b'\\' {
        count += 1;
    }
    if count % 2 == 1 {
        (&line[..line.len() - 1], true)
    } else {
        (line, false)
    }
}

fn parse_entry(path: &Path, line: &str) -> Result<Option<(String, String)>, CredentialError> {
    let trimmed = line.trim_start_matches([' ', '\t', '\u{c}']);
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
        return Ok(None);
    }
    let chars: Vec<char> = trimmed.chars().collect();
    let limit = chars.len();
    let mut key_len = 0;
    let mut preceding_backslash = false;
    let mut has_sep = false;
    let mut value_start = limit;
    while key_len < limit {
        let c = chars[key_len];
        if (c == '=' || c == ':') && !preceding_backslash {
            value_start = key_len + 1;
            has_sep = true;
            break;
        } else if (c == ' ' || c == '\t' || c == '\u{c}') && !preceding_backslash {
            value_start = key_len + 1;
            break;
        }
        preceding_backslash = c == '\\' && !preceding_backslash;
        key_len += 1;
    }
    while value_start < limit {
        let c = chars[value_start];
        if c != ' ' && c != '\t' && c != '\u{c}' {
            if !has_sep && (c == '=' || c == ':') {
                has_sep = true;
            } else {
                break;
            }
        }
        value_start += 1;
    }
    let key = load_convert(path, &chars[..key_len])?;
    let value = load_convert(path, &chars[value_start..])?;
    Ok(Some((key, value)))
}

fn load_convert(path: &Path, chars: &[char]) -> Result<String, CredentialError> {
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if c != '\\' {
            out.push(c);
            continue;
        }
        if i >= chars.len() {
            return Err(corrupt(path, "trailing escape"));
        }
        let next = chars[i];
        i += 1;
        if next == 'u' {
            if i + 4 > chars.len() {
                return Err(corrupt(path, "short unicode escape"));
            }
            let mut value = 0u32;
            for _ in 0..4 {
                let hex = chars[i];
                i += 1;
                let digit = hex
                    .to_digit(16)
                    .ok_or_else(|| corrupt(path, "bad unicode escape"))?;
                value = (value << 4) | digit;
            }
            let ch =
                char::from_u32(value).ok_or_else(|| corrupt(path, "invalid unicode scalar"))?;
            out.push(ch);
        } else {
            out.push(match next {
                't' => '\t',
                'r' => '\r',
                'n' => '\n',
                'f' => '\u{c}',
                other => other,
            });
        }
    }
    Ok(out)
}

fn corrupt(path: &Path, detail: &str) -> CredentialError {
    CredentialError::Corrupt {
        path: path.to_path_buf(),
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rust-reg-credentials-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Ok(metadata) = fs::metadata(&self.0) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&self.0, perms);
            }
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample() -> Credentials {
        let mut credentials = Credentials::default();
        credentials.set_device_id("device-1").unwrap();
        credentials.set_base_url("http://kuprin.su/").unwrap();
        credentials
            .bind("5245081", "token-value", Some("Выставка".into()))
            .unwrap();
        credentials
    }

    #[test]
    fn device_and_url_alone_are_not_logged_in() {
        let mut credentials = Credentials::default();
        credentials.set_device_id("device-1").unwrap();
        credentials.set_base_url("http://stage.kuprin.su/").unwrap();
        assert!(!credentials.is_logged_in());
        assert_eq!(credentials.device_id(), Some("device-1"));
        assert_eq!(credentials.base_url(), Some("http://stage.kuprin.su/"));
    }

    #[test]
    fn empty_fields_are_rejected_and_previous_binding_stays() {
        let mut credentials = sample();
        assert!(matches!(
            credentials.set_device_id("  "),
            Err(CredentialError::EmptyField("device_id"))
        ));
        assert!(matches!(
            credentials.bind("  ", "token", None),
            Err(CredentialError::EmptyField("expo_id"))
        ));
        assert_eq!(credentials.device_id(), Some("device-1"));
        assert_eq!(credentials.project_token(), Some("token-value"));
        assert!(credentials.is_logged_in());
    }

    #[test]
    fn clear_binding_keeps_device_and_url() {
        let mut credentials = sample();
        credentials.clear_binding();
        assert!(!credentials.is_logged_in());
        assert_eq!(credentials.device_id(), Some("device-1"));
        assert_eq!(credentials.base_url(), Some("http://kuprin.su/"));
        assert_eq!(credentials.expo_id(), None);
        assert_eq!(credentials.expo_name(), None);
        assert_eq!(credentials.project_token(), None);
    }

    #[test]
    fn debug_hides_the_token() {
        let rendered = format!("{:?}", sample());
        assert!(!rendered.contains("token-value"));
        assert!(rendered.contains("set"));
    }

    #[test]
    fn store_and_load_round_trip() {
        let scratch = Scratch::new();
        let path = scratch.path().join(FILE_NAME);
        let file = CredentialFile::new(&path);
        let credentials = sample();
        file.store(&credentials).unwrap();
        assert_eq!(file.load().unwrap(), credentials);
        assert_eq!(CredentialFile::new(&path).load().unwrap(), credentials);

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("Выставка"));
        assert!(!text.contains("curentr_expo_token"));
        #[cfg(unix)]
        {
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let temps: Vec<_> = fs::read_dir(scratch.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(temps.is_empty(), "{temps:?}");
    }

    #[test]
    fn empty_binding_strings_are_logged_out() {
        let scratch = Scratch::new();
        let path = scratch.path().join(FILE_NAME);
        fs::write(
            &path,
            r#"{"version":1,"device_id":"device-1","base_url":"http://kuprin.su/","expo_id":"","expo_name":"","project_token":""}"#,
        )
        .unwrap();
        let loaded = CredentialFile::new(&path).load().unwrap();
        assert!(!loaded.is_logged_in());
        assert_eq!(loaded.device_id(), Some("device-1"));
        assert_eq!(loaded.expo_name(), None);
    }

    #[test]
    fn incomplete_binding_is_not_replaced() {
        let scratch = Scratch::new();
        let path = scratch.path().join(FILE_NAME);
        let original = r#"{"version":1,"expo_id":"5245081","project_token":null}"#;
        fs::write(&path, original).unwrap();
        let file = CredentialFile::new(&path);
        let err = file
            .update(|credentials| {
                credentials.set_device_id("device-1")?;
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(
            err,
            CredentialError::IncompleteBinding {
                missing: "project_token",
                ..
            }
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn unsupported_version_and_truncated_json_stay_on_disk() {
        let scratch = Scratch::new();
        let version_path = scratch.path().join("version.json");
        fs::write(&version_path, r#"{"version":2}"#).unwrap();
        let err = CredentialFile::new(&version_path).load().unwrap_err();
        assert!(matches!(
            err,
            CredentialError::UnsupportedVersion { version: 2, .. }
        ));
        assert_eq!(
            fs::read_to_string(&version_path).unwrap(),
            r#"{"version":2}"#
        );

        let truncated = scratch.path().join("truncated.json");
        fs::write(&truncated, "{").unwrap();
        let file = CredentialFile::new(&truncated);
        assert!(matches!(
            file.load().unwrap_err(),
            CredentialError::Corrupt { .. }
        ));
        file.update(|_| Ok(())).unwrap_err();
        assert_eq!(fs::read_to_string(&truncated).unwrap(), "{");
    }

    #[test]
    fn scala_properties_import_then_json_store() {
        let scratch = Scratch::new();
        let legacy = scratch.path().join("main.props");
        fs::write(
            &legacy,
            "# comment\n\
             last_conf_synch=10\n\
             device_id = device-1\n\
             current_expo_name=Tech\\u0424\n\
             curentr_expo_token=ab\\\n\
             cd\n\
             current_expo_uid:5245081\n\
             ! ignored\n",
        )
        .unwrap();
        let loaded = CredentialFile::new(&legacy).load().unwrap();
        assert!(loaded.is_logged_in());
        assert_eq!(loaded.device_id(), Some("device-1"));
        assert_eq!(loaded.expo_id(), Some("5245081"));
        assert_eq!(loaded.expo_name(), Some("TechФ"));
        assert_eq!(loaded.project_token(), Some("abcd"));
        assert!(fs::read_to_string(&legacy)
            .unwrap()
            .contains("last_conf_synch"));

        let path = scratch.path().join(FILE_NAME);
        CredentialFile::new(&path).store(&loaded).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with('{'));
        assert!(text.contains("\"project_token\": \"abcd\""));
        assert!(!text.contains("curentr_expo_token"));
        assert!(fs::read_to_string(&legacy)
            .unwrap()
            .contains("curentr_expo_token"));
    }

    #[test]
    fn later_legacy_token_key_wins() {
        let scratch = Scratch::new();
        let path = scratch.path().join("main.props");
        fs::write(
            &path,
            "project_token=older\ncurentr_expo_token=newer\ncurrent_expo_uid=1\n",
        )
        .unwrap();
        let loaded = CredentialFile::new(&path).load().unwrap();
        assert_eq!(loaded.project_token(), Some("newer"));
    }

    #[test]
    fn crash_temp_is_ignored_and_a_failed_replace_keeps_the_previous_file() {
        let scratch = Scratch::new();
        let path = scratch.path().join(FILE_NAME);
        let file = CredentialFile::new(&path);
        let first = sample();
        file.store(&first).unwrap();
        let previous = fs::read(&path).unwrap();

        fs::write(
            scratch.path().join("credentials.json.tmp-crash"),
            "{partial",
        )
        .unwrap();
        assert_eq!(file.load().unwrap(), first);

        let mut perms = fs::metadata(scratch.path()).unwrap().permissions();
        perms.set_mode(0o555);
        fs::set_permissions(scratch.path(), perms).unwrap();
        let mut second = first.clone();
        second.bind("other", "other-token", None).unwrap();
        let err = file.store(&second).unwrap_err();
        assert!(matches!(err, CredentialError::Io { .. }), "{err}");
        assert_eq!(fs::read(&path).unwrap(), previous);

        let mut perms = fs::metadata(scratch.path()).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(scratch.path(), perms).unwrap();
        assert_eq!(file.load().unwrap(), first);
    }

    #[test]
    fn update_creates_the_file_and_a_rejected_edit_keeps_bytes() {
        let scratch = Scratch::new();
        let path = scratch.path().join("nested").join(FILE_NAME);
        let file = CredentialFile::new(&path);
        assert!(!file.load().unwrap().is_logged_in());
        file.update(|credentials| {
            credentials.set_device_id("device-1")?;
            credentials.set_base_url("http://kuprin.su/")?;
            credentials.bind("1", "tok", None)?;
            Ok(())
        })
        .unwrap();
        let loaded = CredentialFile::new(&path).load().unwrap();
        assert!(loaded.is_logged_in());
        assert_eq!(loaded.project_token(), Some("tok"));

        let bytes = fs::read(&path).unwrap();
        file.update(|credentials| {
            credentials.clear_binding();
            credentials.set_base_url(" ")
        })
        .unwrap_err();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(CredentialFile::new(&path).load().unwrap().is_logged_in());
    }

    #[test]
    fn update_clears_a_binding() {
        let scratch = Scratch::new();
        let path = scratch.path().join(FILE_NAME);
        let file = CredentialFile::new(&path);
        file.store(&sample()).unwrap();
        file.update(|credentials| {
            credentials.clear_binding();
            Ok(())
        })
        .unwrap();
        let loaded = file.load().unwrap();
        assert!(!loaded.is_logged_in());
        assert_eq!(loaded.device_id(), Some("device-1"));
        assert_eq!(loaded.base_url(), Some("http://kuprin.su/"));
    }

    #[test]
    fn missing_and_empty_files_load_as_empty() {
        let scratch = Scratch::new();
        let missing = scratch.path().join("missing.json");
        assert_eq!(
            CredentialFile::new(&missing).load().unwrap(),
            Credentials::default()
        );
        let empty = scratch.path().join("empty.json");
        fs::write(&empty, " \n").unwrap();
        assert_eq!(
            CredentialFile::new(&empty).load().unwrap(),
            Credentials::default()
        );
    }
}
