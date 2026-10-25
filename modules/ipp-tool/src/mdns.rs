//! DNS-SD browse for `_ipp._tcp` and `_ipps._tcp`.
//! A browse that cannot open a socket returns no records. `_printer._tcp` is not queried.

use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::catalog::{Browse, Service};

const TYPES: [&str; 2] = ["_ipp._tcp.local.", "_ipps._tcp.local."];
const WINDOW: Duration = Duration::from_millis(800);

/// Production browser. Tests pass their own [`Browse`] and do not open this one.
#[derive(Debug, Default)]
pub struct MdnsBrowse;

impl Browse for MdnsBrowse {
    fn browse(&self) -> Vec<Service> {
        browse_lan()
    }
}

fn browse_lan() -> Vec<Service> {
    let Ok(daemon) = ServiceDaemon::new() else {
        return Vec::new();
    };
    let mut receivers = Vec::new();
    for kind in TYPES {
        match daemon.browse(kind) {
            Ok(receiver) => receivers.push((kind, receiver)),
            Err(_) => {}
        }
    }
    let deadline = Instant::now() + WINDOW;
    let mut found = Vec::new();
    for (kind, receiver) in receivers {
        while Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match receiver.recv_timeout(left) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if let Some(service) = from_resolved(kind, &info) {
                        found.push(service);
                    }
                }
                Ok(ServiceEvent::SearchStopped(_)) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
    let _ = daemon.shutdown();
    found
}

fn from_resolved(kind: &str, info: &mdns_sd::ResolvedService) -> Option<Service> {
    if !info.is_valid() {
        return None;
    }
    let address = info
        .get_addresses_v4()
        .into_iter()
        .next()
        .map(|ip| ip.to_string());
    let name = info.get_hostname().trim_end_matches('.').to_string();
    let host = address.clone().filter(|ip| !ip.is_empty()).unwrap_or(name);
    let rp = info
        .get_property_val_str("rp")
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    service_record(kind, &host, info.get_port(), rp, address)
}

/// One browse answer. `_printer._tcp` and any other type are left out.
pub(crate) fn service_record(
    kind: &str,
    host: &str,
    port: u16,
    rp: Option<String>,
    address: Option<String>,
) -> Option<Service> {
    if kind.contains("_printer._tcp") {
        return None;
    }
    if !(kind.contains("_ipp._tcp") || kind.contains("_ipps._tcp")) {
        return None;
    }
    let host = host.trim_end_matches('.').to_string();
    if host.is_empty() {
        return None;
    }
    Some(Service {
        kind: kind
            .trim_end_matches('.')
            .trim_end_matches(".local")
            .to_string(),
        host,
        port,
        rp,
        address,
    })
}

#[cfg(test)]
mod tests {
    use super::service_record;

    #[test]
    fn an_ipp_answer_keeps_the_address() {
        let service = service_record(
            "_ipp._tcp.local.",
            "192.0.2.10",
            631,
            Some("ipp/print".to_string()),
            Some("192.0.2.10".to_string()),
        )
        .unwrap();
        assert_eq!(service.kind, "_ipp._tcp");
        assert_eq!(service.host, "192.0.2.10");
        assert_eq!(service.port, 631);
        assert_eq!(service.rp.as_deref(), Some("ipp/print"));
        assert_eq!(service.address.as_deref(), Some("192.0.2.10"));
    }

    #[test]
    fn an_ipps_answer_keeps_its_type() {
        let service = service_record("_ipps._tcp", "192.0.2.11", 443, None, None).unwrap();
        assert!(service.kind.contains("_ipps._tcp"));
        assert!(service.rp.is_none());
    }

    #[test]
    fn lpd_is_not_a_record() {
        assert!(service_record("_printer._tcp", "192.0.2.10", 515, None, None).is_none());
    }
}
