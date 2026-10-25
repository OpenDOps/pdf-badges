/// One discovered printer. `info` fills `formats`, `settings`, and `toner`. `jobs` is filled by `Printer::jobs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub uri: String,
    pub address: String,
    pub port: u16,
    pub secure: bool,
    pub make_model: String,
    pub formats: Formats,
    pub settings: Settings,
    pub toner: Toner,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Formats {
    pub supported: Vec<String>,
    pub default: Option<String>,
    pub preferred: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub state: String,
    pub state_reasons: Vec<String>,
    pub media_supported: Vec<String>,
    pub media_default: String,
    pub media_ready: Vec<String>,
    pub media_col_ready: Vec<String>,
    pub media_type_supported: Vec<String>,
    pub media_type_default: String,
    pub sides_supported: Vec<String>,
    pub sides_default: String,
    pub resolution_supported: Vec<String>,
    pub resolution_default: String,
    pub color_supported: Vec<String>,
    pub color_default: String,
    pub operations_supported: Vec<String>,
    pub queued_job_count: Option<i32>,
    pub settable: Vec<String>,
}

/// Marker columns are parallel. `reasons` is the toner keywords from `printer-state-reasons` when `levels` was not sent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Toner {
    pub names: Vec<String>,
    pub types: Vec<String>,
    pub colors: Vec<String>,
    pub levels: Vec<i32>,
    pub low_levels: Vec<i32>,
    pub high_levels: Vec<i32>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Job {
    pub id: i32,
    pub state: i32,
    pub reasons: Vec<String>,
    pub name: String,
    pub impressions_completed: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl Error {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl Printer {
    /// Build a record from an `ipp://` or `ipps://` URI. No socket is opened.
    pub fn from_uri(uri: &str) -> Result<Self, Error> {
        let target = parse_uri(uri)?;
        Ok(Self {
            name: String::new(),
            uri: uri.to_string(),
            address: target.host,
            port: target.port,
            secure: target.secure,
            make_model: String::new(),
            formats: Formats::default(),
            settings: Settings::default(),
            toner: Toner::default(),
            jobs: Vec::new(),
        })
    }
}

pub(crate) struct Target {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub secure: bool,
}

pub(crate) fn parse_uri(uri: &str) -> Result<Target, Error> {
    let (scheme, rest) = uri
        .split_once("://")
        .ok_or_else(|| Error::new(format!("IPP URI has no scheme: {uri}")))?;
    let secure = match scheme {
        "ipp" => false,
        "ipps" => true,
        _ => return Err(Error::new(format!("IPP URI scheme is {scheme}"))),
    };
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() {
        return Err(Error::new(format!("IPP URI has no host: {uri}")));
    }
    let (host, port) = split_host_port(authority)?;
    if host.is_empty() {
        return Err(Error::new(format!("IPP URI has no host: {uri}")));
    }
    Ok(Target {
        host,
        port,
        path,
        secure,
    })
}

fn split_host_port(authority: &str) -> Result<(String, u16), Error> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| Error::new(format!("IPP URI has a broken host: {authority}")))?;
        let port = if let Some(port) = after.strip_prefix(':') {
            parse_port(port)?
        } else if after.is_empty() {
            631
        } else {
            return Err(Error::new(format!(
                "IPP URI has a broken host: {authority}"
            )));
        };
        return Ok((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) => {
            Ok((host.to_string(), parse_port(port)?))
        }
        _ => Ok((authority.to_string(), 631)),
    }
}

fn parse_port(port: &str) -> Result<u16, Error> {
    port.parse::<u16>()
        .map_err(|_| Error::new(format!("IPP URI has port {port}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ipps_uri_is_secure() {
        let printer = Printer::from_uri("ipps://192.0.2.10/ipp/print").unwrap();
        assert!(printer.secure);
        assert_eq!(printer.port, 631);
        assert_eq!(printer.address, "192.0.2.10");
        assert!(printer.formats.supported.is_empty());
        assert!(printer.formats.default.is_none());
        assert!(printer.formats.preferred.is_none());
        assert_eq!(printer.settings, Settings::default());
        assert_eq!(printer.toner, Toner::default());
        assert!(printer.jobs.is_empty());
    }

    #[test]
    fn an_ipp_uri_is_not_secure() {
        let printer = Printer::from_uri("ipp://192.0.2.10:631/ipp/print").unwrap();
        assert!(!printer.secure);
        assert_eq!(printer.port, 631);
        assert_eq!(printer.address, "192.0.2.10");
    }
}
