use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Once};
use std::time::Duration;

use crate::message::{decode, encode_request, Message, Operation, Value};
use crate::record::{parse_uri, Error, Formats, Job, Printer, Settings, Target, Toner};

const REQUESTED: &[&str] = &[
    "document-format-supported",
    "document-format-default",
    "document-format-preferred",
    "printer-settable-attributes-supported",
    "media-supported",
    "media-default",
    "media-ready",
    "media-col-ready",
    "media-type-supported",
    "sides-supported",
    "sides-default",
    "printer-resolution-supported",
    "printer-resolution-default",
    "print-color-mode-supported",
    "operations-supported",
    "marker-names",
    "marker-types",
    "marker-colors",
    "marker-levels",
    "marker-low-levels",
    "marker-high-levels",
    "printer-state",
    "printer-state-reasons",
    "printer-make-and-model",
    "queued-job-count",
];

impl Printer {
    /// `Get-Printer-Attributes`. A connect failure, a timeout, a non-IPP body, or a refused
    /// status leaves this record unchanged.
    pub fn info(&mut self) -> Result<(), Error> {
        let target = parse_uri(&self.uri)?;
        let requested = REQUESTED
            .iter()
            .map(|name| Value::Keyword((*name).to_string()))
            .collect();
        let body = encode_request(
            Operation::GetPrinterAttributes,
            1,
            &[
                ("printer-uri", vec![Value::Uri(self.uri.clone())]),
                ("requested-attributes", requested),
            ],
            &[],
        );
        let response = post_ipp(&target, &body)?;
        let message = decode(&response)?;
        if message.code >= 0x0100 {
            return Err(Error::new(format!("IPP status {:#06x}", message.code)));
        }
        apply(self, &message);
        Ok(())
    }

    /// Write one setting the printer lists as settable. A refused name or value sends nothing.
    /// A rejected status leaves the stored default.
    pub fn set(&mut self, setting: &str, value: &str) -> Result<(), Error> {
        let attribute = match setting {
            "media" | "media-type" | "sides" | "printer-resolution" | "print-color-mode" => setting,
            _ => {
                return Err(Error::new(format!("{setting} is not a setting")));
            }
        };
        if !self.settings.settable.iter().any(|name| name == attribute) {
            return Err(Error::new(format!("{attribute} is not settable")));
        }
        let supported = supported_values(&self.settings, attribute);
        if !supported.iter().any(|item| item == value) {
            return Err(Error::new(format!(
                "{value} is not a supported {attribute}"
            )));
        }
        let target = parse_uri(&self.uri)?;
        let written = setting_value(attribute, value)?;
        let body = encode_request(
            Operation::SetPrinterAttributes,
            1,
            &[("printer-uri", vec![Value::Uri(self.uri.clone())])],
            &[(attribute, vec![written])],
        );
        let response = post_ipp(&target, &body)?;
        let message = decode(&response)?;
        if message.code >= 0x0100 {
            return Err(Error::new(format!("IPP status {:#06x}", message.code)));
        }
        store_default(&mut self.settings, attribute, value);
        Ok(())
    }

    /// `Print-Job` for one document. `options` are job attributes. The call returns the response.
    pub fn print_job(
        &self,
        format: &str,
        options: &[(&str, &str)],
        document: &[u8],
    ) -> Result<Message, Error> {
        let mut attributes = vec![
            ("printer-uri".to_string(), Value::Uri(self.uri.clone())),
            (
                "document-format".to_string(),
                Value::Mime(format.to_string()),
            ),
        ];
        for (name, value) in options {
            attributes.push(((*name).to_string(), Value::Keyword((*value).to_string())));
        }
        self.exchange(Operation::PrintJob, &attributes, document)
    }

    /// `Get-Job-Attributes` for one job id.
    pub fn job_attributes(&self, id: i32) -> Result<Message, Error> {
        let attributes = vec![
            ("printer-uri".to_string(), Value::Uri(self.uri.clone())),
            ("job-id".to_string(), Value::Integer(id)),
        ];
        self.exchange(Operation::GetJobAttributes, &attributes, &[])
    }

    /// `Cancel-Job` for one job id.
    pub fn cancel_job(&self, id: i32) -> Result<(), Error> {
        self.job_operation(Operation::CancelJob, id)
    }

    /// `Hold-Job` for one job id.
    pub fn hold_job(&self, id: i32) -> Result<(), Error> {
        self.job_operation(Operation::HoldJob, id)
    }

    /// `Release-Job` for one job id.
    pub fn release_job(&self, id: i32) -> Result<(), Error> {
        self.job_operation(Operation::ReleaseJob, id)
    }

    /// `Resume-Printer` for this URI.
    pub fn resume(&self) -> Result<(), Error> {
        let attributes = vec![("printer-uri".to_string(), Value::Uri(self.uri.clone()))];
        self.exchange(Operation::ResumePrinter, &attributes, &[])?;
        Ok(())
    }

    /// One page in the preferred supported type, or the first supported type in the fixed order.
    pub fn print_test(&self) -> Result<i32, Error> {
        let format = page_format(&self.formats)?;
        let message = self.print_job(&format, &[], page_bytes(&format))?;
        job_id_of(&message)
    }

    /// The file bytes, when the mapped type is in `document-format-supported`.
    /// `raw` sends `application/octet-stream`.
    pub fn print_file(&self, path: &str, raw: bool) -> Result<i32, Error> {
        let format = if raw {
            "application/octet-stream".to_string()
        } else {
            mime_for(path)?.to_string()
        };
        if !self.formats.supported.iter().any(|item| item == &format) {
            return Err(Error::new(format!("{format} is not supported")));
        }
        let bytes =
            std::fs::read(path).map_err(|err| Error::new(format!("reading {path}: {err}")))?;
        let message = self.print_job(&format, &[], &bytes)?;
        job_id_of(&message)
    }

    /// `Get-Jobs` for jobs that are not completed, then the completed ones. Newest id first in each set.
    pub fn jobs(&mut self) -> Result<Vec<Job>, Error> {
        let mut jobs = self.job_list("not-completed")?;
        jobs.extend(self.job_list("completed")?);
        self.jobs = jobs.clone();
        Ok(jobs)
    }

    fn job_list(&self, which: &str) -> Result<Vec<Job>, Error> {
        let requested = vec![
            Value::Keyword("job-id".to_string()),
            Value::Keyword("job-state".to_string()),
            Value::Keyword("job-state-reasons".to_string()),
            Value::Keyword("job-name".to_string()),
            Value::Keyword("job-impressions-completed".to_string()),
        ];
        let pairs = [
            ("printer-uri", vec![Value::Uri(self.uri.clone())]),
            ("which-jobs", vec![Value::Keyword(which.to_string())]),
            ("requested-attributes", requested),
        ];
        let message = self.exchange_pairs(Operation::GetJobs, &pairs, &[])?;
        let mut jobs = jobs_from(&message);
        jobs.sort_by(|left, right| right.id.cmp(&left.id));
        Ok(jobs)
    }

    fn job_operation(&self, operation: Operation, id: i32) -> Result<(), Error> {
        let attributes = vec![
            ("printer-uri".to_string(), Value::Uri(self.uri.clone())),
            ("job-id".to_string(), Value::Integer(id)),
        ];
        self.exchange(operation, &attributes, &[])?;
        Ok(())
    }

    fn exchange(
        &self,
        operation: Operation,
        attributes: &[(String, Value)],
        document: &[u8],
    ) -> Result<Message, Error> {
        let pairs: Vec<(&str, Vec<Value>)> = attributes
            .iter()
            .map(|(name, value)| (name.as_str(), vec![value.clone()]))
            .collect();
        self.exchange_pairs(operation, &pairs, document)
    }

    fn exchange_pairs(
        &self,
        operation: Operation,
        pairs: &[(&str, Vec<Value>)],
        document: &[u8],
    ) -> Result<Message, Error> {
        let target = parse_uri(&self.uri)?;
        let mut body = encode_request(operation, 1, pairs, &[]);
        body.extend_from_slice(document);
        let response = post_ipp(&target, &body)?;
        let message = decode(&response)?;
        if message.code >= 0x0400 {
            return Err(Error::new(format!("IPP status {:#06x}", message.code)));
        }
        Ok(message)
    }
}

const PAGE_ORDER: [&str; 5] = [
    "application/pdf",
    "application/postscript",
    "image/pwg-raster",
    "image/jpeg",
    "image/png",
];

fn page_format(formats: &Formats) -> Result<String, Error> {
    if let Some(preferred) = &formats.preferred {
        if formats.supported.iter().any(|item| item == preferred) {
            return Ok(preferred.clone());
        }
    }
    PAGE_ORDER
        .iter()
        .copied()
        .find(|mime| formats.supported.iter().any(|item| item == *mime))
        .map(str::to_string)
        .ok_or_else(|| Error::new("no supported page format"))
}

fn page_bytes(format: &str) -> &'static [u8] {
    match format {
        "application/pdf" => {
            b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n"
        }
        "application/postscript" => b"%!PS-Adobe-3.0\nshowpage\n",
        "image/pwg-raster" => b"RaS2",
        "image/jpeg" => b"\xff\xd8\xff",
        "image/png" => b"\x89PNG\r\n\x1a\n",
        _ => b"test\n",
    }
}

fn mime_for(path: &str) -> Result<&'static str, Error> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Ok("application/pdf"),
        "ps" | "eps" => Ok("application/postscript"),
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "pwg" => Ok("image/pwg-raster"),
        "urf" => Ok("image/urf"),
        "tif" | "tiff" => Ok("image/tiff"),
        "txt" => Ok("text/plain"),
        "pcl" => Ok("application/vnd.hp-pcl"),
        _ => Err(Error::new(format!("no document format for {path}"))),
    }
}

fn job_id_of(message: &Message) -> Result<i32, Error> {
    match message
        .named("job-id")
        .and_then(|attribute| attribute.values.first())
    {
        Some(Value::Integer(id) | Value::Enum(id)) => Ok(*id),
        _ => Err(Error::new("job-id missing")),
    }
}

fn jobs_from(message: &Message) -> Vec<Job> {
    message
        .groups
        .iter()
        .filter_map(|group| {
            let id = group_int(group, "job-id")?;
            Some(Job {
                id,
                state: group_int(group, "job-state").unwrap_or(0),
                reasons: group_texts(group, "job-state-reasons"),
                name: group_texts(group, "job-name")
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
                impressions_completed: group_int(group, "job-impressions-completed").unwrap_or(0),
            })
        })
        .collect()
}

fn group_int(group: &crate::message::Group, name: &str) -> Option<i32> {
    group
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| {
            attribute.values.iter().find_map(|value| match value {
                Value::Integer(id) | Value::Enum(id) => Some(*id),
                _ => None,
            })
        })
}

fn group_texts(group: &crate::message::Group, name: &str) -> Vec<String> {
    group
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.values.iter().filter_map(Value::text).collect())
        .unwrap_or_default()
}

fn apply(printer: &mut Printer, message: &Message) {
    if let Some(make) = message.first_str("printer-make-and-model") {
        printer.make_model = make;
    }
    printer.formats = formats_from(message);
    printer.settings = settings_from(message);
    printer.toner = toner_from(message);
}

fn formats_from(message: &Message) -> Formats {
    Formats {
        supported: message.texts("document-format-supported"),
        default: message
            .named("document-format-default")
            .and_then(|attribute| attribute.values.first().and_then(Value::text)),
        preferred: message
            .named("document-format-preferred")
            .and_then(|attribute| attribute.values.first().and_then(Value::text)),
    }
}

fn settings_from(message: &Message) -> Settings {
    Settings {
        state: printer_state(message),
        state_reasons: message.texts("printer-state-reasons"),
        media_supported: message.texts("media-supported"),
        media_default: message
            .texts("media-default")
            .into_iter()
            .next()
            .unwrap_or_default(),
        media_ready: message.texts("media-ready"),
        media_col_ready: message.texts("media-col-ready"),
        media_type_supported: message.texts("media-type-supported"),
        media_type_default: message
            .texts("media-type-default")
            .into_iter()
            .next()
            .unwrap_or_default(),
        sides_supported: message.texts("sides-supported"),
        sides_default: message
            .texts("sides-default")
            .into_iter()
            .next()
            .unwrap_or_default(),
        resolution_supported: message.texts("printer-resolution-supported"),
        resolution_default: message
            .texts("printer-resolution-default")
            .into_iter()
            .next()
            .unwrap_or_default(),
        color_supported: message.texts("print-color-mode-supported"),
        color_default: message
            .texts("print-color-mode-default")
            .into_iter()
            .next()
            .unwrap_or_default(),
        operations_supported: message.texts("operations-supported"),
        queued_job_count: message.named("queued-job-count").and_then(|attribute| {
            attribute.values.iter().find_map(|value| match value {
                Value::Integer(count) | Value::Enum(count) => Some(*count),
                _ => None,
            })
        }),
        settable: message.texts("printer-settable-attributes-supported"),
    }
}

fn printer_state(message: &Message) -> String {
    match message
        .named("printer-state")
        .and_then(|attribute| attribute.values.first())
    {
        Some(Value::Enum(3)) => "idle".to_string(),
        Some(Value::Enum(4)) => "processing".to_string(),
        Some(Value::Enum(5)) => "stopped".to_string(),
        Some(value) => value.text().unwrap_or_default(),
        None => String::new(),
    }
}

const TONER_REASONS: &[&str] = &[
    "toner-low",
    "toner-empty",
    "marker-supply-low",
    "marker-supply-empty",
];

fn toner_from(message: &Message) -> Toner {
    let levels = integers(message, "marker-levels");
    if let Some(levels) = levels {
        return Toner {
            names: message.texts("marker-names"),
            types: message.texts("marker-types"),
            colors: message.texts("marker-colors"),
            levels,
            low_levels: integers(message, "marker-low-levels").unwrap_or_default(),
            high_levels: integers(message, "marker-high-levels").unwrap_or_default(),
            reasons: Vec::new(),
        };
    }
    Toner {
        reasons: message
            .texts("printer-state-reasons")
            .into_iter()
            .filter(|reason| TONER_REASONS.contains(&reason.as_str()))
            .collect(),
        ..Toner::default()
    }
}

fn integers(message: &Message, name: &str) -> Option<Vec<i32>> {
    let attribute = message.named(name)?;
    Some(
        attribute
            .values
            .iter()
            .filter_map(|value| match value {
                Value::Integer(number) | Value::Enum(number) => Some(*number),
                _ => None,
            })
            .collect(),
    )
}

fn supported_values<'a>(settings: &'a Settings, attribute: &str) -> &'a [String] {
    match attribute {
        "media" => &settings.media_supported,
        "media-type" => &settings.media_type_supported,
        "sides" => &settings.sides_supported,
        "printer-resolution" => &settings.resolution_supported,
        "print-color-mode" => &settings.color_supported,
        _ => &[],
    }
}

fn store_default(settings: &mut Settings, attribute: &str, value: &str) {
    let slot = match attribute {
        "media" => &mut settings.media_default,
        "media-type" => &mut settings.media_type_default,
        "sides" => &mut settings.sides_default,
        "printer-resolution" => &mut settings.resolution_default,
        "print-color-mode" => &mut settings.color_default,
        _ => return,
    };
    *slot = value.to_string();
}

fn setting_value(attribute: &str, value: &str) -> Result<Value, Error> {
    if attribute == "printer-resolution" {
        return parse_resolution(value);
    }
    Ok(Value::Keyword(value.to_string()))
}

fn parse_resolution(value: &str) -> Result<Value, Error> {
    let (body, units) = if let Some(body) = value.strip_suffix("dpi") {
        (body, 3)
    } else if let Some(body) = value.strip_suffix("dpc") {
        (body, 4)
    } else {
        return Err(Error::new(format!("{value} is not a resolution")));
    };
    let (x, y) = if let Some((x, y)) = body.split_once('x') {
        (parse_dots(x)?, parse_dots(y)?)
    } else {
        let dots = parse_dots(body)?;
        (dots, dots)
    };
    Ok(Value::Resolution { x, y, units })
}

fn parse_dots(value: &str) -> Result<i32, Error> {
    value
        .parse()
        .map_err(|_| Error::new(format!("{value} is not a resolution")))
}

/// Connect stays at two seconds. The body budget is two seconds plus one second
/// for every 256 KiB of the request, and it stops at 60 seconds.
pub fn body_budget(bytes: usize) -> Duration {
    const FLOOR: u64 = 2;
    const STEP: u64 = 256 * 1024;
    const CAP: u64 = 60;
    let seconds = FLOOR + (bytes as u64) / STEP;
    Duration::from_secs(seconds.min(CAP))
}

fn post_ipp(target: &Target, body: &[u8]) -> Result<Vec<u8>, Error> {
    let addr = socket_addr(&target.host, target.port)?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|err| {
        Error::new(format!(
            "connecting to {}:{}: {err}",
            target.host, target.port
        ))
    })?;
    let budget = body_budget(body.len());
    stream
        .set_read_timeout(Some(budget))
        .map_err(|err| Error::new(format!("setting read timeout: {err}")))?;
    stream
        .set_write_timeout(Some(budget))
        .map_err(|err| Error::new(format!("setting write timeout: {err}")))?;
    if target.secure {
        let mut tls = tls_stream(&target.host, stream)?;
        write_ipp(&mut tls, target, body)
    } else {
        let mut stream = stream;
        write_ipp(&mut stream, target, body)
    }
}

/// Venue printers present their own certificate. The bytes still go over TLS.
/// A public CA is not required for that handshake.
fn tls_stream(
    host: &str,
    stream: TcpStream,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, Error> {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptServerCert))
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|_| Error::new(format!("{host} is not a TLS server name")))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), name)
        .map_err(|err| Error::new(format!("starting TLS to {host}: {err}")))?;
    Ok(rustls::StreamOwned::new(conn, stream))
}

#[derive(Debug)]
struct AcceptServerCert;

impl rustls::client::danger::ServerCertVerifier for AcceptServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn write_ipp(stream: &mut impl ReadWrite, target: &Target, body: &[u8]) -> Result<Vec<u8>, Error> {
    let header = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        target.path, target.host, target.port, body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|err| Error::new(format!("sending IPP request: {err}")))?;
    let (status, response) = read_http(stream)?;
    if !(200..300).contains(&status) {
        return Err(Error::new(format!("HTTP status {status}")));
    }
    Ok(response)
}

trait ReadWrite: Read + Write {}

impl<T: Read + Write> ReadWrite for T {}

fn socket_addr(host: &str, port: u16) -> Result<SocketAddr, Error> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    (host, port)
        .to_socket_addrs()
        .map_err(|err| Error::new(format!("resolving {host}: {err}")))?
        .next()
        .ok_or_else(|| Error::new(format!("resolving {host} returned no address")))
}

fn read_http(stream: &mut impl Read) -> Result<(u16, Vec<u8>), Error> {
    let (head, body) = read_framed(stream)?;
    Ok((http_status(&head)?, body))
}

fn read_framed(stream: &mut impl Read) -> Result<(String, Vec<u8>), Error> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        if let Some(parsed) = framed(&buf)? {
            return Ok(parsed);
        }
        match stream.read(&mut tmp) {
            Ok(0) => {
                return framed(&buf)?
                    .ok_or_else(|| Error::new("IPP response ended before the body"));
            }
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(err)
                if err.kind() == std::io::ErrorKind::TimedOut
                    || err.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Err(Error::new(format!("timeout reading IPP response: {err}")));
            }
            Err(err) => return Err(Error::new(format!("reading IPP response: {err}"))),
        }
    }
}

/// Headers plus a `Content-Length` body. The status line is not interpreted, so this
/// also frames the request the stand-in reads.
fn framed(buf: &[u8]) -> Result<Option<(String, Vec<u8>)>, Error> {
    let Some(split) = find_header_end(buf) else {
        return Ok(None);
    };
    let head =
        std::str::from_utf8(&buf[..split]).map_err(|_| Error::new("HTTP head is not UTF-8"))?;
    let Some(length) = content_length(head) else {
        return Ok(None);
    };
    let body_at = split + 4;
    if buf.len() < body_at + length {
        return Ok(None);
    }
    Ok(Some((
        head.to_string(),
        buf[body_at..body_at + length].to_vec(),
    )))
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn http_status(head: &str) -> Result<u16, Error> {
    let line = head.lines().next().unwrap_or("");
    let code = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| Error::new(format!("HTTP status line is {line}")))?;
    code.parse::<u16>()
        .map_err(|_| Error::new(format!("HTTP status is {code}")))
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::message::{decode, Message, Operation, Value};

    pub(super) fn response_with_make(make: &str) -> Vec<u8> {
        crate::message::encode_request(
            Operation::GetPrinterAttributes,
            1,
            &[],
            &[(
                "printer-make-and-model",
                vec![Value::Name(make.to_string())],
            )],
        )
    }

    pub(super) fn serve(ipp: Vec<u8>) -> (u16, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                if framed(&request).ok().flatten().is_some() {
                    break;
                }
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&tmp[..n]),
                }
            }
            let _ = tx.send(request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                ipp.len()
            );
            let _ = sock.write_all(header.as_bytes());
            let _ = sock.write_all(&ipp);
        });
        (port, rx)
    }

    pub(super) fn request_message(raw: &[u8]) -> Message {
        let split = find_header_end(raw).unwrap();
        decode(&raw[split + 4..]).unwrap()
    }

    fn serve_late(ipp: Vec<u8>, delay: Duration) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                if framed(&request).ok().flatten().is_some() {
                    break;
                }
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => request.extend_from_slice(&tmp[..n]),
                }
            }
            thread::sleep(delay);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                ipp.len()
            );
            let _ = sock.write_all(header.as_bytes());
            let _ = sock.write_all(&ipp);
        });
        port
    }

    fn job_response(id: i32) -> Vec<u8> {
        let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
        push_text(&mut out, 0x47, "attributes-charset", "utf-8");
        push_text(&mut out, 0x48, "attributes-natural-language", "en");
        out.push(0x02);
        push_i32(&mut out, 0x21, "job-id", id);
        out.push(0x03);
        out
    }

    fn jobs_response(jobs: &[(i32, i32, &str)]) -> Vec<u8> {
        let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
        push_text(&mut out, 0x47, "attributes-charset", "utf-8");
        push_text(&mut out, 0x48, "attributes-natural-language", "en");
        for (id, state, name) in jobs {
            out.push(0x02);
            push_i32(&mut out, 0x21, "job-id", *id);
            push_i32(&mut out, 0x23, "job-state", *state);
            push_text(&mut out, 0x44, "job-state-reasons", "none");
            push_text(&mut out, 0x42, "job-name", name);
            push_i32(&mut out, 0x21, "job-impressions-completed", 0);
        }
        out.push(0x03);
        out
    }

    fn push_text(out: &mut Vec<u8>, tag: u8, name: &str, value: &str) {
        out.push(tag);
        counted(out, name.as_bytes());
        counted(out, value.as_bytes());
    }

    fn push_i32(out: &mut Vec<u8>, tag: u8, name: &str, value: i32) {
        out.push(tag);
        counted(out, name.as_bytes());
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn counted(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        out.extend_from_slice(bytes);
    }

    fn serve_seq(responses: Vec<Vec<u8>>) -> (u16, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for ipp in responses {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    if framed(&request).ok().flatten().is_some() {
                        break;
                    }
                    match sock.read(&mut tmp) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&tmp[..n]),
                    }
                }
                let _ = tx.send(request);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    ipp.len()
                );
                let _ = sock.write_all(header.as_bytes());
                let _ = sock.write_all(&ipp);
            }
        });
        (port, rx)
    }

    fn printer_with(port: u16, supported: &[&str], preferred: Option<&str>) -> Printer {
        let mut printer = Printer::from_uri(&format!("ipp://127.0.0.1:{port}/ipp/print")).unwrap();
        printer.formats.supported = supported.iter().map(|item| (*item).to_string()).collect();
        printer.formats.preferred = preferred.map(str::to_string);
        printer
    }

    fn request_has(raw: &[u8], operation: Operation) -> Message {
        let message = request_message(raw);
        assert_eq!(message.operation(), Some(operation));
        message
    }

    #[test]
    fn preferred_pdf_is_sent() {
        let (port, rx) = serve(job_response(4));
        let printer = printer_with(port, &["application/pdf"], Some("application/pdf"));
        assert_eq!(printer.print_test().unwrap(), 4);
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let message = request_has(&raw, Operation::PrintJob);
        assert_eq!(
            message.first_str("document-format").as_deref(),
            Some("application/pdf")
        );
        assert!(raw.windows(4).any(|window| window == b"%PDF"));
    }

    #[test]
    fn the_order_picks_postscript() {
        let (port, rx) = serve(job_response(2));
        let printer = printer_with(port, &["image/jpeg", "application/postscript"], None);
        assert_eq!(printer.print_test().unwrap(), 2);
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        request_has(&raw, Operation::PrintJob);
        assert!(raw.windows(4).any(|window| window == b"%!PS"));
    }

    #[test]
    fn no_usable_type_sends_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let printer = printer_with(port, &["text/plain"], None);
        let err = printer.print_test().unwrap_err();
        assert!(
            err.to_string().contains("no supported page format"),
            "{err}"
        );
        assert!(listener.accept().is_err());
    }

    #[test]
    fn a_pdf_file_is_sent() {
        let (port, rx) = serve(job_response(6));
        let dir = std::env::temp_dir().join(format!("ipp-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("page.pdf");
        std::fs::write(&path, b"%PDF-file").unwrap();
        let printer = printer_with(port, &["application/pdf"], None);
        assert_eq!(
            printer.print_file(path.to_str().unwrap(), false).unwrap(),
            6
        );
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let message = request_has(&raw, Operation::PrintJob);
        assert_eq!(
            message.first_str("document-format").as_deref(),
            Some("application/pdf")
        );
        assert!(raw.windows(9).any(|window| window == b"%PDF-file"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_png_the_printer_rejected_is_not_sent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(format!("ipp-png-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("page.png");
        std::fs::write(&path, b"\x89PNG").unwrap();
        let printer = printer_with(port, &["application/pdf"], None);
        let err = printer
            .print_file(path.to_str().unwrap(), false)
            .unwrap_err();
        assert!(err.to_string().contains("image/png"), "{err}");
        assert!(listener.accept().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn raw_is_octet_stream() {
        let (port, rx) = serve(job_response(3));
        let dir = std::env::temp_dir().join(format!("ipp-raw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("badge.bin");
        std::fs::write(&path, b"^XA").unwrap();
        let printer = printer_with(port, &["application/octet-stream"], None);
        assert_eq!(printer.print_file(path.to_str().unwrap(), true).unwrap(), 3);
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let message = request_has(&raw, Operation::PrintJob);
        assert_eq!(
            message.first_str("document-format").as_deref(),
            Some("application/octet-stream")
        );
        assert!(raw.windows(3).any(|window| window == b"^XA"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jobs_list_the_open_ones_first() {
        let open = jobs_response(&[(1, 5, "older"), (4, 5, "newer")]);
        let done = jobs_response(&[(2, 9, "finished")]);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().unwrap();
                sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    if framed(&request).ok().flatten().is_some() {
                        break;
                    }
                    match sock.read(&mut tmp) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&tmp[..n]),
                    }
                }
                let ipp = if request.windows(13).any(|window| window == b"not-completed") {
                    open.clone()
                } else {
                    done.clone()
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    ipp.len()
                );
                let _ = sock.write_all(header.as_bytes());
                let _ = sock.write_all(&ipp);
            }
        });
        let mut printer = printer_with(port, &[], None);
        let jobs = printer.jobs().unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].id, 4);
        assert_eq!(jobs[0].state, 5);
        assert_eq!(jobs[1].id, 1);
        assert_eq!(jobs[2].id, 2);
        assert_eq!(jobs[2].state, 9);
        assert_eq!(printer.jobs, jobs);
    }

    #[test]
    fn cancel_interrupts_the_job() {
        let (port, rx) = serve_seq(vec![
            crate::message::encode_response(0, 1),
            jobs_response(&[(7, 7, "badge")]),
        ]);
        let printer = printer_with(port, &[], None);
        printer.cancel_job(7).unwrap();
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let message = request_has(&raw, Operation::CancelJob);
        assert_eq!(message.texts("job-id"), ["7"]);
        let job = printer.job_attributes(7).unwrap();
        assert_eq!(job.texts("job-state"), ["7"]);
    }

    #[test]
    fn an_unknown_id_is_the_printer_status() {
        let (port, _rx) = serve(crate::message::encode_response(0x0400, 1));
        let mut printer = printer_with(port, &[], None);
        printer.jobs = vec![Job {
            id: 1,
            state: 5,
            ..Job::default()
        }];
        let kept = printer.jobs.clone();
        let err = printer.cancel_job(9).unwrap_err();
        assert!(err.to_string().contains("0x0400"), "{err}");
        assert_eq!(printer.jobs, kept);
    }

    #[test]
    fn hold_and_release_name_the_job() {
        let (port, rx) = serve_seq(vec![
            crate::message::encode_response(0, 1),
            crate::message::encode_response(0, 1),
        ]);
        let printer = printer_with(port, &[], None);
        printer.hold_job(3).unwrap();
        printer.release_job(3).unwrap();
        let held = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let released = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            request_has(&held, Operation::HoldJob).texts("job-id"),
            ["3"]
        );
        assert_eq!(
            request_has(&released, Operation::ReleaseJob).texts("job-id"),
            ["3"]
        );
    }

    #[test]
    fn resume_names_the_printer() {
        let (port, rx) = serve(crate::message::encode_response(0, 1));
        let printer = printer_with(port, &[], None);
        printer.resume().unwrap();
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let message = request_has(&raw, Operation::ResumePrinter);
        assert_eq!(message.texts("printer-uri"), [printer.uri.as_str()]);
    }

    #[test]
    fn body_budget_grows_with_the_document() {
        assert_eq!(body_budget(0), Duration::from_secs(2));
        assert_eq!(body_budget(256 * 1024 - 1), Duration::from_secs(2));
        assert_eq!(body_budget(256 * 1024), Duration::from_secs(3));
        assert_eq!(body_budget(256 * 1024 * 80), Duration::from_secs(60));
    }

    #[test]
    fn a_small_response_is_bounded_at_two_seconds() {
        let port = serve_late(response_with_make("Zebra"), Duration::from_millis(2500));
        let uri = format!("ipp://127.0.0.1:{port}/ipp/print");
        let mut printer = Printer::from_uri(&uri).unwrap();
        let started = std::time::Instant::now();
        let err = printer.info().unwrap_err();
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(1500) && elapsed < Duration::from_millis(3000),
            "{elapsed:?} {err}"
        );
        assert!(err.to_string().contains("timeout"), "{err}");
    }

    #[test]
    fn a_large_document_waits_for_the_response() {
        let port = serve_late(
            crate::message::encode_response(0, 1),
            Duration::from_millis(2500),
        );
        let uri = format!("ipp://127.0.0.1:{port}/ipp/print");
        let printer = Printer::from_uri(&uri).unwrap();
        let document = vec![0u8; 256 * 1024];
        let started = std::time::Instant::now();
        printer
            .print_job("application/pdf", &[], &document)
            .unwrap();
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(2500), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    #[test]
    fn info_reads_the_make() {
        let (port, rx) = serve(response_with_make("Zebra"));
        let uri = format!("ipp://127.0.0.1:{port}/ipp/print");
        let mut printer = Printer::from_uri(&uri).unwrap();
        printer.info().unwrap();
        assert_eq!(printer.make_model, "Zebra");
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let head = std::str::from_utf8(&raw[..find_header_end(&raw).unwrap()]).unwrap();
        assert!(head.starts_with("POST /ipp/print "), "{head}");
        assert!(
            head.to_ascii_lowercase()
                .contains("content-type: application/ipp"),
            "{head}"
        );
        let message = request_message(&raw);
        assert_eq!(message.operation(), Some(Operation::GetPrinterAttributes));
        assert_eq!(
            message.first_str("printer-uri").as_deref(),
            Some(uri.as_str())
        );
        assert_eq!(message.strings("requested-attributes"), REQUESTED);
    }

    #[test]
    fn a_down_printer_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let uri = format!("ipp://127.0.0.1:{port}/ipp/print");
        let mut printer = Printer::from_uri(&uri).unwrap();
        printer.make_model = "HP".to_string();
        let err = printer.info().unwrap_err();
        assert!(
            err.message().contains("connecting"),
            "expected a connect failure, got {err}"
        );
        assert_eq!(printer.make_model, "HP");
    }

    #[test]
    fn a_body_that_is_not_ipp_leaves_the_make() {
        let (port, _rx) = serve(b"not-ipp".to_vec());
        let uri = format!("ipp://127.0.0.1:{port}/ipp/print");
        let mut printer = Printer::from_uri(&uri).unwrap();
        printer.make_model = "HP".to_string();
        let err = printer.info().unwrap_err();
        assert!(
            err.message().contains("truncated IPP message"),
            "expected a non-IPP body, got {err}"
        );
        assert_eq!(printer.make_model, "HP");
    }
}

#[cfg(test)]
mod harness {
    use std::sync::mpsc;

    use crate::message::{encode_request, Message, Operation, Value};
    use crate::record::Printer;

    pub(super) fn serve(ipp: Vec<u8>) -> (u16, mpsc::Receiver<Vec<u8>>) {
        super::tests::serve(ipp)
    }

    pub(super) fn request_message(raw: &[u8]) -> Message {
        super::tests::request_message(raw)
    }

    pub fn keywords(values: &[&str]) -> Vec<Value> {
        values
            .iter()
            .map(|value| Value::Keyword((*value).to_string()))
            .collect()
    }

    pub fn names(values: &[&str]) -> Vec<Value> {
        values
            .iter()
            .map(|value| Value::Name((*value).to_string()))
            .collect()
    }

    pub fn mimes(values: &[&str]) -> Vec<Value> {
        values
            .iter()
            .map(|value| Value::Mime((*value).to_string()))
            .collect()
    }

    pub fn integers(values: &[i32]) -> Vec<Value> {
        values.iter().copied().map(Value::Integer).collect()
    }

    pub fn reply(attributes: &[(&str, Vec<Value>)]) -> Vec<u8> {
        encode_request(Operation::GetPrinterAttributes, 1, &[], attributes)
    }

    pub fn open(ipp: Vec<u8>) -> (Printer, mpsc::Receiver<Vec<u8>>) {
        let (port, rx) = serve(ipp);
        let printer = Printer::from_uri(&format!("ipp://127.0.0.1:{port}/ipp/print")).unwrap();
        (printer, rx)
    }
}

#[cfg(test)]
mod formats {
    use super::harness;

    #[test]
    fn supported_formats_are_kept() {
        let body = harness::reply(&[(
            "document-format-supported",
            harness::mimes(&[
                "application/pdf",
                "application/postscript",
                "image/pwg-raster",
                "application/vnd.cups-postscript",
            ]),
        )]);
        let (mut printer, _) = harness::open(body);
        printer.info().unwrap();
        assert_eq!(
            printer.formats.supported,
            [
                "application/pdf",
                "application/postscript",
                "image/pwg-raster",
                "application/vnd.cups-postscript",
            ]
        );
    }

    #[test]
    fn preferred_is_optional() {
        let body = harness::reply(&[
            (
                "document-format-supported",
                harness::mimes(&["application/pdf"]),
            ),
            (
                "document-format-default",
                harness::mimes(&["application/pdf"]),
            ),
        ]);
        let (mut printer, _) = harness::open(body);
        printer.formats.preferred = Some("image/jpeg".to_string());
        printer.info().unwrap();
        assert!(printer.formats.preferred.is_none());
        assert_eq!(printer.formats.default.as_deref(), Some("application/pdf"));
        assert_eq!(printer.formats.supported, ["application/pdf"]);
    }
}

#[cfg(test)]
mod settings {
    use super::harness;

    #[test]
    fn sides_and_media_are_read() {
        let body = harness::reply(&[
            (
                "sides-supported",
                harness::keywords(&["one-sided", "two-sided-long-edge"]),
            ),
            ("sides-default", harness::keywords(&["one-sided"])),
            ("media-default", harness::keywords(&["iso_a4_210x297mm"])),
        ]);
        let (mut printer, _) = harness::open(body);
        printer.info().unwrap();
        assert_eq!(
            printer.settings.sides_supported,
            ["one-sided", "two-sided-long-edge"]
        );
        assert_eq!(printer.settings.sides_default, "one-sided");
        assert_eq!(printer.settings.media_default, "iso_a4_210x297mm");
    }

    #[test]
    fn an_omitted_resolution_stays_empty() {
        let body = harness::reply(&[
            (
                "sides-supported",
                harness::keywords(&["one-sided", "two-sided-long-edge"]),
            ),
            ("sides-default", harness::keywords(&["one-sided"])),
        ]);
        let (mut printer, _) = harness::open(body);
        printer.settings.resolution_supported = vec!["600dpi".to_string()];
        printer.settings.resolution_default = "600dpi".to_string();
        printer.info().unwrap();
        assert!(printer.settings.resolution_supported.is_empty());
        assert!(printer.settings.resolution_default.is_empty());
        assert_eq!(printer.settings.sides_default, "one-sided");
    }
}

#[cfg(test)]
mod toner {
    use super::harness;
    use crate::message::Value;
    use crate::record::Toner;

    #[test]
    fn levels_are_the_reading() {
        let body = harness::reply(&[
            ("marker-names", harness::names(&["black", "cyan"])),
            ("marker-types", harness::keywords(&["toner", "toner"])),
            ("marker-levels", harness::integers(&[40, -1])),
        ]);
        let (mut printer, _) = harness::open(body);
        printer.info().unwrap();
        assert_eq!(printer.toner.names, ["black", "cyan"]);
        assert_eq!(printer.toner.types, ["toner", "toner"]);
        assert_eq!(printer.toner.levels, [40, -1]);
        assert!(printer.toner.reasons.is_empty());
    }

    #[test]
    fn reasons_stand_in_for_levels() {
        let body = harness::reply(&[(
            "printer-state-reasons",
            harness::keywords(&["media-empty", "toner-low"]),
        )]);
        let (mut printer, _) = harness::open(body);
        printer.info().unwrap();
        assert!(printer.toner.levels.is_empty());
        assert_eq!(printer.toner.reasons, ["toner-low"]);
        assert_eq!(printer.settings.state_reasons, ["media-empty", "toner-low"]);
    }

    #[test]
    fn neither_is_an_empty_reading() {
        let body = harness::reply(&[
            ("printer-state", vec![Value::Enum(3)]),
            ("printer-state-reasons", harness::keywords(&["cover-open"])),
        ]);
        let (mut printer, _) = harness::open(body);
        printer.info().unwrap();
        assert_eq!(printer.toner, Toner::default());
        assert_eq!(printer.settings.state, "idle");
        assert_eq!(printer.settings.state_reasons, ["cover-open"]);
    }
}

#[cfg(test)]
mod set {
    use std::io::ErrorKind;
    use std::net::TcpListener;

    use super::harness;
    use crate::message::{encode_response, Operation};

    #[test]
    fn duplex_is_written() {
        let (mut printer, rx) = harness::open(encode_response(0x0000, 1));
        printer.settings.settable = vec!["sides".to_string()];
        printer.settings.sides_supported =
            vec!["one-sided".to_string(), "two-sided-long-edge".to_string()];
        printer.settings.sides_default = "one-sided".to_string();
        printer.set("sides", "two-sided-long-edge").unwrap();
        assert_eq!(printer.settings.sides_default, "two-sided-long-edge");
        let raw = rx.recv().unwrap();
        let message = harness::request_message(&raw);
        assert_eq!(message.operation(), Some(Operation::SetPrinterAttributes));
        assert_eq!(
            message.first_str("sides").as_deref(),
            Some("two-sided-long-edge")
        );
    }

    #[test]
    fn a_readonly_name_sends_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut printer =
            crate::Printer::from_uri(&format!("ipp://127.0.0.1:{port}/ipp/print")).unwrap();
        printer.settings.settable = vec!["media".to_string()];
        printer.settings.sides_supported = vec!["two-sided-long-edge".to_string()];
        printer.settings.sides_default = "one-sided".to_string();
        let err = printer.set("sides", "two-sided-long-edge").unwrap_err();
        assert!(
            err.message().contains("sides"),
            "error should name sides, got {err}"
        );
        assert_eq!(printer.settings.sides_default, "one-sided");
        match listener.accept() {
            Err(err) if err.kind() == ErrorKind::WouldBlock => {}
            other => panic!("stand-in saw a write: {other:?}"),
        }
    }

    #[test]
    fn a_rejected_write_keeps_the_default() {
        let (mut printer, rx) = harness::open(encode_response(0x0400, 1));
        printer.settings.settable = vec!["sides".to_string()];
        printer.settings.sides_supported =
            vec!["one-sided".to_string(), "two-sided-long-edge".to_string()];
        printer.settings.sides_default = "one-sided".to_string();
        let err = printer.set("sides", "two-sided-long-edge").unwrap_err();
        assert!(
            err.message().contains("0x0400"),
            "expected the printer status, got {err}"
        );
        assert_eq!(printer.settings.sides_default, "one-sided");
        let raw = rx.recv().unwrap();
        let message = harness::request_message(&raw);
        assert_eq!(message.operation(), Some(Operation::SetPrinterAttributes));
    }
}

#[cfg(test)]
mod tls {
    use std::io::Read;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use crate::record::Printer;

    #[test]
    fn ipps_does_not_send_cleartext() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut sock = loop {
                match listener.accept() {
                    Ok((sock, _)) => break sock,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        if start.elapsed() > Duration::from_secs(2) {
                            return Vec::new();
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return Vec::new(),
                }
            };
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buf = [0u8; 8];
            let n = sock.read(&mut buf).unwrap_or(0);
            buf[..n].to_vec()
        });
        let mut printer = Printer::from_uri(&format!("ipps://127.0.0.1:{port}/ipp/print")).unwrap();
        let _ = printer.info();
        let bytes = handle.join().unwrap();
        assert!(
            bytes.first() != Some(&b'P'),
            "ipps wrote cleartext HTTP: {bytes:?}"
        );
    }
}
