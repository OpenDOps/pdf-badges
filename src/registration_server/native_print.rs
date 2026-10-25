//! Operating-system printer queues.
//!
//! Linux and macOS compile the CUPS client (steps 1–4 and the CUPS `list`).
//! Windows compiles the spooler client (steps 5–7). The queue record is the
//! same shape on both, so a caller does not grow a second map.

const ZPL_WIDTH_IN: f64 = 4.1;
const EVOLIS_WIDTH_IN: f64 = 2.125;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zpl,
    Raster,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zpl => "zpl",
            Self::Raster => "raster",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Queue {
    pub name: String,
    pub device_uri: String,
    pub make_model: String,
    pub state: String,
    pub accepting: bool,
    pub host: String,
    pub kind: Kind,
    pub dpi: u16,
    pub width_in: Option<f64>,
    pub choices: Vec<Choice>,
    pub installed: bool,
    pub connected: bool,
    /// Windows spooler `cJobs`. Linux and macOS leave this empty and read
    /// `queued-job-count` from IPP when a route asks for the shorter queue.
    pub jobs: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Make `zebra technologies`, `tsc`, or model `ztc gx430t` is ZPL. That model and
/// make `evolis` are 300 dpi. Other ZPL queues are 203 dpi and 4.1 inches wide.
/// Evolis is 2.125 inches wide. Any other make is raster at 300 dpi with no width.
pub(crate) fn classify(make_model: &str) -> (Kind, u16, Option<f64>) {
    let lower = make_model.to_ascii_lowercase();
    let gx430t = lower.contains("ztc gx430t");
    let zpl = lower.contains("zebra technologies") || lower.contains("tsc") || gx430t;
    let evolis = lower.contains("evolis");
    let kind = if zpl { Kind::Zpl } else { Kind::Raster };
    let dpi = if gx430t || evolis || !zpl { 300 } else { 203 };
    let width_in = if zpl {
        Some(ZPL_WIDTH_IN)
    } else if evolis {
        Some(EVOLIS_WIDTH_IN)
    } else {
        None
    };
    (kind, dpi, width_in)
}

/// One queue from a listing, then `Finished`. `Failed` replaces `Finished` when the list cannot be read.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    Queue(Queue),
    Finished,
    Failed(String),
}

/// Send each queue as soon as that queue is ready, then `Finished`.
/// Dropping the receiver stops the listing.
pub fn watch(service: Option<String>) -> std::sync::mpsc::Receiver<Report> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || emit_queues(service.as_deref(), &tx));
    rx
}

/// One device from `CUPS-Get-Devices`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub uri: String,
    pub make_model: String,
}

/// One device from a listing, then `Finished`. `Failed` replaces `Finished` when the list cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceReport {
    Device(Device),
    Finished,
    Failed(String),
}

/// Send each device as soon as that device is read, then `Finished`.
/// Dropping the receiver stops the listing. Windows sends `Finished` and does not call CUPS.
pub fn watch_devices(service: Option<String>) -> std::sync::mpsc::Receiver<DeviceReport> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || emit_devices(service.as_deref(), &tx));
    rx
}

fn emit_devices(service: Option<&str>, tx: &std::sync::mpsc::Sender<DeviceReport>) {
    let devices = match list_devices(service) {
        Ok(devices) => devices,
        Err(err) => {
            let _ = tx.send(DeviceReport::Failed(err.to_string()));
            return;
        }
    };
    for device in devices {
        if tx.send(DeviceReport::Device(device)).is_err() {
            return;
        }
    }
    let _ = tx.send(DeviceReport::Finished);
}

fn emit_queues(service: Option<&str>, tx: &std::sync::mpsc::Sender<Report>) {
    let queues = match list_service(service) {
        Ok(queues) => queues,
        Err(err) => {
            let _ = tx.send(Report::Failed(err.to_string()));
            return;
        }
    };
    for mut queue in queues {
        if let Err(err) = fill_reported(&mut queue, service) {
            let _ = tx.send(Report::Failed(err.to_string()));
            return;
        }
        if tx.send(Report::Queue(queue)).is_err() {
            return;
        }
    }
    let _ = tx.send(Report::Finished);
}

fn fill_reported(queue: &mut Queue, service: Option<&str>) -> Result<(), Error> {
    let Some(base) = service else {
        return Ok(());
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        fill_choices(queue, base)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (queue, base);
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
include!("native_print_unix.rs");

#[cfg(windows)]
include!("native_print_windows.rs");

include!("native_print_ffi.rs");
