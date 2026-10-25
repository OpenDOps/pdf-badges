use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const CUPS_GET_PRINTERS: u16 = 0x4002;
const CUPS_GET_DEVICES: u16 = 0x400b;
const CUPS_GET_PPD: u16 = 0x400f;
const PRINTER_ATTRIBUTES: u8 = 0x04;
const IPP_PORT: u16 = 631;

impl Queue {
    /// One queue from a make and a model. `choices` and `host` start empty.
    /// `installed` is true because the queue exists. `connected` stays false.
    pub fn from_make(make: &str, model: &str) -> Self {
        let make_model = match (make.is_empty(), model.is_empty()) {
            (false, false) => format!("{make} {model}"),
            _ => format!("{make}{model}"),
        };
        let (kind, dpi, width_in) = classify(&make_model);
        Self {
            name: String::new(),
            device_uri: String::new(),
            make_model,
            state: String::new(),
            accepting: false,
            host: String::new(),
            kind,
            dpi,
            width_in,
            choices: Vec::new(),
            installed: true,
            connected: false,
            jobs: None,
        }
    }
}

thread_local! {
    static CUPS_BASE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `CUPS-Get-Printers` on the local cupsd. A test points this at the stand-in.
/// This does not call `EnumPrinters` and does not send `Print-Job`.
pub fn list() -> Result<Vec<Queue>, Error> {
    let base = CUPS_BASE.with(|slot| slot.borrow().clone());
    list_service(base.as_deref())
}

/// `service` is the local print service. `None` is the default cupsd.
pub fn list_service(service: Option<&str>) -> Result<Vec<Queue>, Error> {
    cups_queues(service.unwrap_or("http://127.0.0.1:631"))
}

/// Host, port, and address taken from an `ipp://` or `ipps://` device URI.
/// A host name stays in `name`. The caller that has a resolver passes the address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub name: String,
    pub port: u16,
    pub address: Option<String>,
}

impl Host {
    pub fn with_address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }
}

/// `Some` for `ipp://` and `ipps://`. `None` for `usb://`, `socket://`, `dnssd://`,
/// and every other scheme. A missing host in an IPP URI is an error that names the URI.
pub fn host_from_uri(uri: &str) -> Result<Option<Host>, Error> {
    let rest = if let Some(rest) = uri.strip_prefix("ipp://") {
        rest
    } else if let Some(rest) = uri.strip_prefix("ipps://") {
        rest
    } else {
        return Ok(None);
    };
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err(Error::new(format!("{uri} has no host")));
    }
    let (name, port) =
        split_host_port(authority).map_err(|_| Error::new(format!("{uri} has no host")))?;
    if name.is_empty() {
        return Err(Error::new(format!("{uri} has no host")));
    }
    let address = name
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| ip.to_string());
    Ok(Some(Host {
        name,
        port,
        address,
    }))
}

fn split_host_port(authority: &str) -> Result<(String, u16), Error> {
    let Some((host, port)) = authority.rsplit_once(':') else {
        return Ok((authority.to_string(), IPP_PORT));
    };
    if host.is_empty() || host.contains(':') {
        return Ok((authority.to_string(), IPP_PORT));
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| Error::new(format!("{authority} has no port")))?;
    Ok((host.to_string(), port))
}

/// Store the device URI. `host` is the IPP host, or empty for any other scheme.
pub fn apply_device_uri(queue: &mut Queue, uri: &str) -> Result<(), Error> {
    queue.device_uri = uri.to_string();
    queue.host = host_from_uri(uri)?
        .map(|host| host.name)
        .unwrap_or_default();
    Ok(())
}

/// `CUPS-Get-PPD` for one queue. `PageSize` is omitted. A missing PPD leaves `choices` empty
/// and leaves the rest of the record as it was.
pub fn fill_choices(queue: &mut Queue, base: &str) -> Result<(), Error> {
    if queue.device_uri.is_empty() {
        queue.choices.clear();
        return Ok(());
    }
    queue.choices = ppd_choices(base, &queue.device_uri)?;
    Ok(())
}

fn ppd_choices(base: &str, printer_uri: &str) -> Result<Vec<Choice>, Error> {
    let response = post_ipp(
        base,
        &encode_operation(CUPS_GET_PPD, &[("printer-uri", 0x45, printer_uri)]),
    )?;
    let end = message_end(&response)?;
    let message = ipp_tool::decode(&response[..end]).map_err(|err| Error::new(err.to_string()))?;
    if message.code >= 0x0400 {
        return Ok(Vec::new());
    }
    let ppd = std::str::from_utf8(&response[end..]).map_err(|_| Error::new("PPD is not UTF-8"))?;
    Ok(choices_from_ppd(ppd))
}

/// PPD `*OpenUI` options. `PageSize` is left out because media size comes from the job layout.
fn choices_from_ppd(ppd: &str) -> Vec<Choice> {
    let mut choices = Vec::new();
    let mut current: Option<String> = None;
    let mut values = Vec::new();
    for line in ppd.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("*OpenUI *") {
            push_choice(&mut choices, &mut current, &mut values);
            let name = rest.split(['/', ':']).next().unwrap_or("").trim();
            current = if name.is_empty() || name == "PageSize" {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }
        if line.starts_with("*CloseUI") {
            push_choice(&mut choices, &mut current, &mut values);
            continue;
        }
        let Some(name) = current.as_deref() else {
            continue;
        };
        let Some(rest) = line.strip_prefix(&format!("*{name} ")) else {
            continue;
        };
        let value = rest.split(['/', ':']).next().unwrap_or("").trim();
        if value.is_empty() || values.iter().any(|have| have == value) {
            continue;
        }
        values.push(value.to_string());
    }
    push_choice(&mut choices, &mut current, &mut values);
    choices
}

fn push_choice(choices: &mut Vec<Choice>, current: &mut Option<String>, values: &mut Vec<String>) {
    let Some(name) = current.take() else {
        values.clear();
        return;
    };
    if !values.is_empty() {
        choices.push(Choice {
            name,
            values: std::mem::take(values),
        });
    }
    values.clear();
}

fn message_end(body: &[u8]) -> Result<usize, Error> {
    if body.len() < 9 {
        return Err(Error::new("IPP response is truncated"));
    }
    let mut index = 8;
    loop {
        if index >= body.len() {
            return Err(Error::new("IPP response is truncated"));
        }
        let tag = body[index];
        index += 1;
        if tag == 0x03 {
            return Ok(index);
        }
        if tag < 0x10 {
            continue;
        }
        index = skip_counted(body, index)?;
        index = skip_counted(body, index)?;
    }
}

fn skip_counted(body: &[u8], index: usize) -> Result<usize, Error> {
    if index + 2 > body.len() {
        return Err(Error::new("IPP response is truncated"));
    }
    let len = u16::from_be_bytes([body[index], body[index + 1]]) as usize;
    let next = index + 2 + len;
    if next > body.len() {
        return Err(Error::new("IPP response is truncated"));
    }
    Ok(next)
}

/// `CUPS-Get-Devices` against a cupsd base. One device-attribute group becomes one record.
fn list_devices(service: Option<&str>) -> Result<Vec<Device>, Error> {
    let base = service.unwrap_or("http://127.0.0.1:631");
    let response = post_ipp(base, &encode_operation(CUPS_GET_DEVICES, &[]))?;
    let message = ipp_tool::decode(&response).map_err(|err| Error::new(err.to_string()))?;
    if message.code >= 0x0400 {
        return Err(Error::new(format!(
            "CUPS-Get-Devices status {:#06x}",
            message.code
        )));
    }
    Ok(message
        .groups
        .iter()
        .filter_map(|group| {
            let uri = attr_str(group, "device-uri")?;
            if uri.is_empty() {
                return None;
            }
            let make_model = attr_str(group, "device-make-and-model")
                .filter(|text| !text.is_empty())
                .or_else(|| attr_str(group, "device-info"))
                .unwrap_or_default();
            Some(Device { uri, make_model })
        })
        .collect())
}

/// One IPP request to cupsd. The caller names the operation.
pub(crate) fn cups_call(
    base: &str,
    operation: u16,
    attributes: &[(&str, u8, &str)],
) -> Result<Vec<u8>, Error> {
    post_ipp(base, &encode_operation(operation, attributes))
}

/// One already-encoded IPP request to cupsd, including a document after the attributes.
pub(crate) fn cups_submit(base: &str, body: &[u8]) -> Result<Vec<u8>, Error> {
    post_ipp(base, body)
}

/// `CUPS-Get-Printers` against a cupsd base such as `http://127.0.0.1:631`.
/// One printer-attribute group becomes one record. This does not send
/// `CUPS-Add-Modify-Printer` or `Print-Job`.
pub fn cups_queues(base: &str) -> Result<Vec<Queue>, Error> {
    let response = post_ipp(base, &encode_operation(CUPS_GET_PRINTERS, &[]))?;
    let message = ipp_tool::decode(&response).map_err(|err| Error::new(err.to_string()))?;
    if message.code >= 0x0400 {
        return Err(Error::new(format!(
            "CUPS-Get-Printers status {:#06x}",
            message.code
        )));
    }
    message
        .groups
        .iter()
        .filter(|group| group.tag == PRINTER_ATTRIBUTES)
        .map(queue_from_group)
        .collect()
}

fn queue_from_group(group: &ipp_tool::Group) -> Result<Queue, Error> {
    let make_model = attr_str(group, "printer-make-and-model").unwrap_or_default();
    let device_uri = attr_str(group, "device-uri").unwrap_or_default();
    let (kind, dpi, width_in) = classify(&make_model);
    let host = host_from_uri(&device_uri)?
        .map(|host| host.name)
        .unwrap_or_default();
    Ok(Queue {
        name: attr_str(group, "printer-name").unwrap_or_default(),
        device_uri,
        make_model,
        state: printer_state(attr_enum(group, "printer-state")),
        accepting: attr_bool(group, "printer-is-accepting-jobs").unwrap_or(false),
        host,
        kind,
        dpi,
        width_in,
        choices: Vec::new(),
        installed: true,
        connected: false,
        jobs: None,
    })
}

fn printer_state(state: Option<i32>) -> String {
    match state {
        Some(3) => "idle".to_string(),
        Some(4) => "processing".to_string(),
        Some(5) => "stopped".to_string(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn attr<'a>(group: &'a ipp_tool::Group, name: &str) -> Option<&'a ipp_tool::Value> {
    group
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.values.first())
}

fn attr_str(group: &ipp_tool::Group, name: &str) -> Option<String> {
    attr(group, name)?.as_str().map(str::to_string)
}

fn attr_bool(group: &ipp_tool::Group, name: &str) -> Option<bool> {
    match attr(group, name)? {
        ipp_tool::Value::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn attr_enum(group: &ipp_tool::Group, name: &str) -> Option<i32> {
    match attr(group, name)? {
        ipp_tool::Value::Enum(value) | ipp_tool::Value::Integer(value) => Some(*value),
        _ => None,
    }
}

fn encode_operation(operation: u16, attributes: &[(&str, u8, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x02, 0x00]);
    out.extend_from_slice(&operation.to_be_bytes());
    out.extend_from_slice(&1i32.to_be_bytes());
    out.push(0x01);
    write_string(&mut out, 0x47, "attributes-charset", "utf-8");
    write_string(&mut out, 0x48, "attributes-natural-language", "en");
    for (name, tag, value) in attributes {
        write_string(&mut out, *tag, name, value);
    }
    out.push(0x03);
    out
}

fn write_string(out: &mut Vec<u8>, tag: u8, name: &str, value: &str) {
    out.push(tag);
    write_counted(out, name.as_bytes());
    write_counted(out, value.as_bytes());
}

fn write_counted(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn post_ipp(base: &str, body: &[u8]) -> Result<Vec<u8>, Error> {
    let target = HttpTarget::parse(base)?;
    let mut stream = TcpStream::connect_timeout(&target.addr, Duration::from_secs(2))
        .map_err(|err| Error::new(format!("connecting to {base}: {err}")))?;
    let budget = ipp_tool::body_budget(body.len());
    stream
        .set_read_timeout(Some(budget))
        .map_err(|err| Error::new(format!("setting read timeout: {err}")))?;
    stream
        .set_write_timeout(Some(budget))
        .map_err(|err| Error::new(format!("setting write timeout: {err}")))?;
    let header = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        target.path,
        target.host,
        target.port,
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|err| Error::new(format!("sending IPP request: {err}")))?;
    let (status, response) = read_http(&mut stream)?;
    if !(200..300).contains(&status) {
        return Err(Error::new(format!("HTTP status {status}")));
    }
    Ok(response)
}

struct HttpTarget {
    host: String,
    port: u16,
    path: String,
    addr: std::net::SocketAddr,
}

impl HttpTarget {
    fn parse(base: &str) -> Result<Self, Error> {
        let rest = base
            .strip_prefix("http://")
            .ok_or_else(|| Error::new(format!("{base} is not an http base")))?;
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, "/".to_string()),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() && !host.contains(':') => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| Error::new(format!("{base} has no port")))?;
                (host.to_string(), port)
            }
            _ => (authority.to_string(), IPP_PORT),
        };
        if host.is_empty() {
            return Err(Error::new(format!("{base} has no host")));
        }
        let addr = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|err| Error::new(format!("resolving {host}: {err}")))?
            .next()
            .ok_or_else(|| Error::new(format!("resolving {host} returned no address")))?;
        Ok(Self {
            host,
            port,
            path,
            addr,
        })
    }
}

fn read_http(stream: &mut TcpStream) -> Result<(u16, Vec<u8>), Error> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        if let Some(parsed) = frame(&buf)? {
            return Ok(parsed);
        }
        match stream.read(&mut tmp) {
            Ok(0) => {
                return frame(&buf)?
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

fn frame(buf: &[u8]) -> Result<Option<(u16, Vec<u8>)>, Error> {
    let Some(end) = buf.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Ok(None);
    };
    let head =
        std::str::from_utf8(&buf[..end]).map_err(|_| Error::new("HTTP head is not UTF-8"))?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| Error::new("HTTP status is missing"))?;
    let length = content_length(head)?;
    let body_at = end + 4;
    if buf.len() < body_at + length {
        return Ok(None);
    }
    Ok(Some((status, buf[body_at..body_at + length].to_vec())))
}

fn content_length(head: &str) -> Result<usize, Error> {
    for line in head.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse()
                .map_err(|_| Error::new("Content-Length is not a number"));
        }
    }
    Err(Error::new("Content-Length is missing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    mod record {
        use super::*;

        #[test]
        fn zebra_and_tsc_are_zpl() {
            let zebra = Queue::from_make("Zebra Technologies", "ZTC GX430t");
            assert_eq!(zebra.kind, Kind::Zpl);
            assert_eq!(zebra.kind.as_str(), "zpl");
            assert_eq!(zebra.dpi, 300);
            assert_eq!(zebra.width_in, Some(4.1));
            assert!(zebra.host.is_empty());
            assert!(zebra.choices.is_empty());
            assert!(zebra.installed);
            assert!(!zebra.connected);

            let tsc = Queue::from_make("TSC", "");
            assert_eq!(tsc.kind, Kind::Zpl);
            assert_eq!(tsc.dpi, 203);
            assert_eq!(tsc.width_in, Some(4.1));
        }

        #[test]
        fn evolis_is_raster() {
            let evolis = Queue::from_make("Evolis", "");
            assert_eq!(evolis.kind, Kind::Raster);
            assert_eq!(evolis.kind.as_str(), "raster");
            assert_eq!(evolis.dpi, 300);
            assert_eq!(evolis.width_in, Some(2.125));
        }

        #[test]
        fn another_make_is_raster() {
            let hp = Queue::from_make("HP", "");
            assert_eq!(hp.kind, Kind::Raster);
            assert_eq!(hp.dpi, 300);
            assert!(hp.width_in.is_none());
        }
    }

    mod host {
        use super::*;

        #[test]
        fn an_ipp_uri_yields_the_address() {
            let literal = host_from_uri("ipp://192.0.2.10/ipp/print")
                .unwrap()
                .unwrap();
            assert_eq!(literal.name, "192.0.2.10");
            assert_eq!(literal.port, 631);
            assert_eq!(literal.address.as_deref(), Some("192.0.2.10"));

            let named = host_from_uri("ipps://printer.local:443/ipp/print")
                .unwrap()
                .unwrap();
            assert_eq!(named.name, "printer.local");
            assert_eq!(named.port, 443);
            assert!(named.address.is_none());
            let resolved = named.with_address("192.0.2.20");
            assert_eq!(resolved.name, "printer.local");
            assert_eq!(resolved.port, 443);
            assert_eq!(resolved.address.as_deref(), Some("192.0.2.20"));

            let missing = host_from_uri("ipp:///ipp/print").unwrap_err();
            assert!(missing.to_string().contains("ipp:///ipp/print"));
        }

        #[test]
        fn usb_and_socket_have_no_host() {
            for uri in [
                "usb://Zebra/ZTC?serial=ABC",
                "socket://192.0.2.10:9100",
                "dnssd://Zebra._ipp._tcp.local./",
            ] {
                assert!(host_from_uri(uri).unwrap().is_none());
                let mut queue = Queue::from_make("Zebra Technologies", "ZTC GX430t");
                apply_device_uri(&mut queue, uri).unwrap();
                assert!(queue.host.is_empty());
                assert_eq!(queue.device_uri, uri);
            }
        }
    }

    mod cups {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};
        use std::thread;
        use std::time::Duration;

        use super::*;

        pub(super) struct Listed {
            name: &'static str,
            uri: &'static str,
            make_model: &'static str,
            state: i32,
            accepting: bool,
        }

        pub(super) fn sample() -> Vec<Listed> {
            vec![
                Listed {
                    name: "zebra",
                    uri: "usb://Zebra/ZTC?serial=ABC",
                    make_model: "Zebra Technologies ZTC GX430t",
                    state: 3,
                    accepting: true,
                },
                Listed {
                    name: "ipp",
                    uri: "ipp://192.0.2.10/ipp/print",
                    make_model: "HP LaserJet",
                    state: 4,
                    accepting: true,
                },
                Listed {
                    name: "evolis",
                    uri: "usb://Evolis/Zenius?serial=E1",
                    make_model: "Evolis Zenius",
                    state: 5,
                    accepting: false,
                },
            ]
        }

        pub(super) fn serve(
            response: Vec<u8>,
        ) -> (String, Arc<Mutex<Vec<u16>>>, thread::JoinHandle<()>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let ops = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&ops);
            let handle = thread::spawn(move || {
                let (mut sock, _) = listener.accept().unwrap();
                sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let body = read_body(&mut sock);
                recorded.lock().unwrap().push(operation_id(&body));
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                sock.write_all(head.as_bytes()).unwrap();
                sock.write_all(&response).unwrap();
            });
            (format!("http://{addr}"), ops, handle)
        }

        fn read_body(sock: &mut std::net::TcpStream) -> Vec<u8> {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                if let Some(body) = request_body(&buf) {
                    return body;
                }
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => return buf,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            }
        }

        fn request_body(buf: &[u8]) -> Option<Vec<u8>> {
            let end = buf.windows(4).position(|window| window == b"\r\n\r\n")?;
            let head = std::str::from_utf8(&buf[..end]).ok()?;
            let length = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })?;
            let start = end + 4;
            if buf.len() < start + length {
                return None;
            }
            Some(buf[start..start + length].to_vec())
        }

        fn operation_id(body: &[u8]) -> u16 {
            if body.len() < 4 {
                return 0;
            }
            u16::from_be_bytes([body[2], body[3]])
        }

        pub(super) fn cups_response(printers: &[Listed]) -> Vec<u8> {
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            for printer in printers {
                out.push(0x04);
                write_text(&mut out, 0x42, "printer-name", printer.name);
                write_text(&mut out, 0x45, "device-uri", printer.uri);
                write_text(&mut out, 0x41, "printer-make-and-model", printer.make_model);
                write_enum(&mut out, "printer-state", printer.state);
                write_bool(&mut out, "printer-is-accepting-jobs", printer.accepting);
            }
            out.push(0x03);
            out
        }

        fn write_text(out: &mut Vec<u8>, tag: u8, name: &str, value: &str) {
            out.push(tag);
            counted(out, name.as_bytes());
            counted(out, value.as_bytes());
        }

        fn write_enum(out: &mut Vec<u8>, name: &str, value: i32) {
            out.push(0x23);
            counted(out, name.as_bytes());
            out.extend_from_slice(&4u16.to_be_bytes());
            out.extend_from_slice(&value.to_be_bytes());
        }

        fn write_bool(out: &mut Vec<u8>, name: &str, value: bool) {
            out.push(0x22);
            counted(out, name.as_bytes());
            out.extend_from_slice(&1u16.to_be_bytes());
            out.push(u8::from(value));
        }

        fn counted(out: &mut Vec<u8>, bytes: &[u8]) {
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            out.extend_from_slice(bytes);
        }

        #[test]
        fn queues_become_records() {
            let (base, _, handle) = serve(cups_response(&sample()));
            let queues = cups_queues(&base).unwrap();
            handle.join().unwrap();

            let zebra = queues.iter().find(|queue| queue.name == "zebra").unwrap();
            assert_eq!(zebra.device_uri, "usb://Zebra/ZTC?serial=ABC");
            assert_eq!(zebra.kind, Kind::Zpl);
            assert_eq!(zebra.dpi, 300);
            assert_eq!(zebra.width_in, Some(4.1));
            assert!(zebra.host.is_empty());
            assert_eq!(zebra.state, "idle");
            assert!(zebra.accepting);
            assert!(zebra.installed);
            assert!(!zebra.connected);

            let ipp = queues.iter().find(|queue| queue.name == "ipp").unwrap();
            assert_eq!(ipp.host, "192.0.2.10");
            assert_eq!(ipp.state, "processing");
            assert!(ipp.accepting);

            let evolis = queues.iter().find(|queue| queue.name == "evolis").unwrap();
            assert_eq!(evolis.kind, Kind::Raster);
            assert_eq!(evolis.dpi, 300);
            assert_eq!(evolis.width_in, Some(2.125));
            assert_eq!(evolis.state, "stopped");
            assert!(!evolis.accepting);
        }

        #[test]
        fn get_printers_is_the_only_call() {
            let (base, ops, handle) = serve(cups_response(&sample()));
            cups_queues(&base).unwrap();
            handle.join().unwrap();
            let ops = ops.lock().unwrap().clone();
            assert_eq!(ops, vec![0x4002]);
            assert!(!ops.contains(&0x0002));
        }
    }

    mod choices {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;
        use std::time::Duration;

        use super::*;

        const PPD: &str = "\
*OpenUI *PageSize/Media Size: PickOne
*DefaultPageSize: Letter
*PageSize Letter/US Letter: \"\"
*PageSize A4/A4: \"\"
*CloseUI: *PageSize
*OpenUI *Resolution/Output Resolution: PickOne
*DefaultResolution: 300dpi
*Resolution 300dpi/300 DPI: \"\"
*Resolution 600dpi/600 DPI: \"\"
*CloseUI: *Resolution
";

        fn serve(status: u16, ppd: &str) -> (String, thread::JoinHandle<()>) {
            let mut body = vec![0x02, 0x00];
            body.extend_from_slice(&status.to_be_bytes());
            body.extend_from_slice(&1i32.to_be_bytes());
            body.push(0x01);
            write_text(&mut body, 0x47, "attributes-charset", "utf-8");
            write_text(&mut body, 0x48, "attributes-natural-language", "en");
            body.push(0x03);
            body.extend_from_slice(ppd.as_bytes());
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = thread::spawn(move || {
                let (mut sock, _) = listener.accept().unwrap();
                sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    if request_complete(&buf) {
                        break;
                    }
                    match sock.read(&mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                sock.write_all(head.as_bytes()).unwrap();
                sock.write_all(&body).unwrap();
            });
            (format!("http://{addr}"), handle)
        }

        fn request_complete(buf: &[u8]) -> bool {
            let Some(end) = buf.windows(4).position(|window| window == b"\r\n\r\n") else {
                return false;
            };
            let Ok(head) = std::str::from_utf8(&buf[..end]) else {
                return false;
            };
            let Some(length) = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            }) else {
                return false;
            };
            buf.len() >= end + 4 + length
        }

        fn write_text(out: &mut Vec<u8>, tag: u8, name: &str, value: &str) {
            out.push(tag);
            counted(out, name.as_bytes());
            counted(out, value.as_bytes());
        }

        fn counted(out: &mut Vec<u8>, bytes: &[u8]) {
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            out.extend_from_slice(bytes);
        }

        #[test]
        fn page_size_is_omitted() {
            let (base, handle) = serve(0x0000, PPD);
            let mut queue = Queue::from_make("HP", "LaserJet");
            queue.device_uri = "ipp://127.0.0.1/printers/hp".to_string();
            fill_choices(&mut queue, &base).unwrap();
            handle.join().unwrap();
            assert_eq!(queue.choices.len(), 1);
            assert_eq!(queue.choices[0].name, "Resolution");
            assert_eq!(
                queue.choices[0].values,
                vec!["300dpi".to_string(), "600dpi".to_string()]
            );
            assert!(queue.choices.iter().all(|choice| choice.name != "PageSize"));
            assert_eq!(queue.kind, Kind::Raster);
        }

        #[test]
        fn a_missing_ppd_leaves_choices_empty() {
            let (base, handle) = serve(0x0404, "");
            let mut queue = Queue::from_make("Zebra Technologies", "ZTC GX430t");
            queue.device_uri = "usb://Zebra/ZTC?serial=ABC".to_string();
            let kind = queue.kind;
            let dpi = queue.dpi;
            let width = queue.width_in;
            fill_choices(&mut queue, &base).unwrap();
            handle.join().unwrap();
            assert!(queue.choices.is_empty());
            assert_eq!(queue.kind, kind);
            assert_eq!(queue.dpi, dpi);
            assert_eq!(queue.width_in, width);
        }
    }

    mod list {
        use super::super::{list, Kind, CUPS_BASE};
        use super::cups::{cups_response, sample, serve};

        #[test]
        fn unix_list_is_cups() {
            let (base, ops, handle) = serve(cups_response(&sample()));
            CUPS_BASE.with(|slot| *slot.borrow_mut() = Some(base));
            let queues = list().unwrap();
            CUPS_BASE.with(|slot| *slot.borrow_mut() = None);
            handle.join().unwrap();

            let ops = ops.lock().unwrap().clone();
            assert_eq!(ops, vec![0x4002]);
            assert!(!ops.contains(&0x0002));

            let zebra = queues.iter().find(|queue| queue.name == "zebra").unwrap();
            assert_eq!(zebra.device_uri, "usb://Zebra/ZTC?serial=ABC");
            assert_eq!(zebra.kind, Kind::Zpl);
            assert_eq!(zebra.dpi, 300);
            assert_eq!(zebra.width_in, Some(4.1));
            assert!(zebra.host.is_empty());

            let ipp = queues.iter().find(|queue| queue.name == "ipp").unwrap();
            assert_eq!(ipp.host, "192.0.2.10");

            let evolis = queues.iter().find(|queue| queue.name == "evolis").unwrap();
            assert_eq!(evolis.kind, Kind::Raster);
            assert_eq!(evolis.width_in, Some(2.125));
        }
    }
}
