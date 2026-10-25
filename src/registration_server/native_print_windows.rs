//! Windows spooler. Steps 5 and 6 map a queue. Step 7 reads `EnumPrinters`.
//! Tests install a fixture, so they do not call `EnumPrinters` and they do not send IPP.

use std::cell::RefCell;

#[derive(Clone)]
struct SpoolerRow {
    name: String,
    driver: String,
    port: String,
    status: String,
    /// `PRINTER_INFO_2.cJobs`. The fixture stands in for `EnumPrinters`.
    jobs: u32,
}

thread_local! {
    static SPOOLER_FIXTURE: RefCell<Option<Vec<SpoolerRow>>> = const { RefCell::new(None) };
}

/// One spooler queue. The driver string fills kind, dpi, and width the same way a
/// CUPS make and model does. The port string becomes `device_uri` and, for IPP, `host`.
pub fn from_spooler(name: &str, driver: &str, port: &str, status: &str) -> Queue {
    let (kind, dpi, width_in) = classify(driver);
    let mut queue = Queue {
        name: name.to_string(),
        device_uri: String::new(),
        make_model: driver.to_string(),
        state: spooler_state(status),
        accepting: spooler_accepting(status),
        host: String::new(),
        kind,
        dpi,
        width_in,
        choices: Vec::new(),
        installed: true,
        connected: false,
        jobs: None,
    };
    apply_port(&mut queue, port);
    queue
}

fn spooler_state(status: &str) -> String {
    let lower = status.to_ascii_lowercase();
    if lower.contains("printing") || lower.contains("processing") {
        "processing".to_string()
    } else if lower.contains("offline")
        || lower.contains("not available")
        || lower.contains("error")
    {
        "stopped".to_string()
    } else {
        "idle".to_string()
    }
}

fn spooler_accepting(status: &str) -> bool {
    let lower = status.to_ascii_lowercase();
    !(lower.contains("paused") || lower.contains("offline") || lower.contains("not available"))
}

/// `ipp://` and `ipps://` keep the URI and set `host`. `https://` or `http://` whose
/// path contains `ipp` becomes `ipps://` or `ipp://`. A `USB` port becomes `usb://`.
/// An `IP_` port becomes `socket://` on port 9100 unless the name gives a port.
/// Anything else, including WSD, becomes `winspool://`. USB and socket leave `host` empty.
pub fn apply_port(queue: &mut Queue, port: &str) {
    if port.is_empty() {
        return;
    }
    let (device_uri, host) = device_uri_for(port, &queue.make_model);
    queue.device_uri = device_uri;
    queue.host = host;
}

fn device_uri_for(port: &str, driver: &str) -> (String, String) {
    if port.starts_with("ipp://") || port.starts_with("ipps://") {
        return (port.to_string(), ipp_host(port));
    }
    if let Some(rewritten) = rewrite_http_ipp(port) {
        return (rewritten.clone(), ipp_host(&rewritten));
    }
    if port.starts_with("USB") {
        let driver = driver.replace(' ', "_");
        return (format!("usb://{driver}/{port}"), String::new());
    }
    if let Some(socket) = socket_from_ip_port(port) {
        return (socket, String::new());
    }
    (format!("winspool://{port}"), String::new())
}

fn rewrite_http_ipp(port: &str) -> Option<String> {
    let (scheme, rest) = if let Some(rest) = port.strip_prefix("https://") {
        ("ipps", rest)
    } else if let Some(rest) = port.strip_prefix("http://") {
        ("ipp", rest)
    } else {
        return None;
    };
    let path = rest.split_once('/').map(|(_, path)| path).unwrap_or("");
    if !path.contains("ipp") {
        return None;
    }
    Some(format!("{scheme}://{rest}"))
}

fn ipp_host(uri: &str) -> String {
    let rest = uri
        .strip_prefix("ipp://")
        .or_else(|| uri.strip_prefix("ipps://"))
        .unwrap_or("");
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return String::new();
    }
    match authority.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty() && !host.contains(':') && port.parse::<u16>().is_ok() =>
        {
            host.to_string()
        }
        _ => authority.to_string(),
    }
}

fn socket_from_ip_port(port: &str) -> Option<String> {
    let rest = port.strip_prefix("IP_")?;
    if rest.is_empty() {
        return None;
    }
    let (address, printer_port) = if let Some((address, printer_port)) = rest.rsplit_once(':') {
        (address, printer_port)
    } else if let Some((address, printer_port)) = rest.rsplit_once('_') {
        if !address.is_empty() && printer_port.chars().all(|ch| ch.is_ascii_digit()) {
            (address, printer_port)
        } else {
            (rest, "9100")
        }
    } else {
        (rest, "9100")
    };
    let printer_port = printer_port.parse::<u16>().ok()?;
    if address.is_empty() {
        return None;
    }
    Some(format!("socket://{address}:{printer_port}"))
}

/// Local queues and connections. A test fixture stands in for `EnumPrinters`.
/// This does not send `CUPS-Get-Printers` or `Print-Job`.
pub fn list() -> Result<Vec<Queue>, Error> {
    list_service(None)
}

/// Windows has no CUPS device list. The watch finishes with no devices.
fn list_devices(_service: Option<&str>) -> Result<Vec<Device>, Error> {
    Ok(Vec::new())
}

/// `service` is accepted so the call matches Linux and macOS. The spooler does not use it.
pub fn list_service(_service: Option<&str>) -> Result<Vec<Queue>, Error> {
    if let Some(rows) = SPOOLER_FIXTURE.with(|slot| slot.borrow().clone()) {
        return Ok(rows
            .iter()
            .map(|row| {
                let mut queue = from_spooler(&row.name, &row.driver, &row.port, &row.status);
                queue.jobs = Some(row.jobs as i32);
                queue
            })
            .collect());
    }
    enum_printers()
}

fn enum_printers() -> Result<Vec<Queue>, Error> {
    let flags = 0x0000_0002 | 0x0000_0004;
    let mut needed = 0u32;
    let mut returned = 0u32;
    unsafe {
        EnumPrintersW(
            flags,
            std::ptr::null(),
            2,
            std::ptr::null_mut(),
            0,
            &mut needed,
            &mut returned,
        );
    }
    if needed == 0 {
        return Ok(Vec::new());
    }
    let words = (needed as usize).div_ceil(8);
    let mut buf = vec![0u64; words];
    let ok = unsafe {
        EnumPrintersW(
            flags,
            std::ptr::null(),
            2,
            buf.as_mut_ptr() as *mut u8,
            (buf.len() * 8) as u32,
            &mut needed,
            &mut returned,
        )
    };
    if ok == 0 {
        return Err(Error::new("EnumPrinters failed"));
    }
    let mut queues = Vec::with_capacity(returned as usize);
    for index in 0..returned as usize {
        let info = unsafe { &*(buf.as_ptr() as *const PrinterInfo2).add(index) };
        let mut queue = from_spooler(
            &wide_to_string(info.printer),
            &wide_to_string(info.driver),
            &wide_to_string(info.port),
            &status_words(info.status),
        );
        queue.jobs = Some(info.jobs as i32);
        queues.push(queue);
    }
    Ok(queues)
}

fn status_words(status: u32) -> String {
    let mut words = Vec::new();
    if status & 0x0000_0400 != 0 {
        words.push("printing");
    }
    if status & 0x0000_4000 != 0 {
        words.push("processing");
    }
    if status & 0x0000_0080 != 0 {
        words.push("offline");
    }
    if status & 0x0000_1000 != 0 {
        words.push("not available");
    }
    if status & 0x0000_0002 != 0 {
        words.push("error");
    }
    if status & 0x0000_0001 != 0 {
        words.push("paused");
    }
    if words.is_empty() {
        "idle".to_string()
    } else {
        words.join(" ")
    }
}

/// `cJobs` for one spooler queue. A missing queue has no count.
pub fn spooler_jobs(name: &str) -> Option<i32> {
    list()
        .ok()?
        .into_iter()
        .find(|queue| queue.name == name)
        .and_then(|queue| queue.jobs)
}

fn wide_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

#[repr(C)]
struct PrinterInfo2 {
    server: *const u16,
    printer: *const u16,
    share: *const u16,
    port: *const u16,
    driver: *const u16,
    comment: *const u16,
    location: *const u16,
    devmode: *mut u8,
    sep: *const u16,
    processor: *const u16,
    datatype: *const u16,
    parameters: *const u16,
    security: *mut u8,
    attributes: u32,
    priority: u32,
    default_priority: u32,
    start_time: u32,
    until_time: u32,
    status: u32,
    jobs: u32,
    average_ppm: u32,
}

#[link(name = "winspool")]
extern "system" {
    fn EnumPrintersW(
        flags: u32,
        name: *const u16,
        level: u32,
        buf: *mut u8,
        cb: u32,
        needed: *mut u32,
        returned: *mut u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    mod spooler {
        use super::super::{from_spooler, Kind};

        #[test]
        fn a_zebra_driver_is_zpl() {
            let queue = from_spooler("zebra", "ZTC GX430t", "", "printing");
            assert_eq!(queue.kind, Kind::Zpl);
            assert_eq!(queue.kind.as_str(), "zpl");
            assert_eq!(queue.dpi, 300);
            assert_eq!(queue.width_in, Some(4.1));
            assert_eq!(queue.state, "processing");
            assert!(queue.accepting);
            assert!(queue.installed);
            assert!(!queue.connected);
        }

        #[test]
        fn an_offline_queue_is_stopped() {
            let queue = from_spooler("Hall printer", "HP LaserJet", "", "offline");
            assert_eq!(queue.name, "Hall printer");
            assert_eq!(queue.state, "stopped");
            assert!(!queue.accepting);
        }

        #[test]
        fn an_hp_driver_is_raster() {
            let queue = from_spooler("hp", "HP LaserJet", "", "idle");
            assert_eq!(queue.kind, Kind::Raster);
            assert_eq!(queue.dpi, 300);
            assert!(queue.width_in.is_none());
        }
    }

    mod port {
        use super::super::from_spooler;

        #[test]
        fn an_ipp_port_sets_the_host() {
            let queue = from_spooler("ipp", "HP LaserJet", "ipp://192.0.2.10/ipp/print", "idle");
            assert_eq!(queue.device_uri, "ipp://192.0.2.10/ipp/print");
            assert_eq!(queue.host, "192.0.2.10");
        }

        #[test]
        fn https_ipp_becomes_ipps() {
            let queue = from_spooler("ipp", "HP LaserJet", "https://192.0.2.11/ipp/print", "idle");
            assert_eq!(queue.device_uri, "ipps://192.0.2.11/ipp/print");
            assert_eq!(queue.host, "192.0.2.11");
        }

        #[test]
        fn a_usb_port_has_no_host() {
            let queue = from_spooler("zebra", "Zebra Technologies", "USB001", "idle");
            assert_eq!(queue.device_uri, "usb://Zebra_Technologies/USB001");
            assert!(queue.host.is_empty());
        }

        #[test]
        fn a_tcp_port_is_a_socket() {
            let queue = from_spooler("raw", "HP LaserJet", "IP_192.0.2.12", "idle");
            assert_eq!(queue.device_uri, "socket://192.0.2.12:9100");
            assert!(queue.host.is_empty());
        }
    }

    mod list {
        use std::net::TcpListener;

        use super::super::{list, SpoolerRow, SPOOLER_FIXTURE};

        #[test]
        fn windows_list_is_the_spooler() {
            SPOOLER_FIXTURE.with(|slot| {
                *slot.borrow_mut() = Some(vec![
                    SpoolerRow {
                        name: "ipp".to_string(),
                        driver: "HP LaserJet".to_string(),
                        port: "ipp://192.0.2.10/ipp/print".to_string(),
                        status: "idle".to_string(),
                        jobs: 2,
                    },
                    SpoolerRow {
                        name: "zebra".to_string(),
                        driver: "Zebra Technologies".to_string(),
                        port: "USB001".to_string(),
                        status: "idle".to_string(),
                        jobs: 7,
                    },
                ]);
            });
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let queues = list().unwrap();
            assert_eq!(super::super::spooler_jobs("zebra"), Some(7));
            assert_eq!(super::super::spooler_jobs("ipp"), Some(2));
            assert_eq!(super::super::spooler_jobs("missing"), None);
            SPOOLER_FIXTURE.with(|slot| *slot.borrow_mut() = None);
            let err = listener.accept().unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
            assert_eq!(queues.len(), 2);
            let ipp = queues.iter().find(|queue| queue.name == "ipp").unwrap();
            assert_eq!(ipp.host, "192.0.2.10");
            let zebra = queues.iter().find(|queue| queue.name == "zebra").unwrap();
            assert!(zebra.host.is_empty());
            assert_eq!(zebra.device_uri, "usb://Zebra_Technologies/USB001");
            assert_eq!(zebra.jobs, Some(7));
        }
    }
}
