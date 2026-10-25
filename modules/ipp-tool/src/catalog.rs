//! Browse, then `Get-Printer-Attributes`. Tests pass the browse records.
//! A result that does not answer is left out. `add` queries one URI and does not browse.

use crate::record::{Error, Printer};

/// One DNS-SD answer. `kind` is `_ipp._tcp` or `_ipps._tcp`. `host` is what `info` connects to.
/// `address` is the resolved address when it differs from `host`. Absent `rp` means `ipp/print`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub rp: Option<String>,
    pub address: Option<String>,
}

pub trait Browse {
    fn browse(&self) -> Vec<Service>;
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    printers: Vec<Printer>,
}

impl Catalog {
    pub fn from_printers(printers: Vec<Printer>) -> Self {
        Self { printers }
    }

    pub fn discover(browse: &impl Browse) -> Result<Self, Error> {
        let mut catalog = Self::default();
        catalog.ingest(browse.browse());
        Ok(catalog)
    }

    pub fn add(&mut self, uri: &str) -> Result<(), Error> {
        let mut printer = Printer::from_uri(uri)?;
        printer.info()?;
        if printer.name.is_empty() {
            printer.name = printer.make_model.clone();
        }
        self.insert(printer);
        Ok(())
    }

    /// Query printers already known, then browse again. A printer that does not answer is dropped.
    pub fn refresh(&mut self, browse: &impl Browse) -> Result<(), Error> {
        let mut kept = Vec::new();
        for mut printer in std::mem::take(&mut self.printers) {
            if printer.info().is_ok() {
                kept.push(printer);
            }
        }
        self.printers = kept;
        self.ingest(browse.browse());
        Ok(())
    }

    pub fn printers(&self) -> &[Printer] {
        &self.printers
    }

    fn ingest(&mut self, services: Vec<Service>) {
        for service in collapse(services) {
            if let Ok(printer) = load_service(&service) {
                self.insert(printer);
            }
        }
    }

    fn insert(&mut self, printer: Printer) {
        if let Some(existing) = self
            .printers
            .iter_mut()
            .find(|item| item.address == printer.address && item.port == printer.port)
        {
            if printer.secure || !existing.secure {
                *existing = printer;
            }
            return;
        }
        self.printers.push(printer);
    }
}

fn collapse(services: Vec<Service>) -> Vec<Service> {
    let mut chosen = Vec::new();
    for mut service in services {
        if !is_ipp(&service.kind) {
            continue;
        }
        if service.port == 0 {
            service.port = 631;
        }
        if service.address.is_none() {
            service.address = Some(service.host.clone());
        }
        let address = service.address.clone().unwrap_or_default();
        if let Some(existing) = chosen.iter_mut().find(|item: &&mut Service| {
            item.address.as_deref() == Some(address.as_str()) && item.port == service.port
        }) {
            if service.kind.contains("ipps") {
                *existing = service;
            }
            continue;
        }
        chosen.push(service);
    }
    chosen
}

fn is_ipp(kind: &str) -> bool {
    kind.contains("_ipp._tcp") || kind.contains("_ipps._tcp")
}

fn load_service(service: &Service) -> Result<Printer, Error> {
    let mut printer = Printer::from_uri(&uri_for(service))?;
    if let Some(address) = &service.address {
        printer.address = address.clone();
    }
    printer.info()?;
    if printer.name.is_empty() {
        printer.name = printer.make_model.clone();
    }
    Ok(printer)
}

/// One printer from a listing, then `Finished`. `Missed` is one URI that did not answer.
/// `Failed` replaces `Finished` when the listing itself stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Printer(Printer),
    Missed(String),
    Finished,
    Failed(String),
}

/// Browse results and `details` requests, one printer at a time.
pub struct Watch {
    events: std::sync::mpsc::Receiver<Report>,
    details: Option<std::sync::mpsc::Sender<String>>,
}

impl Watch {
    pub fn recv(&self) -> Result<Report, std::sync::mpsc::RecvError> {
        self.events.recv()
    }

    pub fn try_recv(&self) -> Result<Report, std::sync::mpsc::TryRecvError> {
        self.events.try_recv()
    }

    /// The listing keeps answering until this sender is dropped.
    pub fn take_details(&mut self) -> std::sync::mpsc::Sender<String> {
        self.details.take().expect("details sender")
    }
}

impl Catalog {
    /// `known` printers are queried again. A printer that does not answer is left out.
    /// `Finished` follows the last listing result. `details` is still answered after that.
    pub fn watch(browse: std::sync::Arc<dyn Browse + Send + Sync>, known: Vec<Printer>) -> Watch {
        let (events_tx, events_rx) = std::sync::mpsc::channel();
        let (details_tx, details_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || run_watch(browse, known, details_rx, events_tx));
        Watch {
            events: events_rx,
            details: Some(details_tx),
        }
    }
}

struct Seen {
    key: String,
    printer: Option<Printer>,
}

fn run_watch(
    browse: std::sync::Arc<dyn Browse + Send + Sync>,
    known: Vec<Printer>,
    details_rx: std::sync::mpsc::Receiver<String>,
    events_tx: std::sync::mpsc::Sender<Report>,
) {
    let mut seen = Vec::new();
    for printer in known {
        drain_details(&details_rx, &mut seen, &events_tx);
        if !emit_known(&printer, &mut seen, &events_tx) {
            return;
        }
    }
    let services = browse.browse();
    for service in collapse(services) {
        drain_details(&details_rx, &mut seen, &events_tx);
        if !emit_service(&service, &mut seen, &events_tx) {
            return;
        }
    }
    drain_details(&details_rx, &mut seen, &events_tx);
    if events_tx.send(Report::Finished).is_err() {
        return;
    }
    while let Ok(uri) = details_rx.recv() {
        if !answer_uri(&uri, &mut seen, &events_tx) {
            return;
        }
    }
}

fn drain_details(
    details_rx: &std::sync::mpsc::Receiver<String>,
    seen: &mut Vec<Seen>,
    events_tx: &std::sync::mpsc::Sender<Report>,
) {
    while let Ok(uri) = details_rx.try_recv() {
        if !answer_uri(&uri, seen, events_tx) {
            return;
        }
    }
}

fn emit_known(
    printer: &Printer,
    seen: &mut Vec<Seen>,
    events_tx: &std::sync::mpsc::Sender<Report>,
) -> bool {
    let key = seen_key(&printer.address, printer.port);
    if seen.iter().any(|item| item.key == key) {
        return true;
    }
    match reload(printer) {
        Ok(found) => {
            seen.push(Seen {
                key,
                printer: Some(found.clone()),
            });
            events_tx.send(Report::Printer(found)).is_ok()
        }
        Err(_) => {
            seen.push(Seen { key, printer: None });
            true
        }
    }
}

fn emit_service(
    service: &Service,
    seen: &mut Vec<Seen>,
    events_tx: &std::sync::mpsc::Sender<Report>,
) -> bool {
    let address = service
        .address
        .clone()
        .unwrap_or_else(|| service.host.clone());
    let key = seen_key(&address, service.port);
    if let Some(item) = seen.iter().find(|item| item.key == key) {
        if let Some(printer) = &item.printer {
            return events_tx.send(Report::Printer(printer.clone())).is_ok();
        }
        return true;
    }
    match load_service(service) {
        Ok(found) => {
            seen.push(Seen {
                key,
                printer: Some(found.clone()),
            });
            events_tx.send(Report::Printer(found)).is_ok()
        }
        Err(_) => {
            seen.push(Seen { key, printer: None });
            true
        }
    }
}

fn answer_uri(
    uri: &str,
    seen: &mut Vec<Seen>,
    events_tx: &std::sync::mpsc::Sender<Report>,
) -> bool {
    let probe = match Printer::from_uri(uri) {
        Ok(printer) => printer,
        Err(_) => return events_tx.send(Report::Missed(uri.to_string())).is_ok(),
    };
    let key = seen_key(&probe.address, probe.port);
    if let Some(item) = seen.iter().find(|item| item.key == key) {
        return match &item.printer {
            Some(printer) => events_tx.send(Report::Printer(printer.clone())).is_ok(),
            None => events_tx.send(Report::Missed(uri.to_string())).is_ok(),
        };
    }
    match load_uri(uri) {
        Ok(found) => {
            seen.push(Seen {
                key,
                printer: Some(found.clone()),
            });
            events_tx.send(Report::Printer(found)).is_ok()
        }
        Err(_) => {
            seen.push(Seen { key, printer: None });
            events_tx.send(Report::Missed(uri.to_string())).is_ok()
        }
    }
}

fn reload(printer: &Printer) -> Result<Printer, Error> {
    let mut found = printer.clone();
    found.info()?;
    if found.name.is_empty() {
        found.name = found.make_model.clone();
    }
    Ok(found)
}

fn load_uri(uri: &str) -> Result<Printer, Error> {
    let mut printer = Printer::from_uri(uri)?;
    printer.info()?;
    if printer.name.is_empty() {
        printer.name = printer.make_model.clone();
    }
    Ok(printer)
}

fn seen_key(address: &str, port: u16) -> String {
    format!("{address}:{port}")
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::message::{encode_request, Operation, Value};

    struct Fixed(Vec<Service>);

    impl Browse for Fixed {
        fn browse(&self) -> Vec<Service> {
            self.0.clone()
        }
    }

    fn service(kind: &str, host: &str, port: u16) -> Service {
        Service {
            kind: kind.to_string(),
            host: host.to_string(),
            port,
            rp: Some("ipp/print".to_string()),
            address: Some(host.to_string()),
        }
    }

    fn make_response(make: &str, jobs: i32) -> Vec<u8> {
        encode_request(
            Operation::GetPrinterAttributes,
            1,
            &[],
            &[
                (
                    "printer-make-and-model",
                    vec![Value::Name(make.to_string())],
                ),
                ("queued-job-count", vec![Value::Integer(jobs)]),
            ],
        )
    }

    fn serve(ipp: Vec<u8>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                let ready = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .and_then(|split| {
                        let head = std::str::from_utf8(&request[..split]).ok()?;
                        let length = head.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            if name.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })?;
                        (request.len() >= split + 4 + length).then_some(())
                    });
                if ready.is_some() {
                    break;
                }
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => request.extend_from_slice(&tmp[..n]),
                }
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                ipp.len()
            );
            let _ = sock.write_all(header.as_bytes());
            let _ = sock.write_all(&ipp);
        });
        port
    }

    #[test]
    fn ipps_replaces_ipp() {
        let kept = collapse(vec![
            service("_ipp._tcp", "192.0.2.10", 631),
            service("_ipps._tcp", "192.0.2.10", 631),
        ]);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].kind.contains("ipps"), "{}", kept[0].kind);
    }

    #[test]
    fn two_ports_stay_two_printers() {
        let kept = collapse(vec![
            service("_ipp._tcp", "192.0.2.10", 631),
            service("_ipp._tcp", "192.0.2.10", 443),
        ]);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn a_browse_becomes_a_printer() {
        let port = serve(make_response("Zebra", 1));
        let catalog =
            Catalog::discover(&Fixed(vec![service("_ipp._tcp", "127.0.0.1", port)])).unwrap();
        assert_eq!(catalog.printers().len(), 1);
        assert_eq!(catalog.printers()[0].make_model, "Zebra");
        assert_eq!(
            catalog.printers()[0].uri,
            format!("ipp://127.0.0.1:{port}/ipp/print")
        );
    }

    #[test]
    fn lpd_is_ignored() {
        let catalog =
            Catalog::discover(&Fixed(vec![service("_printer._tcp", "127.0.0.1", 9)])).unwrap();
        assert!(catalog.printers().is_empty());
    }

    #[test]
    fn a_silent_browse_result_is_omitted() {
        let port = serve(make_response("Zebra", 1));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((sock, _)) = listener.accept() {
                drop(sock);
            }
        });
        let catalog = Catalog::discover(&Fixed(vec![
            service("_ipp._tcp", "127.0.0.1", port),
            service("_ipp._tcp", "127.0.0.1", dead),
        ]))
        .unwrap();
        assert_eq!(catalog.printers().len(), 1);
        assert_eq!(catalog.printers()[0].make_model, "Zebra");
    }

    #[test]
    fn refresh_drops_a_silent_printer() {
        let port = serve(make_response("Zebra", 3));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((sock, _)) = listener.accept() {
                drop(sock);
            }
        });
        let live = Printer::from_uri(&format!("ipp://127.0.0.1:{port}/ipp/print")).unwrap();
        let quiet = Printer::from_uri(&format!("ipp://127.0.0.1:{dead}/ipp/print")).unwrap();
        let mut catalog = Catalog::from_printers(vec![live, quiet]);
        catalog.refresh(&Fixed(Vec::new())).unwrap();
        assert_eq!(catalog.printers().len(), 1);
        assert_eq!(catalog.printers()[0].settings.queued_job_count, Some(3));
    }

    #[test]
    fn add_takes_a_uri() {
        let port = serve(make_response("Desk", 0));
        let mut catalog = Catalog::default();
        catalog
            .add(&format!("ipp://127.0.0.1:{port}/ipp/print"))
            .unwrap();
        assert_eq!(catalog.printers().len(), 1);
        assert_eq!(catalog.printers()[0].make_model, "Desk");
    }

    #[test]
    fn a_dead_uri_adds_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut catalog = Catalog::default();
        assert!(catalog
            .add(&format!("ipp://127.0.0.1:{port}/ipp/print"))
            .is_err());
        assert!(catalog.printers().is_empty());
    }
}

fn uri_for(service: &Service) -> String {
    let scheme = if service.kind.contains("ipps") {
        "ipps"
    } else {
        "ipp"
    };
    let path = service.rp.as_deref().unwrap_or("ipp/print");
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let port = if service.port == 0 { 631 } else { service.port };
    format!("{scheme}://{}:{port}{path}", service.host)
}
