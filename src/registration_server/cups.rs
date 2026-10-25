//! Operating-system queues joined to discovered IPP printers.
//! Each tool sends a printer as soon as it is found. The current list is what viewers read.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use ipp_tool::{
    Browse, Catalog, Formats, Printer as IppPrinter, Report as IppReport, Settings, Toner,
};
use serde::{Deserialize, Serialize};

use super::native_print::{
    self, Choice, Device, DeviceReport, Kind, Queue, Report as NativeReport,
};

#[path = "print_job.rs"]
mod print_job;

pub use print_job::{PrintRequest, Printed};

#[derive(Debug)]
pub struct Error {
    message: String,
}

impl Error {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn is_no_queue(&self) -> bool {
        self.message == "no queue"
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// One printer. A native `ipp://` or `ipps://` queue and the discovered printer
/// at that address are the same record. The native queue name stays.
#[derive(Debug, Clone, PartialEq)]
pub struct Printer {
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
    /// `native` for an operating-system queue, `ipp` when there is no queue.
    pub class: String,
    pub values: Values,
    pub routing: Routing,
    /// Milliseconds of the last successful pick. Not stored in the routing file.
    pub last_print: u64,
    pub formats: Formats,
    pub settings: Settings,
    pub toner: Toner,
}

/// Saved options in `base/printer_conf/{name}.yml`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Values {
    pub options: BTreeMap<String, String>,
}

/// Routing for the open event in `base/{event_id}/pr_{name}.yml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    pub sector_name: String,
    pub used: bool,
    pub zone: String,
    pub category: String,
    /// `Some` when the queue is only for internet registrations, or only for ones that are not.
    /// `None` is not a filter unless every used queue leaves the flag empty.
    pub is_internet: Option<bool>,
    pub second_name: String,
    pub print_cert: bool,
    pub auto_print: bool,
    pub no_postscript: bool,
}

impl Default for Routing {
    fn default() -> Self {
        Self {
            sector_name: String::new(),
            used: true,
            zone: String::new(),
            category: String::new(),
            is_internet: None,
            second_name: String::new(),
            print_cert: false,
            auto_print: false,
            no_postscript: false,
        }
    }
}

/// The bytes one `Print-Job` sends. The variant is the document format.
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    /// A 1-bit graphic. A `zpl` queue wraps it as `^XA` … `^XZ`.
    Graphic { bytes: Vec<u8>, width_px: u32 },
    /// A PNG. `width_in` and `height_in` are the layout size sent as the media size.
    Png {
        bytes: Vec<u8>,
        width_in: f64,
        height_in: f64,
    },
    /// One PDF document. Postscript is not a page.
    Pdf { bytes: Vec<u8> },
}

/// What a print asks the current list to choose.
#[derive(Debug, Clone, Default)]
pub struct Job {
    /// An explicit queue name. It wins when that queue is connected, installed, and used.
    pub name: Option<String>,
    /// Visitor category. Empty means the job does not name one.
    pub category: Option<String>,
    pub internet: bool,
    /// The certificate attribute. The category printer is not used for this job.
    pub certificate: bool,
    pub zone: Option<String>,
    /// `round_robin` or `fewest_jobs`. A missing rule is `round_robin`.
    pub rule: Option<String>,
}

/// A change to the current list. Subscribers do not see the searching list.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Added(Printer),
    Updated(Printer),
    Removed(Printer),
}

struct Slot {
    /// Queues. A queue is added when its device enters `devices` and removed when that device leaves.
    current: Vec<Printer>,
    /// The current devices list.
    devices: Vec<Printer>,
    searching: Vec<Printer>,
    cache: Vec<IppPrinter>,
    generation: u64,
    native_done: bool,
    ipp_done: bool,
    device_done: bool,
    pending: HashSet<String>,
    fresh: HashSet<String>,
    /// Device URIs read on this pass.
    seen_devices: HashSet<String>,
    /// Queue names already considered for a one-time resume.
    adopted: HashSet<String>,
    /// USB devices waiting until the queue listing has been read.
    pending_install: Vec<Device>,
    running: bool,
    error: Option<String>,
}

#[derive(Clone)]
struct FileHome {
    base: PathBuf,
    event_id: String,
}

struct Inner {
    service: String,
    browse: Arc<dyn Browse + Send + Sync>,
    slot: Mutex<Slot>,
    cv: Condvar,
    details: Mutex<Option<Sender<String>>>,
    subscribers: Mutex<Vec<Sender<Change>>>,
    files: Mutex<Option<FileHome>>,
    desk: Mutex<print_job::Desk>,
}

/// The current list, filled from tool events. `refresh` starts another listing.
#[derive(Clone)]
pub struct Search {
    inner: Arc<Inner>,
}

impl Search {
    /// Start a listing and wait until both tools have finished.
    pub fn open(
        service: &str,
        browse: Arc<impl Browse + Send + Sync + 'static>,
    ) -> Result<Self, Error> {
        let search = Self::start(service, browse);
        search.wait()?;
        Ok(search)
    }

    /// Start a listing and return while the tools are still sending printers.
    pub fn start(service: &str, browse: Arc<impl Browse + Send + Sync + 'static>) -> Self {
        let browse: Arc<dyn Browse + Send + Sync> = browse;
        let search = Self {
            inner: Arc::new(Inner {
                service: service.to_string(),
                browse,
                slot: Mutex::new(Slot {
                    current: Vec::new(),
                    devices: Vec::new(),
                    searching: Vec::new(),
                    cache: Vec::new(),
                    generation: 0,
                    native_done: true,
                    ipp_done: true,
                    device_done: true,
                    pending: HashSet::new(),
                    fresh: HashSet::new(),
                    seen_devices: HashSet::new(),
                    adopted: HashSet::new(),
                    pending_install: Vec::new(),
                    running: false,
                    error: None,
                }),
                cv: Condvar::new(),
                details: Mutex::new(None),
                subscribers: Mutex::new(Vec::new()),
                files: Mutex::new(None),
                desk: Mutex::new(print_job::Desk::new()),
            }),
        };
        begin(&search.inner);
        search
    }

    /// A search that does not list printers. Tests insert the current list.
    #[cfg(test)]
    pub(crate) fn parked(service: &str) -> Self {
        Self {
            inner: Arc::new(Inner {
                service: service.to_string(),
                browse: Arc::new(SilentBrowse),
                slot: Mutex::new(Slot {
                    current: Vec::new(),
                    devices: Vec::new(),
                    searching: Vec::new(),
                    cache: Vec::new(),
                    generation: 0,
                    native_done: true,
                    ipp_done: true,
                    device_done: true,
                    pending: HashSet::new(),
                    fresh: HashSet::new(),
                    seen_devices: HashSet::new(),
                    adopted: HashSet::new(),
                    pending_install: Vec::new(),
                    running: false,
                    error: None,
                }),
                cv: Condvar::new(),
                details: Mutex::new(None),
                subscribers: Mutex::new(Vec::new()),
                files: Mutex::new(None),
                desk: Mutex::new(print_job::Desk::new()),
            }),
        }
    }

    /// Put one queue on the current list. The listing thread is not running.
    #[cfg(test)]
    pub(crate) fn testing_insert(&self, printer: Printer) {
        self.inner
            .slot
            .lock()
            .expect("printers")
            .current
            .push(printer);
    }

    /// Changes to the current list. A dropped receiver is removed on the next change.
    pub fn subscribe(&self) -> mpsc::Receiver<Change> {
        let (tx, rx) = mpsc::channel();
        self.inner.subscribers.lock().expect("subscribers").push(tx);
        rx
    }

    /// Insert one `ipp://` or `ipps://` URI. No browse runs.
    pub fn add(&self, uri: &str) -> Result<(), Error> {
        let mut catalog = Catalog::default();
        catalog
            .add(uri)
            .map_err(|err| Error::new(err.to_string()))?;
        let found = catalog
            .printers()
            .first()
            .cloned()
            .ok_or_else(|| Error::new(format!("{uri} was not added")))?;
        let home = files_of(&self.inner);
        let mut slot = self.inner.slot.lock().expect("printers");
        remember(&mut slot.cache, &found);
        let mut printer = merge_details(&mut slot.searching, &found);
        attach_files(home.as_ref(), &mut printer);
        let key = device_key(&printer.host, &printer.device_uri);
        upsert(&mut slot.searching, &key, printer.clone());
        let changes = publish_current(&mut slot, &key, &printer);
        drop(slot);
        emit(&self.inner, changes);
        Ok(())
    }

    /// Settings and routing are read from `base` for `event_id` when a device is accepted.
    pub fn set_files(&self, base: &Path, event_id: &str) {
        *self.inner.files.lock().expect("files") = Some(FileHome {
            base: base.to_path_buf(),
            event_id: event_id.to_string(),
        });
    }

    /// Write the routing file, including an empty second name, and keep it on the current record.
    pub fn save_routing(&self, name: &str, routing: Routing) -> Result<(), Error> {
        let home = files_of(&self.inner).ok_or_else(|| Error::new("printer files are not set"))?;
        let mut slot = self.inner.slot.lock().expect("printers");
        let class = slot
            .current
            .iter()
            .find(|printer| printer.name == name)
            .map(|printer| printer.class.clone())
            .ok_or_else(|| Error::new(format!("{name} is not on the current list")))?;
        if let Some(printer) = slot.current.iter_mut().find(|printer| printer.name == name) {
            printer.routing = routing.clone();
        }
        if let Some(printer) = slot.devices.iter_mut().find(|printer| printer.name == name) {
            printer.routing = routing.clone();
        }
        if let Some(printer) = slot
            .searching
            .iter_mut()
            .find(|printer| printer.name == name)
        {
            printer.routing = routing.clone();
        }
        drop(slot);
        write_routing(&home, name, &class, &routing).map_err(|err| Error::new(err.to_string()))
    }

    /// One connected, installed, used queue from the current list, or an error that names no queue.
    /// The choice does not record a print. `last_print` moves when `submit` completes.
    pub fn pick(&self, job: &Job) -> Result<Printer, Error> {
        let pool = {
            let slot = self.inner.slot.lock().expect("printers");
            eligible(&slot.current, job)
        };
        select(&self.inner.service, pool, job).ok_or_else(|| Error::new("no queue"))
    }

    /// One `Print-Job` for the bytes in `page`. The page selects the document format:
    /// raw ZPL, `image/png`, or `application/pdf`. An operating-system queue uses the
    /// native library. A printer with no operating-system queue uses the IPP tool.
    /// A failed job is cancelled and sent again, up to ten times.
    pub fn submit(&self, printer: &Printer, page: &Page) -> Result<i32, Error> {
        let (format, document, options) = prepare_page(printer, page);
        if printer.state == "stopped" || !printer.accepting {
            resume_printer(&self.inner.service, printer)?;
        }
        let mut last = Error::new("job aborted");
        for _ in 0..10 {
            // A send that fails, including a timeout, returns. It does not start another copy.
            let id = send_job(&self.inner.service, printer, &format, &options, &document)?;
            match wait_job(&self.inner.service, printer, id)? {
                Outcome::Completed => {
                    self.note_printed(&printer.name);
                    return Ok(id);
                }
                Outcome::Running => return Err(Error::new("job still processing")),
                outcome => match settled_job(&self.inner.service, printer, id)? {
                    Some(id) => {
                        self.note_printed(&printer.name);
                        return Ok(id);
                    }
                    None => {
                        cancel_job(&self.inner.service, printer, id)?;
                        if outcome == Outcome::Stopped {
                            resume_printer(&self.inner.service, printer)?;
                            last = Error::new("queue stopped");
                        } else {
                            last = Error::new("job aborted");
                        }
                    }
                },
            }
        }
        Err(last)
    }

    #[cfg(test)]
    fn set_last_print(&self, name: &str, at: u64) {
        let mut slot = self.inner.slot.lock().expect("printers");
        remember_print(&mut slot, name, at);
    }

    fn note_printed(&self, name: &str) {
        let at = unix_millis();
        let mut slot = self.inner.slot.lock().expect("printers");
        remember_print(&mut slot, name, at);
    }

    /// List both tools again. Returns while that listing is still running.
    pub fn refresh(&self) {
        begin(&self.inner);
    }

    /// Wait until the listing that is running has settled.
    pub fn wait(&self) -> Result<(), Error> {
        let mut slot = self.inner.slot.lock().expect("printers");
        while slot.running {
            slot = self.inner.cv.wait(slot).expect("printers");
        }
        match &slot.error {
            Some(message) => Err(Error::new(message.clone())),
            None => Ok(()),
        }
    }

    pub fn printers(&self) -> Vec<Printer> {
        self.inner.slot.lock().expect("printers").current.clone()
    }

    /// The devices whose queues are in `printers`.
    pub fn devices(&self) -> Vec<Printer> {
        self.inner.slot.lock().expect("printers").devices.clone()
    }
}

#[cfg(test)]
struct SilentBrowse;

#[cfg(test)]
impl Browse for SilentBrowse {
    fn browse(&self) -> Vec<ipp_tool::Service> {
        Vec::new()
    }
}

fn begin(inner: &Arc<Inner>) {
    let generation = {
        let mut slot = inner.slot.lock().expect("printers");
        slot.generation += 1;
        slot.native_done = false;
        slot.ipp_done = false;
        slot.device_done = false;
        slot.pending.clear();
        slot.fresh.clear();
        slot.seen_devices.clear();
        slot.pending_install.clear();
        slot.running = true;
        slot.error = None;
        slot.searching.clear();
        slot.generation
    };
    *inner.details.lock().expect("details") = None;
    let inner = Arc::clone(inner);
    thread::spawn(move || pass(inner, generation));
}

enum Side {
    Native(NativeReport),
    Ipp(IppReport),
    Device(DeviceReport),
}

fn pass(inner: Arc<Inner>, generation: u64) {
    let known = inner.slot.lock().expect("printers").cache.clone();
    let native_rx = native_print::watch(Some(inner.service.clone()));
    let device_rx = native_print::watch_devices(Some(inner.service.clone()));
    let mut ipp = Catalog::watch(Arc::clone(&inner.browse), known);
    *inner.details.lock().expect("details") = Some(ipp.take_details());
    let (tx, rx) = mpsc::channel();
    let native_tx = tx.clone();
    let device_tx = tx.clone();
    thread::spawn(move || {
        while let Ok(report) = native_rx.recv() {
            if native_tx.send(Side::Native(report)).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        while let Ok(report) = device_rx.recv() {
            if device_tx.send(Side::Device(report)).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        while let Ok(report) = ipp.recv() {
            if tx.send(Side::Ipp(report)).is_err() {
                break;
            }
        }
    });
    while let Ok(side) = rx.recv() {
        if inner.slot.lock().expect("printers").generation != generation {
            break;
        }
        let changes = match side {
            Side::Native(report) => on_native(&inner, generation, report),
            Side::Ipp(report) => on_ipp(&inner, generation, report),
            Side::Device(report) => on_device(&inner, generation, report),
        };
        emit(&inner, changes);
        let installed = install_ready(&inner, generation);
        emit(&inner, installed);
        let slot = inner.slot.lock().expect("printers");
        let settled = slot.generation == generation && !slot.running;
        drop(slot);
        if settled {
            inner.cv.notify_all();
        }
    }
    let mut slot = inner.slot.lock().expect("printers");
    if slot.generation == generation && slot.running {
        slot.error = Some("listing stopped".to_string());
        slot.running = false;
        drop(slot);
        inner.cv.notify_all();
    }
}

fn on_native(inner: &Inner, generation: u64, report: NativeReport) -> Vec<Change> {
    let mut slot = inner.slot.lock().expect("printers");
    if slot.generation != generation {
        return Vec::new();
    }
    match report {
        NativeReport::Queue(queue) => {
            let ask = take_native(&mut slot, &queue);
            let (mut changes, resume) = publish_native(inner, &mut slot, &queue);
            changes.extend(try_settle(&mut slot));
            drop(slot);
            if resume {
                let _ = resume_queue(&inner.service, &queue.name);
            }
            if let Some(uri) = ask {
                let sent = inner
                    .details
                    .lock()
                    .expect("details")
                    .as_ref()
                    .map(|tx| tx.send(uri.clone()).is_ok())
                    .unwrap_or(false);
                if !sent {
                    let mut slot = inner.slot.lock().expect("printers");
                    if slot.generation == generation {
                        if let Some(host) = host_of(&uri) {
                            slot.pending.remove(&host);
                        }
                        changes.extend(try_settle(&mut slot));
                    }
                }
            }
            changes
        }
        NativeReport::Finished => {
            slot.native_done = true;
            try_settle(&mut slot)
        }
        NativeReport::Failed(message) => {
            slot.error = Some(message);
            slot.native_done = true;
            slot.pending.clear();
            slot.pending_install.clear();
            try_settle(&mut slot)
        }
    }
}

fn on_ipp(inner: &Inner, generation: u64, report: IppReport) -> Vec<Change> {
    let mut slot = inner.slot.lock().expect("printers");
    if slot.generation != generation {
        return Vec::new();
    }
    match report {
        IppReport::Printer(found) => on_found(inner, &mut slot, &found),
        IppReport::Missed(uri) => {
            if let Some(host) = host_of(&uri) {
                slot.pending.remove(&host);
            }
            try_settle(&mut slot)
        }
        IppReport::Finished => {
            slot.ipp_done = true;
            try_settle(&mut slot)
        }
        IppReport::Failed(message) => {
            slot.error = Some(message);
            slot.ipp_done = true;
            slot.pending.clear();
            try_settle(&mut slot)
        }
    }
}

/// Record the queue. Returns a URI when the IPP cache has no details for it.
fn take_native(slot: &mut Slot, queue: &Queue) -> Option<String> {
    if !is_ipp(&queue.device_uri) {
        return None;
    }
    if slot.cache.iter().any(|item| item.address == queue.host) {
        return None;
    }
    if slot
        .searching
        .iter()
        .any(|item| item.host == queue.host && item.connected)
    {
        return None;
    }
    if !slot.pending.insert(queue.host.clone()) {
        return None;
    }
    Some(queue.device_uri.clone())
}

fn publish_native(inner: &Inner, slot: &mut Slot, queue: &Queue) -> (Vec<Change>, bool) {
    let mut printer = from_queue(queue);
    if is_ipp(&queue.device_uri) {
        if let Some(found) = slot.cache.iter().find(|item| item.address == queue.host) {
            apply_details(&mut printer, found);
        } else if let Some(existing) = slot
            .searching
            .iter()
            .find(|item| item.host == queue.host && item.connected)
        {
            printer.formats = existing.formats.clone();
            printer.settings = existing.settings.clone();
            printer.toner = existing.toner.clone();
            printer.connected = true;
        }
    } else if slot.seen_devices.contains(&queue.device_uri) {
        printer.connected = true;
    }
    if let Some(existing) = slot
        .current
        .iter()
        .find(|item| item.device_uri == queue.device_uri)
    {
        printer.last_print = existing.last_print;
        if existing.connected && !is_ipp(&queue.device_uri) {
            printer.connected = true;
        }
    }
    attach_files(files_of(inner).as_ref(), &mut printer);
    let resume = arm_resume(slot, &printer);
    let key = device_key(&printer.host, &printer.device_uri);
    upsert(&mut slot.searching, &key, printer.clone());
    (publish_current(slot, &key, &printer), resume)
}

fn on_found(inner: &Inner, slot: &mut Slot, found: &IppPrinter) -> Vec<Change> {
    remember(&mut slot.cache, found);
    slot.pending.remove(&found.address);
    let mut printer = merge_details(&mut slot.searching, found);
    attach_files(files_of(inner).as_ref(), &mut printer);
    let key = device_key(&printer.host, &printer.device_uri);
    upsert(&mut slot.searching, &key, printer.clone());
    let changes = publish_current(slot, &key, &printer);
    let settled = try_settle(slot);
    let mut changes = changes;
    changes.extend(settled);
    changes
}

fn merge_details(searching: &mut Vec<Printer>, found: &IppPrinter) -> Printer {
    let key = device_key(&found.address, &found.uri);
    if let Some(existing) = searching
        .iter()
        .find(|item| item.host == found.address || device_key(&item.host, &item.device_uri) == key)
        .cloned()
    {
        let mut printer = existing;
        apply_details(&mut printer, found);
        upsert(
            searching,
            &device_key(&printer.host, &printer.device_uri),
            printer.clone(),
        );
        printer
    } else {
        let printer = from_discovered(found);
        upsert(searching, &key, printer.clone());
        printer
    }
}

fn try_settle(slot: &mut Slot) -> Vec<Change> {
    if !slot.running
        || !slot.native_done
        || !slot.ipp_done
        || !slot.device_done
        || !slot.pending.is_empty()
    {
        return Vec::new();
    }
    if slot.error.is_some() {
        slot.running = false;
        return Vec::new();
    }
    let found: Vec<(String, Printer)> = slot
        .searching
        .iter()
        .map(|printer| {
            (
                device_key(&printer.host, &printer.device_uri),
                printer.clone(),
            )
        })
        .collect();
    let found_keys: HashSet<String> = found.iter().map(|(key, _)| key.clone()).collect();
    let mut changes = Vec::new();
    let mut devices = Vec::new();
    for device in std::mem::take(&mut slot.devices) {
        let key = device_key(&device.host, &device.device_uri);
        if found_keys.contains(&key) {
            devices.push(device);
            continue;
        }
        let queue = remove_printer(&mut slot.current, &key).unwrap_or_else(|| device.clone());
        changes.push(Change::Removed(queue));
    }
    slot.devices = devices;
    for (key, printer) in found {
        if slot
            .devices
            .iter()
            .any(|item| device_key(&item.host, &item.device_uri) == key)
        {
            store_printer(&mut slot.devices, &key, &printer);
            if let Some(index) = printer_index(&slot.current, &key) {
                let last_print = slot.current[index].last_print;
                let mut printer = printer;
                printer.last_print = last_print;
                if slot.current[index] != printer {
                    slot.current[index] = printer.clone();
                    changes.push(Change::Updated(printer));
                }
            } else {
                changes.extend(add_queue(slot, &key, &printer));
            }
            continue;
        }
        changes.extend(add_device(slot, &key, &printer));
    }
    slot.fresh.clear();
    slot.running = false;
    debug_assert!(same_membership(slot));
    changes
}

/// A device enters the current devices list, and its queue is added with it.
fn add_device(slot: &mut Slot, key: &str, printer: &Printer) -> Vec<Change> {
    store_printer(&mut slot.devices, key, printer);
    let mut changes = add_queue(slot, key, printer);
    slot.fresh.insert(key.to_string());
    if changes.is_empty() {
        changes.push(Change::Added(printer.clone()));
    }
    changes
}

fn add_queue(slot: &mut Slot, key: &str, printer: &Printer) -> Vec<Change> {
    if printer_index(&slot.current, key).is_some() {
        store_printer(&mut slot.current, key, printer);
        return Vec::new();
    }
    slot.current.push(printer.clone());
    vec![Change::Added(printer.clone())]
}

fn publish_current(slot: &mut Slot, key: &str, printer: &Printer) -> Vec<Change> {
    if printer_index(&slot.devices, key).is_some() {
        if slot.fresh.contains(key)
            && slot
                .devices
                .iter()
                .any(|item| device_key(&item.host, &item.device_uri) == key && item != printer)
        {
            store_printer(&mut slot.devices, key, printer);
            store_printer(&mut slot.current, key, printer);
            debug_assert!(same_membership(slot));
            return vec![Change::Updated(printer.clone())];
        }
        if printer_index(&slot.current, key).is_none() {
            let changes = add_queue(slot, key, printer);
            debug_assert!(same_membership(slot));
            return changes;
        }
        return Vec::new();
    }
    let changes = add_device(slot, key, printer);
    debug_assert!(same_membership(slot));
    changes
}

fn store_printer(list: &mut Vec<Printer>, key: &str, printer: &Printer) {
    if let Some(index) = printer_index(list, key) {
        list[index] = printer.clone();
        return;
    }
    list.push(printer.clone());
}

fn remove_printer(list: &mut Vec<Printer>, key: &str) -> Option<Printer> {
    let index = printer_index(list, key)?;
    Some(list.remove(index))
}

fn printer_index(list: &[Printer], key: &str) -> Option<usize> {
    list.iter()
        .position(|item| device_key(&item.host, &item.device_uri) == key)
}

fn same_membership(slot: &Slot) -> bool {
    let mut devices: Vec<_> = slot
        .devices
        .iter()
        .map(|printer| device_key(&printer.host, &printer.device_uri))
        .collect();
    let mut queues: Vec<_> = slot
        .current
        .iter()
        .map(|printer| device_key(&printer.host, &printer.device_uri))
        .collect();
    devices.sort();
    queues.sort();
    devices == queues
}

fn upsert(searching: &mut Vec<Printer>, key: &str, printer: Printer) {
    if let Some(existing) = searching
        .iter_mut()
        .find(|item| device_key(&item.host, &item.device_uri) == key)
    {
        *existing = printer;
        return;
    }
    searching.push(printer);
}

fn remember(cache: &mut Vec<IppPrinter>, found: &IppPrinter) {
    if let Some(existing) = cache.iter_mut().find(|item| item.address == found.address) {
        if found.secure || !existing.secure {
            *existing = found.clone();
        }
        return;
    }
    cache.push(found.clone());
}

fn apply_details(printer: &mut Printer, found: &IppPrinter) {
    printer.formats = found.formats.clone();
    printer.settings = found.settings.clone();
    printer.toner = found.toner.clone();
    printer.connected = true;
}

fn emit(inner: &Inner, changes: Vec<Change>) {
    if changes.is_empty() {
        return;
    }
    let mut subscribers = inner.subscribers.lock().expect("subscribers");
    subscribers.retain(|tx| {
        for change in &changes {
            if tx.send(change.clone()).is_err() {
                return false;
            }
        }
        true
    });
}

fn device_key(host: &str, device_uri: &str) -> String {
    if host.is_empty() {
        format!("dev:{device_uri}")
    } else {
        format!("ipp:{host}")
    }
}

fn host_of(uri: &str) -> Option<String> {
    IppPrinter::from_uri(uri)
        .ok()
        .map(|printer| printer.address)
}

fn from_queue(queue: &Queue) -> Printer {
    Printer {
        name: queue.name.clone(),
        device_uri: queue.device_uri.clone(),
        make_model: queue.make_model.clone(),
        state: queue.state.clone(),
        accepting: queue.accepting,
        host: queue.host.clone(),
        kind: queue.kind,
        dpi: queue.dpi,
        width_in: queue.width_in,
        choices: queue.choices.clone(),
        installed: queue.installed,
        connected: false,
        class: "native".to_string(),
        values: Values::default(),
        routing: Routing::default(),
        last_print: 0,
        formats: Formats::default(),
        settings: Settings::default(),
        toner: Toner::default(),
    }
}

fn from_discovered(found: &IppPrinter) -> Printer {
    let (kind, dpi, width_in) = native_print::classify(&found.make_model);
    Printer {
        name: if found.name.is_empty() {
            found.address.clone()
        } else {
            found.name.clone()
        },
        device_uri: found.uri.clone(),
        make_model: found.make_model.clone(),
        state: found.settings.state.clone(),
        accepting: true,
        host: found.address.clone(),
        kind,
        dpi,
        width_in,
        choices: Vec::new(),
        installed: true,
        connected: true,
        class: "ipp".to_string(),
        values: Values::default(),
        routing: Routing::default(),
        last_print: 0,
        formats: found.formats.clone(),
        settings: found.settings.clone(),
        toner: found.toner.clone(),
    }
}

fn is_ipp(uri: &str) -> bool {
    uri.starts_with("ipp://") || uri.starts_with("ipps://")
}

fn on_device(inner: &Inner, generation: u64, report: DeviceReport) -> Vec<Change> {
    let mut slot = inner.slot.lock().expect("printers");
    if slot.generation != generation {
        return Vec::new();
    }
    match report {
        DeviceReport::Device(device) => {
            slot.seen_devices.insert(device.uri.clone());
            if is_ipp(&device.uri) {
                return Vec::new();
            }
            if let Some(index) = slot
                .current
                .iter()
                .position(|printer| printer.device_uri == device.uri)
            {
                return mark_connected(&mut slot, index);
            }
            let mut printer = from_device(&device);
            attach_files(files_of(inner).as_ref(), &mut printer);
            let key = device_key(&printer.host, &printer.device_uri);
            upsert(&mut slot.searching, &key, printer.clone());
            let changes = publish_current(&mut slot, &key, &printer);
            slot.pending_install.push(device);
            changes
        }
        DeviceReport::Finished => {
            let mut changes = disconnect_missing(&mut slot);
            slot.device_done = true;
            changes.extend(try_settle(&mut slot));
            changes
        }
        DeviceReport::Failed(_) => {
            slot.device_done = true;
            slot.pending_install.clear();
            try_settle(&mut slot)
        }
    }
}

fn mark_connected(slot: &mut Slot, index: usize) -> Vec<Change> {
    let uri = slot.current[index].device_uri.clone();
    let key = device_key(&slot.current[index].host, &uri);
    if let Some(printer) = slot
        .searching
        .iter_mut()
        .find(|printer| printer.device_uri == uri)
    {
        printer.connected = true;
    } else if let Some(printer) = slot
        .current
        .iter()
        .find(|printer| printer.device_uri == uri)
    {
        let mut printer = printer.clone();
        printer.connected = true;
        upsert(&mut slot.searching, &key, printer);
    }
    if let Some(printer) = slot
        .devices
        .iter_mut()
        .find(|printer| printer.device_uri == uri)
    {
        printer.connected = true;
    }
    if slot.current[index].connected {
        return Vec::new();
    }
    slot.current[index].connected = true;
    vec![Change::Updated(slot.current[index].clone())]
}

fn disconnect_missing(slot: &mut Slot) -> Vec<Change> {
    let mut changes = Vec::new();
    for printer in &mut slot.current {
        if is_ipp(&printer.device_uri) || slot.seen_devices.contains(&printer.device_uri) {
            continue;
        }
        if !printer.connected {
            continue;
        }
        printer.connected = false;
        let uri = printer.device_uri.clone();
        changes.push(Change::Updated(printer.clone()));
        if let Some(device) = slot.devices.iter_mut().find(|item| item.device_uri == uri) {
            device.connected = false;
        }
        if let Some(found) = slot
            .searching
            .iter_mut()
            .find(|item| item.device_uri == uri)
        {
            found.connected = false;
        }
    }
    changes
}

fn from_device(device: &Device) -> Printer {
    let make_model = if device.make_model.is_empty() {
        make_model_from_uri(&device.uri)
    } else {
        device.make_model.clone()
    };
    let (kind, dpi, width_in) = native_print::classify(&make_model);
    Printer {
        name: queue_name(&device.uri, &make_model),
        device_uri: device.uri.clone(),
        make_model,
        state: "idle".to_string(),
        accepting: true,
        host: String::new(),
        kind,
        dpi,
        width_in,
        choices: Vec::new(),
        installed: false,
        connected: true,
        class: "native".to_string(),
        values: Values::default(),
        routing: Routing::default(),
        last_print: 0,
        formats: Formats::default(),
        settings: Settings::default(),
        toner: Toner::default(),
    }
}

fn queue_name(uri: &str, make_model: &str) -> String {
    let Some(rest) = uri.strip_prefix("usb://") else {
        return make_model.replace(' ', "_");
    };
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (make, model) = path.split_once('/').unwrap_or((path, ""));
    let serial = query
        .split('&')
        .find_map(|part| {
            let (key, value) = part.split_once('=')?;
            (key == "serial").then_some(value)
        })
        .unwrap_or("");
    let mut parts = Vec::new();
    for part in [make, model, serial] {
        if part.is_empty() {
            continue;
        }
        parts.push(part.replace(' ', "_"));
    }
    if parts.is_empty() {
        make_model.replace(' ', "_")
    } else {
        parts.join("_")
    }
}

fn make_model_from_uri(uri: &str) -> String {
    let Some(rest) = uri.strip_prefix("usb://") else {
        return String::new();
    };
    let path = rest.split('?').next().unwrap_or(rest);
    let (make, model) = path.split_once('/').unwrap_or((path, ""));
    format!("{make} {model}").trim().to_string()
}

fn arm_resume(slot: &mut Slot, printer: &Printer) -> bool {
    if printer.class != "native" || printer.name.is_empty() {
        return false;
    }
    if !slot.adopted.insert(printer.name.clone()) {
        return false;
    }
    printer.state == "stopped" || !printer.accepting
}

fn install_pending(inner: &Inner, generation: u64, devices: Vec<Device>) -> Vec<Change> {
    let mut changes = Vec::new();
    for device in devices {
        if is_ipp(&device.uri) {
            continue;
        }
        let ready = {
            let slot = inner.slot.lock().expect("printers");
            if slot.generation != generation {
                return changes;
            }
            slot.current
                .iter()
                .find(|printer| printer.device_uri == device.uri && !printer.installed)
                .map(|printer| (printer.name.clone(), printer.make_model.clone()))
        };
        let Some((name, make_model)) = ready else {
            continue;
        };
        let ppd = match ppd_for(&inner.service, &make_model) {
            Some(ppd) => ppd,
            None => continue,
        };
        if add_printer(&inner.service, &name, &device.uri, &ppd).is_err() {
            continue;
        }
        let mut slot = inner.slot.lock().expect("printers");
        if slot.generation != generation {
            return changes;
        }
        let key = device_key("", &device.uri);
        if let Some(printer) = slot
            .current
            .iter_mut()
            .find(|printer| printer.device_uri == device.uri)
        {
            printer.installed = true;
        }
        if let Some(printer) = slot
            .devices
            .iter_mut()
            .find(|printer| printer.device_uri == device.uri)
        {
            printer.installed = true;
        }
        if let Some(printer) = slot
            .searching
            .iter_mut()
            .find(|printer| printer.device_uri == device.uri)
        {
            printer.installed = true;
        }
        if let Some(printer) = slot
            .current
            .iter()
            .find(|printer| printer.device_uri == device.uri)
        {
            let printer = printer.clone();
            upsert(&mut slot.searching, &key, printer.clone());
            changes.push(Change::Updated(printer));
        }
    }
    changes
}

const CUPS_ADD_MODIFY_PRINTER: u16 = 0x4003;
const CUPS_ACCEPT_JOBS: u16 = 0x4008;
const CUPS_GET_PPDS: u16 = 0x400c;
const IPP_RESUME_PRINTER: u16 = 0x0011;

fn ppd_for(base: &str, make_model: &str) -> Option<String> {
    let listed = list_ppds(base).ok()?;
    pick_ppd(make_model, &listed)
}

fn pick_ppd(make_model: &str, ppds: &[(String, String)]) -> Option<String> {
    let make = make_model.to_ascii_lowercase();
    if make.contains("zebra") {
        if let Some((name, _)) = ppds.iter().find(|(name, _)| name == "zebra.ppd") {
            return Some(name.clone());
        }
    }
    if make.contains("evolis") {
        if let Some((name, _)) = ppds.iter().find(|(name, _)| name == "evolis-zeniusE.ppd") {
            return Some(name.clone());
        }
    }
    ppds.iter()
        .find(|(_, model)| {
            let model = model.to_ascii_lowercase();
            !model.is_empty() && (make.contains(model.as_str()) || model.contains(make.as_str()))
        })
        .map(|(name, _)| name.clone())
}

fn install_ready(inner: &Inner, generation: u64) -> Vec<Change> {
    let pending = {
        let mut slot = inner.slot.lock().expect("printers");
        if slot.generation != generation || !slot.native_done || slot.pending_install.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut slot.pending_install)
    };
    let changes = install_pending(inner, generation, pending);
    let mut slot = inner.slot.lock().expect("printers");
    if slot.generation == generation {
        let settled = try_settle(&mut slot);
        drop(slot);
        let mut changes = changes;
        changes.extend(settled);
        return changes;
    }
    changes
}

fn list_ppds(base: &str) -> Result<Vec<(String, String)>, Error> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let response = native_print::cups_call(base, CUPS_GET_PPDS, &[])
            .map_err(|err| Error::new(err.to_string()))?;
        let message = ipp_tool::decode(&response).map_err(|err| Error::new(err.to_string()))?;
        if message.code >= 0x0400 {
            return Err(Error::new(format!(
                "CUPS-Get-PPDs status {:#06x}",
                message.code
            )));
        }
        let mut listed = Vec::new();
        for group in &message.groups {
            let Some(name) = group_str(group, "ppd-name") else {
                continue;
            };
            let make = group_str(group, "ppd-make-and-model").unwrap_or_default();
            listed.push((name, make));
        }
        return Ok(listed);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = base;
        Ok(Vec::new())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn group_str(group: &ipp_tool::Group, name: &str) -> Option<String> {
    group
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.values.first())
        .and_then(|value| value.as_str().map(str::to_string))
}

fn add_printer(base: &str, name: &str, device_uri: &str, ppd: &str) -> Result<(), Error> {
    let printer_uri = format!("ipp://localhost/printers/{name}");
    cups_operation(
        base,
        CUPS_ADD_MODIFY_PRINTER,
        &[
            ("printer-uri", 0x45, printer_uri.as_str()),
            ("device-uri", 0x45, device_uri),
            ("ppd-name", 0x42, ppd),
        ],
    )
}

fn resume_queue(base: &str, name: &str) -> Result<(), Error> {
    let printer_uri = format!("ipp://localhost/printers/{name}");
    cups_operation(
        base,
        IPP_RESUME_PRINTER,
        &[("printer-uri", 0x45, printer_uri.as_str())],
    )?;
    cups_operation(
        base,
        CUPS_ACCEPT_JOBS,
        &[("printer-uri", 0x45, printer_uri.as_str())],
    )
}

fn cups_operation(
    base: &str,
    operation: u16,
    attributes: &[(&str, u8, &str)],
) -> Result<(), Error> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        native_print::cups_call(base, operation, attributes)
            .map(|_| ())
            .map_err(|err| Error::new(err.to_string()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (base, operation, attributes);
        Ok(())
    }
}

fn files_of(inner: &Inner) -> Option<FileHome> {
    inner.files.lock().expect("files").clone()
}

fn attach_files(home: Option<&FileHome>, printer: &mut Printer) {
    let Some(home) = home else {
        return;
    };
    if let Ok(values) = settings_for(home, printer) {
        printer.values = values;
    }
    if let Ok(routing) = routing_for(home, printer) {
        printer.routing = routing;
    }
}

fn settings_for(home: &FileHome, printer: &Printer) -> Result<Values, Error> {
    let path = home
        .base
        .join("printer_conf")
        .join(format!("{}.yml", printer.name));
    if let Some(options) = read_options(&path) {
        return Ok(Values { options });
    }
    let options = default_options(printer);
    write_options(&path, &printer.class, &options)?;
    Ok(Values { options })
}

fn routing_for(home: &FileHome, printer: &Printer) -> Result<Routing, Error> {
    let path = home
        .base
        .join(&home.event_id)
        .join(format!("pr_{}.yml", printer.name));
    if let Some(routing) = read_routing(&path) {
        return Ok(routing);
    }
    let routing = Routing::default();
    write_routing(home, &printer.name, &printer.class, &routing)?;
    Ok(routing)
}

fn default_options(printer: &Printer) -> BTreeMap<String, String> {
    if printer.kind == Kind::Zpl {
        return zpl_defaults();
    }
    if printer.make_model.to_ascii_lowercase().contains("evolis") {
        let mut options = BTreeMap::new();
        options.insert("InkType".to_string(), "Black".to_string());
        return options;
    }
    BTreeMap::new()
}

fn zpl_defaults() -> BTreeMap<String, String> {
    let mut options = BTreeMap::new();
    options.insert("zeMediaTracking".to_string(), "Web".to_string());
    options.insert("MediaType".to_string(), "Direct".to_string());
    options.insert("zePrintMode".to_string(), "Cutter".to_string());
    options.insert("Speed".to_string(), "10".to_string());
    options.insert("Darkness".to_string(), "0".to_string());
    options.insert("zpl_label_shift".to_string(), "0".to_string());
    options.insert("zpl_label_top".to_string(), "0".to_string());
    options.insert("zpl_tear_off".to_string(), "0".to_string());
    options
}

fn read_options(path: &Path) -> Option<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let map = value.as_mapping()?.clone();
    let mut options = BTreeMap::new();
    for (key, value) in map {
        let Some(key) = key.as_str() else {
            continue;
        };
        if key == "class" {
            continue;
        }
        let text = match value {
            serde_yaml::Value::String(text) => text,
            serde_yaml::Value::Number(number) => number.to_string(),
            serde_yaml::Value::Bool(flag) => flag.to_string(),
            _ => continue,
        };
        options.insert(key.to_string(), text);
    }
    Some(options)
}

fn write_options(
    path: &Path,
    class: &str,
    options: &BTreeMap<String, String>,
) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| Error::new(err.to_string()))?;
    }
    let mut map = serde_yaml::Mapping::new();
    map.insert(
        serde_yaml::Value::String("class".to_string()),
        serde_yaml::Value::String(class.to_string()),
    );
    for (key, value) in options {
        let yaml = if let Ok(number) = value.parse::<i64>() {
            serde_yaml::Value::Number(number.into())
        } else {
            serde_yaml::Value::String(value.clone())
        };
        map.insert(serde_yaml::Value::String(key.clone()), yaml);
    }
    let text = serde_yaml::to_string(&serde_yaml::Value::Mapping(map))
        .map_err(|err| Error::new(err.to_string()))?;
    std::fs::write(path, text).map_err(|err| Error::new(err.to_string()))
}

#[derive(Serialize, Deserialize)]
struct RoutingFile {
    class: String,
    sector_name: String,
    used: bool,
    zone: String,
    category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    is_internet: Option<bool>,
    second_name: String,
    print_cert: bool,
    auto_print: bool,
    no_postscript: bool,
}

fn read_routing(path: &Path) -> Option<Routing> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: RoutingFile = serde_yaml::from_str(&text).ok()?;
    Some(Routing {
        sector_name: file.sector_name,
        used: file.used,
        zone: file.zone,
        category: file.category,
        is_internet: file.is_internet,
        second_name: file.second_name,
        print_cert: file.print_cert,
        auto_print: file.auto_print,
        no_postscript: file.no_postscript,
    })
}

fn write_routing(home: &FileHome, name: &str, class: &str, routing: &Routing) -> Result<(), Error> {
    let path = home
        .base
        .join(&home.event_id)
        .join(format!("pr_{name}.yml"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| Error::new(err.to_string()))?;
    }
    let file = RoutingFile {
        class: class.to_string(),
        sector_name: routing.sector_name.clone(),
        used: routing.used,
        zone: routing.zone.clone(),
        category: routing.category.clone(),
        is_internet: routing.is_internet,
        second_name: routing.second_name.clone(),
        print_cert: routing.print_cert,
        auto_print: routing.auto_print,
        no_postscript: routing.no_postscript,
    };
    let text = serde_yaml::to_string(&file).map_err(|err| Error::new(err.to_string()))?;
    std::fs::write(path, text).map_err(|err| Error::new(err.to_string()))
}

fn eligible(current: &[Printer], job: &Job) -> Vec<Printer> {
    let mut pool: Vec<Printer> = current
        .iter()
        .filter(|printer| printer.connected && printer.installed && printer.routing.used)
        .cloned()
        .collect();
    if job.name.is_some() {
        return pool;
    }
    if job.certificate {
        pool.retain(|printer| printer.routing.print_cert);
    } else if let Some(category) = job
        .category
        .as_deref()
        .filter(|category| !category.is_empty())
    {
        let bound = pool
            .iter()
            .any(|printer| printer.routing.category == category);
        if bound {
            pool.retain(|printer| printer.routing.category == category);
        } else {
            pool.retain(|printer| printer.routing.category.is_empty());
        }
    }
    let flagged = pool
        .iter()
        .any(|printer| printer.routing.is_internet.is_some());
    if flagged {
        pool.retain(|printer| printer.routing.is_internet == Some(job.internet));
    }
    if let Some(zone) = job.zone.as_deref().filter(|zone| !zone.is_empty()) {
        pool.retain(|printer| printer.routing.zone == zone);
    }
    pool
}

fn select(service: &str, mut pool: Vec<Printer>, job: &Job) -> Option<Printer> {
    if let Some(name) = &job.name {
        return pool.into_iter().find(|printer| printer.name == *name);
    }
    if pool.is_empty() {
        return None;
    }
    if pool.len() == 1 {
        return pool.pop();
    }
    if job.rule.as_deref() == Some("fewest_jobs") {
        let mut ranked: Vec<(Option<i32>, Printer)> = pool
            .into_iter()
            .map(|printer| {
                let count = queued_jobs(service, &printer.name);
                (count, printer)
            })
            .collect();
        if ranked.iter().all(|(count, _)| count.is_some()) {
            ranked.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then(left.1.last_print.cmp(&right.1.last_print))
                    .then(left.1.name.cmp(&right.1.name))
            });
        } else {
            ranked.sort_by(|left, right| {
                left.1
                    .last_print
                    .cmp(&right.1.last_print)
                    .then(left.1.name.cmp(&right.1.name))
            });
        }
        return ranked.into_iter().next().map(|(_, printer)| printer);
    }
    pool.sort_by(|left, right| {
        left.last_print
            .cmp(&right.last_print)
            .then(left.name.cmp(&right.name))
    });
    pool.into_iter().next()
}

fn remember_print(slot: &mut Slot, name: &str, at: u64) -> Option<Printer> {
    let mut updated = None;
    for printer in &mut slot.current {
        if printer.name == name {
            printer.last_print = at;
            updated = Some(printer.clone());
        }
    }
    let updated = updated?;
    for printer in &mut slot.devices {
        if printer.name == name {
            printer.last_print = at;
        }
    }
    for printer in &mut slot.searching {
        if printer.name == name {
            printer.last_print = at;
        }
    }
    Some(updated)
}

const PRINT_JOB: u16 = 0x0002;
const CANCEL_JOB: u16 = 0x0008;
const GET_JOB_ATTRIBUTES: u16 = 0x0009;
const JOB_PENDING: i32 = 3;
const JOB_PENDING_HELD: i32 = 4;
const JOB_PROCESSING: i32 = 5;
const JOB_STOPPED: i32 = 6;
const JOB_COMPLETED: i32 = 9;
const JOB_POLL: std::time::Duration = std::time::Duration::from_millis(100);
const JOB_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Completed,
    Aborted,
    Stopped,
    Running,
}

fn prepare_page(printer: &Printer, page: &Page) -> (String, Vec<u8>, BTreeMap<String, String>) {
    let mut options = printer.values.options.clone();
    match page {
        Page::Graphic { bytes, width_px } => {
            options
                .entry("zeMediaTracking".to_string())
                .or_insert_with(|| "Web".to_string());
            options
                .entry("MediaType".to_string())
                .or_insert_with(|| "Direct".to_string());
            options
                .entry("zePrintMode".to_string())
                .or_insert_with(|| "Cutter".to_string());
            let document = if printer.kind == Kind::Zpl {
                wrap_zpl(printer, bytes, *width_px)
            } else {
                bytes.clone()
            };
            ("application/octet-stream".to_string(), document, options)
        }
        Page::Png {
            bytes,
            width_in,
            height_in,
        } => {
            if printer.make_model.to_ascii_lowercase().contains("evolis") {
                options
                    .entry("InkType".to_string())
                    .or_insert_with(|| "Black".to_string());
            }
            options.insert("media".to_string(), format_media(*width_in, *height_in));
            ("image/png".to_string(), bytes.clone(), options)
        }
        Page::Pdf { bytes } => ("application/pdf".to_string(), bytes.clone(), options),
    }
}

fn format_media(width_in: f64, height_in: f64) -> String {
    format!("{}x{}in", trim_inches(width_in), trim_inches(height_in))
}

fn trim_inches(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn wrap_zpl(printer: &Printer, graphic: &[u8], width_px: u32) -> Vec<u8> {
    let option = |key: &str, default: &str| -> String {
        printer
            .values
            .options
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default.to_string())
    };
    let mode = match option("zePrintMode", "Cutter").as_str() {
        "Tear" => "T",
        "Peel" => "P",
        "Rewind" => "R",
        "Applicator" => "A",
        _ => "C",
    };
    let width_in = printer.width_in.unwrap_or(4.1);
    let printer_px = (width_in * f64::from(printer.dpi)).round() as i64;
    let x_shift = (printer_px - i64::from(width_px)) / 2;
    let mut out = format!(
        "^XA\n^PR{}\n^MD{}\n^MM{mode}\n^LS{}\n^LT{}\n~TA{}\n",
        option("Speed", "10"),
        option("Darkness", "0"),
        option("zpl_label_shift", "0"),
        option("zpl_label_top", "0"),
        option("zpl_tear_off", "0"),
    )
    .into_bytes();
    out.extend_from_slice(graphic);
    out.extend_from_slice(format!("\n^FO{x_shift},0\n^XZ\n").as_bytes());
    out
}

fn send_job(
    service: &str,
    printer: &Printer,
    format: &str,
    options: &BTreeMap<String, String>,
    document: &[u8],
) -> Result<i32, Error> {
    let message = if printer.class == "ipp" {
        let target =
            IppPrinter::from_uri(&printer.device_uri).map_err(|err| Error::new(err.to_string()))?;
        let owned: Vec<(String, String)> = options
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        let pairs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        target
            .print_job(format, &pairs, document)
            .map_err(|err| Error::new(err.to_string()))?
    } else {
        let body = native_request(PRINT_JOB, printer, format, options, &[], document);
        post_native(service, &body)?
    };
    job_number(&message, "job-id")
}

/// Pending, pending-held, and processing stay open. The wait polls them until a
/// terminal state, or until [`JOB_WAIT`]. A job still running at the bound is
/// left on the printer.
fn wait_job(service: &str, printer: &Printer, id: i32) -> Result<Outcome, Error> {
    let started = std::time::Instant::now();
    loop {
        match outcome_of(read_job(service, printer, id)?) {
            Outcome::Running if started.elapsed() < JOB_WAIT => {
                std::thread::sleep(JOB_POLL);
            }
            outcome => return Ok(outcome),
        }
    }
}

/// One more job-state read before another `Print-Job`. A completed job is kept.
/// A job that is still pending or processing is kept and is not sent again.
/// Any other state may be cancelled and sent again.
fn settled_job(service: &str, printer: &Printer, id: i32) -> Result<Option<i32>, Error> {
    match outcome_of(read_job(service, printer, id)?) {
        Outcome::Completed => Ok(Some(id)),
        Outcome::Running => Err(Error::new("job still processing")),
        Outcome::Aborted | Outcome::Stopped => Ok(None),
    }
}

fn outcome_of(job: (i32, Vec<String>)) -> Outcome {
    let (state, reasons) = job;
    if state == JOB_COMPLETED {
        return Outcome::Completed;
    }
    if state == JOB_STOPPED || reasons.iter().any(|reason| reason.contains("stopped")) {
        return Outcome::Stopped;
    }
    if matches!(state, JOB_PENDING | JOB_PENDING_HELD | JOB_PROCESSING) {
        return Outcome::Running;
    }
    Outcome::Aborted
}

fn read_job(service: &str, printer: &Printer, id: i32) -> Result<(i32, Vec<String>), Error> {
    let message = if printer.class == "ipp" {
        let target =
            IppPrinter::from_uri(&printer.device_uri).map_err(|err| Error::new(err.to_string()))?;
        target
            .job_attributes(id)
            .map_err(|err| Error::new(err.to_string()))?
    } else {
        let body = native_request(
            GET_JOB_ATTRIBUTES,
            printer,
            "",
            &BTreeMap::new(),
            &[("job-id", ipp_tool::Value::Integer(id))],
            &[],
        );
        post_native(service, &body)?
    };
    let state = job_number(&message, "job-state")?;
    let reasons = message.texts("job-state-reasons");
    Ok((state, reasons))
}

fn cancel_job(service: &str, printer: &Printer, id: i32) -> Result<(), Error> {
    if printer.class == "ipp" {
        let target =
            IppPrinter::from_uri(&printer.device_uri).map_err(|err| Error::new(err.to_string()))?;
        return target
            .cancel_job(id)
            .map_err(|err| Error::new(err.to_string()));
    }
    let body = native_request(
        CANCEL_JOB,
        printer,
        "",
        &BTreeMap::new(),
        &[("job-id", ipp_tool::Value::Integer(id))],
        &[],
    );
    post_native(service, &body).map(|_| ())
}

fn resume_printer(service: &str, printer: &Printer) -> Result<(), Error> {
    if printer.class == "ipp" {
        let target =
            IppPrinter::from_uri(&printer.device_uri).map_err(|err| Error::new(err.to_string()))?;
        return target.resume().map_err(|err| Error::new(err.to_string()));
    }
    resume_queue(service, &printer.name)
}

fn native_request(
    operation: u16,
    printer: &Printer,
    format: &str,
    options: &BTreeMap<String, String>,
    extra: &[(&str, ipp_tool::Value)],
    document: &[u8],
) -> Vec<u8> {
    let printer_uri = format!("ipp://localhost/printers/{}", printer.name);
    let mut attributes = vec![("printer-uri".to_string(), ipp_tool::Value::Uri(printer_uri))];
    if !format.is_empty() {
        attributes.push((
            "document-format".to_string(),
            ipp_tool::Value::Mime(format.to_string()),
        ));
    }
    for (name, value) in extra {
        attributes.push(((*name).to_string(), value.clone()));
    }
    for (name, value) in options {
        attributes.push((name.clone(), ipp_tool::Value::Keyword(value.clone())));
    }
    let operation = ipp_tool::Operation::from_id(operation).expect("print operation");
    let pairs: Vec<(&str, Vec<ipp_tool::Value>)> = attributes
        .iter()
        .map(|(name, value)| (name.as_str(), vec![value.clone()]))
        .collect();
    let mut body = ipp_tool::encode_request(operation, 1, &pairs, &[]);
    body.extend_from_slice(document);
    body
}

fn post_native(service: &str, body: &[u8]) -> Result<ipp_tool::Message, Error> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let response =
            native_print::cups_submit(service, body).map_err(|err| Error::new(err.to_string()))?;
        let message = ipp_tool::decode(&response).map_err(|err| Error::new(err.to_string()))?;
        if message.code >= 0x0400 {
            return Err(Error::new(format!("IPP status {:#06x}", message.code)));
        }
        return Ok(message);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (service, body);
        Err(Error::new("native print is unavailable"))
    }
}

fn job_number(message: &ipp_tool::Message, name: &str) -> Result<i32, Error> {
    match message
        .named(name)
        .and_then(|attribute| attribute.values.first())
    {
        Some(ipp_tool::Value::Integer(value) | ipp_tool::Value::Enum(value)) => Ok(*value),
        _ => Err(Error::new(format!("{name} missing"))),
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn queued_jobs(service: &str, name: &str) -> Option<i32> {
    #[cfg(windows)]
    {
        let _ = service;
        return native_print::spooler_jobs(name);
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let uri = format!("ipp://localhost/printers/{name}");
        let response = native_print::cups_call(
            service,
            0x000b,
            &[
                ("printer-uri", 0x45, uri.as_str()),
                ("requested-attributes", 0x44, "queued-job-count"),
            ],
        );
        let Ok(response) = response else {
            return None;
        };
        let Ok(message) = ipp_tool::decode(&response) else {
            return None;
        };
        return match message
            .named("queued-job-count")
            .and_then(|attribute| attribute.values.first())
        {
            Some(ipp_tool::Value::Integer(count) | ipp_tool::Value::Enum(count)) => Some(*count),
            _ => None,
        };
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (service, name);
        None
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    mod queues {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{Arc, Condvar, Mutex};
        use std::thread;
        use std::time::Duration;

        use ipp_tool::{Browse, Catalog, Report as IppReport, Service};

        use super::super::super::native_print::{self, Report as NativeReport};
        use super::super::Change;

        use super::super::Search;

        #[derive(Clone)]
        pub(super) struct Listed {
            pub(super) name: &'static str,
            pub(super) uri: &'static str,
            pub(super) make_model: &'static str,
        }

        pub(super) struct FoundDevice {
            pub(super) uri: String,
            pub(super) make_model: String,
        }

        fn four() -> Vec<Listed> {
            vec![
                Listed {
                    name: "zebra",
                    uri: "usb://Zebra/ZTC?serial=ABC",
                    make_model: "Zebra Technologies ZTC GX430t",
                },
                Listed {
                    name: "tsc",
                    uri: "usb://TSC/TTP",
                    make_model: "TSC",
                },
                Listed {
                    name: "evolis",
                    uri: "usb://Evolis/Zenius?serial=E1",
                    make_model: "Evolis Zenius",
                },
                Listed {
                    name: "hp",
                    uri: "socket://192.0.2.20:9100",
                    make_model: "HP LaserJet",
                },
            ]
        }

        pub(super) struct FixedBrowse {
            services: Vec<Service>,
            calls: AtomicUsize,
            held: Mutex<bool>,
            cv: Condvar,
        }

        impl Browse for FixedBrowse {
            fn browse(&self) -> Vec<Service> {
                let mut held = self.held.lock().unwrap();
                while *held {
                    held = self.cv.wait(held).unwrap();
                }
                drop(held);
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.services.clone()
            }
        }

        impl FixedBrowse {
            fn release(&self) {
                *self.held.lock().unwrap() = false;
                self.cv.notify_all();
            }
        }

        pub(super) fn no_browse() -> FixedBrowse {
            FixedBrowse {
                services: Vec::new(),
                calls: AtomicUsize::new(0),
                held: Mutex::new(false),
                cv: Condvar::new(),
            }
        }

        pub(super) struct Cups {
            pub(super) base: String,
            pub(super) ops: Arc<Mutex<Vec<u16>>>,
            pub(super) bodies: Arc<Mutex<Vec<Vec<u8>>>>,
            pub(super) listed: Arc<Mutex<Vec<Listed>>>,
            pub(super) devices: Arc<Mutex<Vec<FoundDevice>>>,
            pub(super) ppds: Arc<Mutex<Vec<(String, String)>>>,
            pub(super) states: Arc<Mutex<std::collections::HashMap<String, (i32, bool)>>>,
            pub(super) jobs: Arc<Mutex<std::collections::HashMap<String, i32>>>,
            /// `job-state` for each following `Print-Job`, in order. An empty script completes.
            pub(super) job_script: Arc<Mutex<Vec<i32>>>,
            /// `job-state` for a later `Get-Job-Attributes` of a job that already reported
            /// a terminal state. The first read stays on `job_script`.
            pub(super) job_follow: Arc<Mutex<Vec<i32>>>,
            pub(super) hold: Arc<AtomicBool>,
            pub(super) device_hold: Arc<AtomicBool>,
            pub(super) install_hold: Arc<AtomicBool>,
            pub(super) limit: Arc<AtomicUsize>,
            pub(super) stop: Arc<AtomicBool>,
            pub(super) on_job: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
            pub(super) handle: Option<thread::JoinHandle<()>>,
        }

        impl Cups {
            pub(super) fn open(printers: &[Listed]) -> Self {
                Self::open_held(printers, false)
            }

            pub(super) fn open_held(printers: &[Listed], held: bool) -> Self {
                let ppd = ppd_response();
                let ops = Arc::new(Mutex::new(Vec::new()));
                let recorded = Arc::clone(&ops);
                let bodies = Arc::new(Mutex::new(Vec::new()));
                let saved = Arc::clone(&bodies);
                let listed = Arc::new(Mutex::new(printers.to_vec()));
                let queues = Arc::clone(&listed);
                let devices = Arc::new(Mutex::new(Vec::new()));
                let found = Arc::clone(&devices);
                let ppds = Arc::new(Mutex::new(Vec::new()));
                let drivers = Arc::clone(&ppds);
                let states = Arc::new(Mutex::new(std::collections::HashMap::new()));
                let queue_states = Arc::clone(&states);
                let jobs = Arc::new(Mutex::new(std::collections::HashMap::new()));
                let queued = Arc::clone(&jobs);
                let job_script = Arc::new(Mutex::new(Vec::new()));
                let script = Arc::clone(&job_script);
                let job_follow = Arc::new(Mutex::new(Vec::new()));
                let follow = Arc::clone(&job_follow);
                let hold = Arc::new(AtomicBool::new(held));
                let gate = Arc::clone(&hold);
                let device_hold = Arc::new(AtomicBool::new(false));
                let device_gate = Arc::clone(&device_hold);
                let install_hold = Arc::new(AtomicBool::new(false));
                let install_gate = Arc::clone(&install_hold);
                let limit = Arc::new(AtomicUsize::new(usize::MAX));
                let budget = Arc::clone(&limit);
                let stop = Arc::new(AtomicBool::new(false));
                let flag = Arc::clone(&stop);
                let on_job: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>> =
                    Arc::new(Mutex::new(None));
                let job_hook = Arc::clone(&on_job);
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.set_nonblocking(true).unwrap();
                let addr = listener.local_addr().unwrap();
                let handle = thread::spawn(move || {
                    let mut next_job = 1i32;
                    let mut job_state = std::collections::HashMap::<i32, i32>::new();
                    let mut job_gets = std::collections::HashMap::<i32, u32>::new();
                    while !flag.load(Ordering::SeqCst) {
                        let mut sock = match listener.accept() {
                            Ok((sock, _)) => sock,
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(20));
                                continue;
                            }
                            Err(_) => break,
                        };
                        sock.set_nonblocking(false).unwrap();
                        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        let body = read_body(&mut sock);
                        let op = operation_id(&body);
                        let text = String::from_utf8_lossy(&body).into_owned();
                        recorded.lock().unwrap().push(op);
                        saved.lock().unwrap().push(body.clone());
                        while !flag.load(Ordering::SeqCst)
                            && ((op == 0x400b && device_gate.load(Ordering::SeqCst))
                                || (op == 0x400c && install_gate.load(Ordering::SeqCst))
                                || gate.load(Ordering::SeqCst)
                                || recorded.lock().unwrap().len() > budget.load(Ordering::SeqCst))
                        {
                            thread::sleep(Duration::from_millis(20));
                        }
                        let payload = match op {
                            0x400f => ppd.clone(),
                            0x4002 => cups_response(
                                &queues.lock().unwrap(),
                                &queue_states.lock().unwrap(),
                            ),
                            0x400b => device_response(&found.lock().unwrap()),
                            0x400c => ppds_response(&drivers.lock().unwrap()),
                            0x000b => jobs_response(&text, &queued.lock().unwrap()),
                            0x0002 => {
                                if let Some(hook) = job_hook.lock().unwrap().clone() {
                                    hook();
                                }
                                let state = {
                                    let mut pending = script.lock().unwrap();
                                    if pending.is_empty() {
                                        9
                                    } else {
                                        pending.remove(0)
                                    }
                                };
                                let id = next_job;
                                next_job += 1;
                                job_state.insert(id, state);
                                job_created(id)
                            }
                            0x0009 => {
                                let id = job_id_in(&body).unwrap_or(next_job.saturating_sub(1));
                                let count = job_gets.entry(id).or_insert(0);
                                *count += 1;
                                let again = *count > 1;
                                let staged = if again {
                                    let mut staged = follow.lock().unwrap();
                                    if staged.is_empty() {
                                        None
                                    } else {
                                        Some(staged.remove(0))
                                    }
                                } else {
                                    None
                                };
                                let state = if let Some(state) = staged {
                                    job_state.insert(id, state);
                                    state
                                } else {
                                    let state = job_state.get(&id).copied().unwrap_or(9);
                                    if matches!(state, 3 | 4 | 5) {
                                        let mut pending = script.lock().unwrap();
                                        if !pending.is_empty() {
                                            let next = pending.remove(0);
                                            job_state.insert(id, next);
                                        }
                                    }
                                    state
                                };
                                job_status(id, state)
                            }
                            _ => empty_ok(),
                        };
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            payload.len()
                        );
                        let _ = sock.write_all(head.as_bytes());
                        let _ = sock.write_all(&payload);
                    }
                });
                Self {
                    base: format!("http://{addr}"),
                    ops,
                    bodies,
                    listed,
                    devices,
                    ppds,
                    states,
                    jobs,
                    job_script,
                    job_follow,
                    hold,
                    device_hold,
                    install_hold,
                    limit,
                    stop,
                    on_job,
                    handle: Some(handle),
                }
            }
        }

        impl Drop for Cups {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::SeqCst);
                if let Some(handle) = self.handle.take() {
                    let _ = handle.join();
                }
            }
        }

        pub(super) struct Ipp {
            pub(super) port: u16,
            pub(super) ops: Arc<std::sync::Mutex<Vec<u16>>>,
            pub(super) bodies: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
            pub(super) limit: Arc<AtomicUsize>,
            /// `job-state` for each following `Get-Job-Attributes`. An empty script completes.
            pub(super) job_script: Arc<Mutex<Vec<i32>>>,
            stop: Arc<AtomicBool>,
            handle: Option<thread::JoinHandle<()>>,
        }

        impl Ipp {
            pub(super) fn open() -> Self {
                let ops = Arc::new(std::sync::Mutex::new(Vec::new()));
                let recorded = Arc::clone(&ops);
                let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
                let saved = Arc::clone(&bodies);
                let stop = Arc::new(AtomicBool::new(false));
                let flag = Arc::clone(&stop);
                let served = Arc::new(AtomicUsize::new(0));
                let limit = Arc::new(AtomicUsize::new(usize::MAX));
                let budget = Arc::clone(&limit);
                let job_script = Arc::new(Mutex::new(Vec::new()));
                let states = Arc::clone(&job_script);
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.set_nonblocking(true).unwrap();
                let port = listener.local_addr().unwrap().port();
                let tls_config = ipp_tls();
                let handle = thread::spawn(move || {
                    while !flag.load(Ordering::SeqCst) {
                        let mut sock = match listener.accept() {
                            Ok((sock, _)) => sock,
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(20));
                                continue;
                            }
                            Err(_) => break,
                        };
                        sock.set_nonblocking(false).unwrap();
                        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        let mut peeked = [0u8; 1];
                        let secure = sock.peek(&mut peeked).ok() == Some(1) && peeked[0] == 0x16;
                        if secure {
                            let Ok(mut tls) = accept_tls(sock, &tls_config) else {
                                continue;
                            };
                            let body = read_any(&mut tls);
                            let payload = ipp_payload(
                                &body, &recorded, &saved, &flag, &served, &budget, &states,
                            );
                            let _ = write_http(&mut tls, &payload);
                        } else {
                            let body = read_body(&mut sock);
                            let payload = ipp_payload(
                                &body, &recorded, &saved, &flag, &served, &budget, &states,
                            );
                            let _ = write_http(&mut sock, &payload);
                        }
                    }
                });
                Self {
                    port,
                    ops,
                    bodies,
                    limit,
                    job_script,
                    stop,
                    handle: Some(handle),
                }
            }

            pub(super) fn service(&self, address: &str) -> Service {
                Service {
                    kind: "_ipp._tcp".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: self.port,
                    rp: Some("ipp/print".to_string()),
                    address: Some(address.to_string()),
                }
            }
        }

        impl Drop for Ipp {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::SeqCst);
                if let Some(handle) = self.handle.take() {
                    let _ = handle.join();
                }
            }
        }

        fn read_body(sock: &mut std::net::TcpStream) -> Vec<u8> {
            read_any(sock)
        }

        fn read_any(sock: &mut impl Read) -> Vec<u8> {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                if let Some(body) = framed(&buf) {
                    return body;
                }
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => return buf,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            }
        }

        fn write_http(sock: &mut impl Write, payload: &[u8]) -> std::io::Result<()> {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            sock.write_all(head.as_bytes())?;
            sock.write_all(payload)
        }

        fn ipp_payload(
            body: &[u8],
            recorded: &Mutex<Vec<u16>>,
            saved: &Mutex<Vec<Vec<u8>>>,
            flag: &AtomicBool,
            served: &AtomicUsize,
            budget: &AtomicUsize,
            states: &Mutex<Vec<i32>>,
        ) -> Vec<u8> {
            let op = operation_id(body);
            recorded.lock().unwrap().push(op);
            saved.lock().unwrap().push(body.to_vec());
            while !flag.load(Ordering::SeqCst)
                && served.load(Ordering::SeqCst) >= budget.load(Ordering::SeqCst)
            {
                thread::sleep(Duration::from_millis(20));
            }
            served.fetch_add(1, Ordering::SeqCst);
            match op {
                0x0002 => job_created(1),
                0x0009 => job_status(1, next_job_state(states)),
                _ => attributes_response(),
            }
        }

        fn next_job_state(states: &Mutex<Vec<i32>>) -> i32 {
            let mut pending = states.lock().unwrap();
            if pending.is_empty() {
                9
            } else {
                pending.remove(0)
            }
        }

        /// The stand-in answers `ipps://` with TLS and `ipp://` with cleartext on the same port.
        fn ipp_tls() -> Arc<rustls::ServerConfig> {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let rcgen::CertifiedKey { cert, signing_key } =
                rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
            let certs = vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())];
            let key = rustls::pki_types::PrivateKeyDer::try_from(signing_key.serialize_der())
                .expect("test key");
            Arc::new(
                rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(certs, key)
                    .expect("test certificate"),
            )
        }

        fn accept_tls(
            sock: std::net::TcpStream,
            config: &Arc<rustls::ServerConfig>,
        ) -> Result<rustls::StreamOwned<rustls::ServerConnection, std::net::TcpStream>, rustls::Error>
        {
            let conn = rustls::ServerConnection::new(Arc::clone(config))?;
            Ok(rustls::StreamOwned::new(conn, sock))
        }

        fn framed(buf: &[u8]) -> Option<Vec<u8>> {
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

        fn cups_response(
            printers: &[Listed],
            states: &std::collections::HashMap<String, (i32, bool)>,
        ) -> Vec<u8> {
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            for printer in printers {
                let (state, accepting) = states.get(printer.name).copied().unwrap_or((3, true));
                out.push(0x04);
                write_text(&mut out, 0x42, "printer-name", printer.name);
                write_text(&mut out, 0x45, "device-uri", printer.uri);
                write_text(&mut out, 0x41, "printer-make-and-model", printer.make_model);
                write_enum(&mut out, "printer-state", state);
                write_bool(&mut out, "printer-is-accepting-jobs", accepting);
            }
            out.push(0x03);
            out
        }

        fn device_response(devices: &[FoundDevice]) -> Vec<u8> {
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            for device in devices {
                out.push(0x04);
                write_text(&mut out, 0x45, "device-uri", &device.uri);
                write_text(&mut out, 0x41, "device-make-and-model", &device.make_model);
            }
            out.push(0x03);
            out
        }

        fn ppds_response(ppds: &[(String, String)]) -> Vec<u8> {
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            for (name, make) in ppds {
                out.push(0x04);
                write_text(&mut out, 0x42, "ppd-name", name);
                write_text(&mut out, 0x41, "ppd-make-and-model", make);
            }
            out.push(0x03);
            out
        }

        fn empty_ok() -> Vec<u8> {
            vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03]
        }

        fn ppd_response() -> Vec<u8> {
            let ppd = "\
*OpenUI *PageSize/Media Size: PickOne
*PageSize Letter/US Letter: \"\"
*CloseUI: *PageSize
*OpenUI *Resolution/Output Resolution: PickOne
*Resolution 300dpi/300 DPI: \"\"
*CloseUI: *Resolution
";
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            out.push(0x03);
            out.extend_from_slice(ppd.as_bytes());
            out
        }

        fn attributes_response() -> Vec<u8> {
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            out.push(0x04);
            write_text(&mut out, 0x41, "printer-make-and-model", "HP LaserJet");
            write_text(
                &mut out,
                0x49,
                "document-format-supported",
                "application/pdf",
            );
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

        fn write_int(out: &mut Vec<u8>, name: &str, value: i32) {
            out.push(0x21);
            counted(out, name.as_bytes());
            out.extend_from_slice(&4u16.to_be_bytes());
            out.extend_from_slice(&value.to_be_bytes());
        }

        fn job_created(id: i32) -> Vec<u8> {
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            out.push(0x02);
            write_int(&mut out, "job-id", id);
            out.push(0x03);
            out
        }

        fn job_status(id: i32, state: i32) -> Vec<u8> {
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            out.push(0x02);
            write_int(&mut out, "job-id", id);
            write_enum(&mut out, "job-state", state);
            out.push(0x03);
            out
        }

        fn job_id_in(body: &[u8]) -> Option<i32> {
            let message = ipp_tool::decode(body).ok()?;
            match message
                .named("job-id")
                .and_then(|attribute| attribute.values.first())
            {
                Some(ipp_tool::Value::Integer(id) | ipp_tool::Value::Enum(id)) => Some(*id),
                _ => None,
            }
        }

        fn jobs_response(body: &str, counts: &std::collections::HashMap<String, i32>) -> Vec<u8> {
            let mut names: Vec<&String> = counts.keys().collect();
            names.sort_by_key(|name| std::cmp::Reverse(name.len()));
            let count = names
                .into_iter()
                .find(|name| body.contains(name.as_str()))
                .map(|name| counts[name])
                .unwrap_or(0);
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            out.push(0x04);
            // A negative stand-in count leaves the attribute out. The route then
            // treats the depth as unknown and orders the pool by last_print.
            if count >= 0 {
                write_int(&mut out, "queued-job-count", count);
            }
            out.push(0x03);
            out
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
        fn queues_become_printers() {
            let cups = Cups::open(&four());
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let printers = search.printers();
            assert_eq!(printers.len(), 4);
            let kinds: Vec<_> = printers
                .iter()
                .map(|printer| printer.kind.as_str())
                .collect();
            assert_eq!(kinds, ["zpl", "zpl", "raster", "raster"]);
            assert_eq!(
                printers
                    .iter()
                    .map(|printer| printer.dpi)
                    .collect::<Vec<_>>(),
                vec![300, 203, 300, 300]
            );
            assert_eq!(printers[0].width_in, Some(4.1));
            assert_eq!(printers[1].width_in, Some(4.1));
            assert_eq!(printers[2].width_in, Some(2.125));
            assert!(printers[3].width_in.is_none());
            for printer in printers {
                assert!(printer
                    .choices
                    .iter()
                    .all(|choice| choice.name != "PageSize"));
                assert!(printer
                    .choices
                    .iter()
                    .any(|choice| choice.name == "Resolution"));
            }
        }

        #[test]
        fn search_sends_get_printers() {
            let cups = Cups::open(&four());
            let ipp = Ipp::open();
            let browse = Arc::new(FixedBrowse {
                services: vec![ipp.service("192.0.2.10")],
                calls: AtomicUsize::new(0),
                held: Mutex::new(false),
                cv: Condvar::new(),
            });
            Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let cups_ops = cups.ops.lock().unwrap().clone();
            let ipp_ops = ipp.ops.lock().unwrap().clone();
            assert_eq!(cups_ops.iter().filter(|op| **op == 0x4002).count(), 1);
            assert!(!cups_ops.contains(&0x0002));
            assert_eq!(browse.calls.load(Ordering::SeqCst), 1);
            assert!(!ipp_ops.contains(&0x0002));
        }

        #[test]
        fn ipp_queue_reuses_the_address() {
            let cups = Cups::open_held(
                &[Listed {
                    name: "desk",
                    uri: "ipp://192.0.2.10/ipp/print",
                    make_model: "HP LaserJet",
                }],
                true,
            );
            let ipp = Ipp::open();
            let browse = Arc::new(FixedBrowse {
                services: vec![ipp.service("192.0.2.10")],
                calls: AtomicUsize::new(0),
                held: Mutex::new(false),
                cv: Condvar::new(),
            });
            let search = Search::start(&cups.base, Arc::clone(&browse));
            let ready = std::time::Instant::now();
            while !search
                .printers()
                .iter()
                .any(|printer| printer.host == "192.0.2.10")
            {
                assert!(ready.elapsed() < Duration::from_secs(2), "browse result");
                thread::sleep(Duration::from_millis(20));
            }
            cups.hold.store(false, Ordering::SeqCst);
            search.wait().unwrap();
            let printers = search.printers();
            assert_eq!(printers.len(), 1);
            assert_eq!(printers[0].name, "desk");
            assert_eq!(
                printers[0].formats.supported,
                vec!["application/pdf".to_string()]
            );
            assert!(printers[0].connected);
            let ipp_ops = ipp.ops.lock().unwrap().clone();
            assert_eq!(ipp_ops.iter().filter(|op| **op == 0x000b).count(), 1);
        }

        #[test]
        fn usb_queue_stays_native() {
            let cups = Cups::open(&[Listed {
                name: "zebra",
                uri: "usb://Zebra/ZTC?serial=ABC",
                make_model: "Zebra Technologies ZTC GX430t",
            }]);
            let ipp = Ipp::open();
            let browse = Arc::new(FixedBrowse {
                services: vec![ipp.service("192.0.2.10")],
                calls: AtomicUsize::new(0),
                held: Mutex::new(false),
                cv: Condvar::new(),
            });
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let printers = search.printers();
            let zebra = printers
                .iter()
                .find(|printer| printer.name == "zebra")
                .unwrap();
            assert_eq!(zebra.device_uri, "usb://Zebra/ZTC?serial=ABC");
            assert!(zebra.host.is_empty());
            assert!(zebra.formats.supported.is_empty());
            assert!(!zebra.connected);
            assert!(zebra.installed);
        }

        #[test]
        fn browse_only_printer_is_extra() {
            let cups = Cups::open(&[Listed {
                name: "zebra",
                uri: "usb://Zebra/ZTC?serial=ABC",
                make_model: "Zebra Technologies ZTC GX430t",
            }]);
            let ipp = Ipp::open();
            let browse = Arc::new(FixedBrowse {
                services: vec![ipp.service("192.0.2.10")],
                calls: AtomicUsize::new(0),
                held: Mutex::new(false),
                cv: Condvar::new(),
            });
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let printers = search.printers();
            assert_eq!(printers.len(), 2);
            let extra = printers
                .iter()
                .find(|printer| printer.host == "192.0.2.10")
                .unwrap();
            assert!(extra.connected);
            assert_eq!(extra.formats.supported, vec!["application/pdf".to_string()]);
        }

        #[test]
        fn add_takes_a_uri() {
            let cups = Cups::open(&[]);
            let ipp = Ipp::open();
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let calls = browse.calls.load(Ordering::SeqCst);
            let uri = format!("ipps://127.0.0.1:{}/ipp/print", ipp.port);
            search.add(&uri).unwrap();
            assert_eq!(browse.calls.load(Ordering::SeqCst), calls);
            assert!(search
                .printers()
                .iter()
                .any(|printer| printer.device_uri == uri));
            let ipp_ops = ipp.ops.lock().unwrap().clone();
            assert!(ipp_ops.contains(&0x000b));
            assert!(!ipp_ops.contains(&0x0002));
        }

        #[test]
        fn refresh_drops_a_silent_printer() {
            let cups = Cups::open(&[]);
            let mut ipp = Ipp::open();
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let uri = format!("ipp://127.0.0.1:{}/ipp/print", ipp.port);
            search.add(&uri).unwrap();
            assert_eq!(search.printers().len(), 1);
            ipp.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = ipp.handle.take() {
                let _ = handle.join();
            }
            search.refresh();
            search.wait().unwrap();
            assert!(search.printers().is_empty());
        }

        #[test]
        fn a_new_printer_is_shown_before_the_search_ends() {
            let cups = Cups::open_held(
                &[Listed {
                    name: "zebra",
                    uri: "usb://Zebra/ZTC?serial=ABC",
                    make_model: "Zebra Technologies ZTC GX430t",
                }],
                true,
            );
            let browse = Arc::new(FixedBrowse {
                services: Vec::new(),
                calls: AtomicUsize::new(0),
                held: Mutex::new(true),
                cv: Condvar::new(),
            });
            let search = Search::start(&cups.base, Arc::clone(&browse));
            let events = search.subscribe();
            cups.hold.store(false, Ordering::SeqCst);
            let added = events.recv_timeout(Duration::from_secs(2)).unwrap();
            match added {
                super::super::Change::Added(printer) => assert_eq!(printer.name, "zebra"),
                other => panic!("expected an added printer, got {other:?}"),
            }
            assert_eq!(search.printers().len(), 1);
            assert_eq!(browse.calls.load(Ordering::SeqCst), 0);
            browse.release();
            search.wait().unwrap();
            assert_eq!(search.printers().len(), 1);
        }

        #[test]
        fn a_missing_printer_leaves_when_both_finish() {
            let cups = Cups::open(&[Listed {
                name: "zebra",
                uri: "usb://Zebra/ZTC?serial=ABC",
                make_model: "Zebra Technologies ZTC GX430t",
            }]);
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            assert_eq!(search.printers().len(), 1);
            let events = search.subscribe();
            cups.hold.store(true, Ordering::SeqCst);
            cups.listed.lock().unwrap().clear();
            search.refresh();
            thread::sleep(Duration::from_millis(100));
            assert_eq!(search.printers().len(), 1);
            assert!(events.try_recv().is_err());
            cups.hold.store(false, Ordering::SeqCst);
            search.wait().unwrap();
            assert!(search.printers().is_empty());
            match events.recv_timeout(Duration::from_secs(2)).unwrap() {
                super::super::Change::Removed(printer) => assert_eq!(printer.name, "zebra"),
                other => panic!("expected a removed printer, got {other:?}"),
            }
        }

        #[test]
        fn native_ipp_asks_when_the_cache_misses() {
            let ipp = Ipp::open();
            let uri = format!("ipp://127.0.0.1:{}/ipp/print", ipp.port);
            let uri: &'static str = Box::leak(uri.into_boxed_str());
            let cups = Cups::open(&[Listed {
                name: "desk",
                uri,
                make_model: "HP LaserJet",
            }]);
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let printers = search.printers();
            assert_eq!(printers.len(), 1);
            assert_eq!(printers[0].name, "desk");
            assert!(printers[0].connected);
            assert_eq!(
                printers[0].formats.supported,
                vec!["application/pdf".to_string()]
            );
            let ipp_ops = ipp.ops.lock().unwrap().clone();
            assert_eq!(ipp_ops.iter().filter(|op| **op == 0x000b).count(), 1);
            assert!(!ipp_ops.contains(&0x0002));
        }

        #[test]
        fn native_watch_sends_each_queue_then_finished() {
            let cups = Cups::open(&[
                Listed {
                    name: "zebra",
                    uri: "usb://Zebra/ZTC?serial=ABC",
                    make_model: "Zebra Technologies ZTC GX430t",
                },
                Listed {
                    name: "tsc",
                    uri: "usb://TSC/TTP",
                    make_model: "TSC",
                },
            ]);
            cups.limit.store(2, Ordering::SeqCst);
            let events = native_print::watch(Some(cups.base.clone()));
            match events.recv_timeout(Duration::from_secs(2)).unwrap() {
                NativeReport::Queue(queue) => assert_eq!(queue.name, "zebra"),
                other => panic!("expected the first queue, got {other:?}"),
            }
            assert!(events.try_recv().is_err());
            cups.limit.store(usize::MAX, Ordering::SeqCst);
            match events.recv_timeout(Duration::from_secs(2)).unwrap() {
                NativeReport::Queue(queue) => assert_eq!(queue.name, "tsc"),
                other => panic!("expected the second queue, got {other:?}"),
            }
            match events.recv_timeout(Duration::from_secs(2)).unwrap() {
                NativeReport::Finished => {}
                other => panic!("expected the listing to finish, got {other:?}"),
            }
        }

        #[test]
        fn ipp_watch_sends_each_printer_then_finished() {
            let ipp = Ipp::open();
            ipp.limit.store(1, Ordering::SeqCst);
            let browse = Arc::new(FixedBrowse {
                services: vec![
                    Service {
                        kind: "_printer._tcp".to_string(),
                        host: "127.0.0.1".to_string(),
                        port: ipp.port,
                        rp: None,
                        address: Some("192.0.2.12".to_string()),
                    },
                    ipp.service("192.0.2.10"),
                    ipp.service("192.0.2.11"),
                ],
                calls: AtomicUsize::new(0),
                held: Mutex::new(false),
                cv: Condvar::new(),
            });
            let browse: Arc<dyn Browse + Send + Sync> = browse;
            let mut watch = Catalog::watch(browse, Vec::new());
            let details = watch.take_details();
            match watch.recv().unwrap() {
                IppReport::Printer(printer) => {
                    assert_eq!(printer.address, "192.0.2.10");
                    assert_eq!(
                        printer.formats.supported,
                        vec!["application/pdf".to_string()]
                    );
                }
                other => panic!("expected the first printer, got {other:?}"),
            }
            assert!(watch.try_recv().is_err());
            ipp.limit.store(usize::MAX, Ordering::SeqCst);
            match watch.recv().unwrap() {
                IppReport::Printer(printer) => assert_eq!(printer.address, "192.0.2.11"),
                other => panic!("expected the second printer, got {other:?}"),
            }
            match watch.recv().unwrap() {
                IppReport::Finished => {}
                other => panic!("expected the listing to finish, got {other:?}"),
            }
            let ops = ipp.ops.lock().unwrap().clone();
            assert_eq!(ops.iter().filter(|op| **op == 0x000b).count(), 2);
            details
                .send("ipp://127.0.0.1:1/ipp/print".to_string())
                .unwrap();
            match watch.recv().unwrap() {
                IppReport::Missed(uri) => assert_eq!(uri, "ipp://127.0.0.1:1/ipp/print"),
                other => panic!("expected a miss, got {other:?}"),
            }
        }

        #[test]
        fn an_ipp_printer_is_shown_before_native_finishes() {
            let cups = Cups::open_held(
                &[Listed {
                    name: "zebra",
                    uri: "usb://Zebra/ZTC?serial=ABC",
                    make_model: "Zebra Technologies ZTC GX430t",
                }],
                true,
            );
            let ipp = Ipp::open();
            let browse = Arc::new(FixedBrowse {
                services: vec![ipp.service("192.0.2.10")],
                calls: AtomicUsize::new(0),
                held: Mutex::new(true),
                cv: Condvar::new(),
            });
            let search = Search::start(&cups.base, Arc::clone(&browse));
            let events = search.subscribe();
            browse.release();
            match events.recv_timeout(Duration::from_secs(2)).unwrap() {
                Change::Added(printer) => assert_eq!(printer.host, "192.0.2.10"),
                other => panic!("expected the browse result, got {other:?}"),
            }
            assert_eq!(search.printers().len(), 1);
            assert!(cups.hold.load(Ordering::SeqCst));
            cups.hold.store(false, Ordering::SeqCst);
            search.wait().unwrap();
            assert_eq!(search.printers().len(), 2);
        }

        #[test]
        fn a_finished_search_updates_the_same_printer() {
            let cups = Cups::open(&[Listed {
                name: "desk",
                uri: "usb://HP/Laser",
                make_model: "HP LaserJet",
            }]);
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            assert_eq!(search.printers()[0].kind.as_str(), "raster");
            let events = search.subscribe();
            cups.listed.lock().unwrap()[0].make_model = "Zebra Technologies ZTC GX430t";
            search.refresh();
            search.wait().unwrap();
            let printers = search.printers();
            assert_eq!(printers.len(), 1);
            assert_eq!(printers[0].name, "desk");
            assert_eq!(printers[0].kind.as_str(), "zpl");
            match events.recv_timeout(Duration::from_secs(2)).unwrap() {
                Change::Updated(printer) => assert_eq!(printer.name, "desk"),
                other => panic!("expected an update, got {other:?}"),
            }
            assert!(events.try_recv().is_err());
        }

        #[test]
        fn the_same_printer_is_not_added_again() {
            let cups = Cups::open(&[Listed {
                name: "zebra",
                uri: "usb://Zebra/ZTC?serial=ABC",
                make_model: "Zebra Technologies ZTC GX430t",
            }]);
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let events = search.subscribe();
            search.refresh();
            search.wait().unwrap();
            assert_eq!(search.printers().len(), 1);
            assert!(events.try_recv().is_err());
        }

        #[test]
        fn add_shows_the_printer_while_the_listing_is_held() {
            let cups = Cups::open_held(&[], true);
            let ipp = Ipp::open();
            let browse = Arc::new(FixedBrowse {
                services: Vec::new(),
                calls: AtomicUsize::new(0),
                held: Mutex::new(true),
                cv: Condvar::new(),
            });
            let search = Search::start(&cups.base, Arc::clone(&browse));
            let uri = format!("ipps://127.0.0.1:{}/ipp/print", ipp.port);
            search.add(&uri).unwrap();
            assert!(search
                .printers()
                .iter()
                .any(|printer| printer.device_uri == uri));
            assert_eq!(browse.calls.load(Ordering::SeqCst), 0);
            assert!(cups.hold.load(Ordering::SeqCst));
            browse.release();
            cups.hold.store(false, Ordering::SeqCst);
            search.wait().unwrap();
            assert!(search
                .printers()
                .iter()
                .any(|printer| printer.device_uri == uri));
        }

        #[test]
        fn usb_is_not_passed_to_the_ipp_tool() {
            let ipp = Ipp::open();
            let cups = Cups::open(&[Listed {
                name: "zebra",
                uri: "usb://Zebra/ZTC?serial=ABC",
                make_model: "Zebra Technologies ZTC GX430t",
            }]);
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            assert_eq!(search.printers().len(), 1);
            assert!(!search.printers()[0].connected);
            assert!(ipp.ops.lock().unwrap().is_empty());
        }

        #[test]
        fn a_failed_listing_keeps_the_current_list() {
            let mut cups = Cups::open(&[Listed {
                name: "zebra",
                uri: "usb://Zebra/ZTC?serial=ABC",
                make_model: "Zebra Technologies ZTC GX430t",
            }]);
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            assert_eq!(search.printers().len(), 1);
            cups.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = cups.handle.take() {
                let _ = handle.join();
            }
            search.refresh();
            let err = search.wait().unwrap_err();
            assert!(err.to_string().contains("connecting"));
            let printers = search.printers();
            assert_eq!(printers.len(), 1);
            assert_eq!(printers[0].name, "zebra");
        }

        fn names_of(printers: &[super::super::Printer]) -> Vec<String> {
            let mut names: Vec<_> = printers
                .iter()
                .map(|printer| printer.name.clone())
                .collect();
            names.sort();
            names
        }

        #[test]
        fn queues_follow_the_current_devices() {
            let cups = Cups::open(&[
                Listed {
                    name: "zebra",
                    uri: "usb://Zebra/ZTC?serial=ABC",
                    make_model: "Zebra Technologies ZTC GX430t",
                },
                Listed {
                    name: "hp",
                    uri: "socket://192.0.2.20:9100",
                    make_model: "HP LaserJet",
                },
            ]);
            let browse = Arc::new(FixedBrowse {
                services: Vec::new(),
                calls: AtomicUsize::new(0),
                held: Mutex::new(false),
                cv: Condvar::new(),
            });
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            assert_eq!(names_of(&search.printers()), names_of(&search.devices()));
            assert_eq!(
                names_of(&search.printers()),
                vec!["hp".to_string(), "zebra".to_string()]
            );
            *browse.held.lock().unwrap() = true;
            cups.hold.store(true, Ordering::SeqCst);
            *cups.listed.lock().unwrap() = vec![Listed {
                name: "tsc",
                uri: "usb://TSC/TTP",
                make_model: "TSC",
            }];
            let events = search.subscribe();
            search.refresh();
            cups.hold.store(false, Ordering::SeqCst);
            let ready = std::time::Instant::now();
            while !search
                .printers()
                .iter()
                .any(|printer| printer.name == "tsc")
            {
                assert!(ready.elapsed() < Duration::from_secs(2), "tsc queue");
                thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(names_of(&search.printers()), names_of(&search.devices()));
            assert_eq!(
                names_of(&search.printers()),
                vec!["hp".to_string(), "tsc".to_string(), "zebra".to_string()]
            );
            browse.release();
            search.wait().unwrap();
            assert_eq!(names_of(&search.printers()), names_of(&search.devices()));
            assert_eq!(names_of(&search.printers()), vec!["tsc".to_string()]);
            let mut added = Vec::new();
            let mut removed = Vec::new();
            while let Ok(change) = events.try_recv() {
                match change {
                    Change::Added(printer) => added.push(printer.name),
                    Change::Removed(printer) => removed.push(printer.name),
                    Change::Updated(_) => {}
                }
            }
            added.sort();
            removed.sort();
            assert_eq!(added, vec!["tsc".to_string()]);
            assert_eq!(removed, vec!["hp".to_string(), "zebra".to_string()]);
        }
    }

    mod devices {
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        use super::super::{Change, Search};
        use super::queues::{no_browse, Cups, FoundDevice, Listed};

        fn zebra() -> Listed {
            Listed {
                name: "zebra",
                uri: "usb://Zebra/ZTC?serial=ABC",
                make_model: "Zebra Technologies ZTC GX430t",
            }
        }

        fn wait_until(mut ready: impl FnMut() -> bool) {
            let started = std::time::Instant::now();
            while !ready() {
                assert!(started.elapsed() < Duration::from_secs(2), "timed out");
                thread::sleep(Duration::from_millis(20));
            }
        }

        #[test]
        fn a_live_device_is_connected() {
            let cups = Cups::open(&[
                zebra(),
                Listed {
                    name: "tsc",
                    uri: "usb://TSC/TTP",
                    make_model: "TSC",
                },
            ]);
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, browse).unwrap();
            assert!(search.printers().iter().all(|printer| !printer.connected));
            let events = search.subscribe();
            cups.device_hold
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let seen = cups
                .ops
                .lock()
                .unwrap()
                .iter()
                .filter(|op| **op == 0x400b)
                .count();
            cups.devices.lock().unwrap().push(FoundDevice {
                uri: zebra().uri.to_string(),
                make_model: zebra().make_model.to_string(),
            });
            search.refresh();
            wait_until(|| {
                cups.ops
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|op| **op == 0x400b)
                    .count()
                    > seen
            });
            assert!(search.printers().iter().all(|printer| !printer.connected));
            cups.device_hold
                .store(false, std::sync::atomic::Ordering::SeqCst);
            let started = std::time::Instant::now();
            loop {
                let change = events.recv_timeout(Duration::from_secs(2)).unwrap();
                if let Change::Updated(printer) = change {
                    if printer.name == "zebra" && printer.connected {
                        break;
                    }
                }
                assert!(started.elapsed() < Duration::from_secs(2));
            }
            let printers = search.printers();
            assert_eq!(printers.len(), 2);
            assert!(printers
                .iter()
                .any(|printer| printer.name == "zebra" && printer.connected));
            assert!(printers
                .iter()
                .any(|printer| printer.name == "tsc" && !printer.connected));
            search.wait().unwrap();
            let printers = search.printers();
            assert_eq!(printers.len(), 2);
            assert!(printers
                .iter()
                .any(|printer| printer.name == "zebra" && printer.connected));
            assert!(printers
                .iter()
                .any(|printer| printer.name == "tsc" && !printer.connected));
            assert_eq!(printers.len(), search.devices().len());
        }

        #[test]
        fn a_missing_device_keeps_the_queue() {
            let cups = Cups::open(&[zebra()]);
            let browse = Arc::new(no_browse());
            cups.devices.lock().unwrap().push(FoundDevice {
                uri: zebra().uri.to_string(),
                make_model: zebra().make_model.to_string(),
            });
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            assert!(search.printers()[0].connected);
            cups.devices.lock().unwrap().clear();
            search.refresh();
            search.wait().unwrap();
            let printers = search.printers();
            assert_eq!(printers.len(), 1);
            assert_eq!(printers[0].name, "zebra");
            assert!(!printers[0].connected);
            assert_eq!(search.devices().len(), 1);
        }

        #[test]
        fn a_new_device_is_installed_once() {
            let cups = Cups::open(&[]);
            let uri = "usb://Zebra Technologies/ZTC?serial=ABC&extra";
            cups.devices.lock().unwrap().push(FoundDevice {
                uri: uri.to_string(),
                make_model: "Zebra Technologies ZTC".to_string(),
            });
            cups.ppds
                .lock()
                .unwrap()
                .push(("zebra.ppd".to_string(), "Zebra".to_string()));
            cups.device_hold
                .store(true, std::sync::atomic::Ordering::SeqCst);
            cups.install_hold
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let browse = Arc::new(no_browse());
            let search = Search::start(&cups.base, browse);
            let events = search.subscribe();
            cups.device_hold
                .store(false, std::sync::atomic::Ordering::SeqCst);
            match events.recv_timeout(Duration::from_secs(2)).unwrap() {
                Change::Added(printer) => {
                    assert_eq!(printer.name, "Zebra_Technologies_ZTC_ABC");
                    assert!(!printer.installed);
                }
                other => panic!("expected the device to be added, got {other:?}"),
            }
            wait_until(|| cups.ops.lock().unwrap().contains(&0x400c));
            assert!(search
                .devices()
                .iter()
                .any(|printer| printer.name == "Zebra_Technologies_ZTC_ABC" && !printer.installed));
            assert!(!cups.ops.lock().unwrap().contains(&0x4003));
            cups.install_hold
                .store(false, std::sync::atomic::Ordering::SeqCst);
            search.wait().unwrap();
            let printers = search.printers();
            assert_eq!(printers.len(), 1);
            assert!(printers[0].installed);
            assert_eq!(printers[0].device_uri, uri);
            assert_eq!(
                cups.ops
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|op| **op == 0x4003)
                    .count(),
                1
            );
            let bodies = cups.bodies.lock().unwrap().clone();
            let install = bodies
                .iter()
                .find(|body| body.len() >= 4 && u16::from_be_bytes([body[2], body[3]]) == 0x4003)
                .unwrap();
            let text = String::from_utf8_lossy(install);
            assert!(text.contains("Zebra_Technologies_ZTC_ABC"));
            assert!(text.contains("zebra.ppd"));
            search.refresh();
            search.wait().unwrap();
            assert_eq!(
                cups.ops
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|op| **op == 0x4003)
                    .count(),
                1
            );
        }

        #[test]
        fn no_ppd_is_not_installed() {
            let cups = Cups::open(&[]);
            cups.devices.lock().unwrap().push(FoundDevice {
                uri: "usb://Acme/Widget?serial=1".to_string(),
                make_model: "Acme Widget".to_string(),
            });
            cups.ppds
                .lock()
                .unwrap()
                .push(("hp.ppd".to_string(), "HP LaserJet".to_string()));
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, browse).unwrap();
            let devices = search.devices();
            assert_eq!(devices.len(), 1);
            assert!(!devices[0].installed);
            assert!(!cups.ops.lock().unwrap().contains(&0x4003));
            assert_eq!(search.printers().len(), 1);
        }

        #[test]
        fn a_stopped_queue_is_resumed_once() {
            let cups = Cups::open(&[zebra()]);
            cups.states
                .lock()
                .unwrap()
                .insert("zebra".to_string(), (5, false));
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, Arc::clone(&browse)).unwrap();
            let ops = cups.ops.lock().unwrap().clone();
            assert_eq!(ops.iter().filter(|op| **op == 0x0011).count(), 1);
            assert_eq!(ops.iter().filter(|op| **op == 0x4008).count(), 1);
            assert!(!ops.contains(&0x0013));
            assert!(!ops.contains(&0x0002));
            for body in cups.bodies.lock().unwrap().iter() {
                let text = String::from_utf8_lossy(body);
                assert!(!text.contains("zeMediaTracking"));
                assert!(!text.contains("Darkness"));
                assert!(!text.contains("InkType"));
                assert!(!text.contains("zePrintMode"));
            }
            search.refresh();
            search.wait().unwrap();
            let ops = cups.ops.lock().unwrap().clone();
            assert_eq!(ops.iter().filter(|op| **op == 0x0011).count(), 1);
            assert_eq!(ops.iter().filter(|op| **op == 0x4008).count(), 1);
        }
    }

    mod files {
        use std::path::PathBuf;
        use std::sync::Arc;

        use super::super::{Routing, Search};
        use super::queues::{no_browse, Cups, Ipp, Listed};

        fn home(label: &str) -> PathBuf {
            let path = std::env::temp_dir().join(format!(
                "rust-reg-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn zebra() -> Listed {
            Listed {
                name: "zebra",
                uri: "usb://Zebra/ZTC?serial=ABC",
                make_model: "Zebra Technologies ZTC GX430t",
            }
        }

        #[test]
        fn missing_settings_are_written() {
            let dir = home("settings");
            let cups = Cups::open_held(&[zebra()], true);
            let browse = Arc::new(no_browse());
            let search = Search::start(&cups.base, browse);
            search.set_files(&dir, "event-1");
            cups.hold.store(false, std::sync::atomic::Ordering::SeqCst);
            search.wait().unwrap();
            let path = dir.join("printer_conf").join("zebra.yml");
            let text = std::fs::read_to_string(&path).unwrap();
            let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
            assert_eq!(value["class"].as_str(), Some("native"));
            assert_eq!(value["Darkness"].as_i64(), Some(0));
            assert_eq!(value["Speed"].as_i64(), Some(10));
            let printer = search
                .printers()
                .into_iter()
                .find(|printer| printer.name == "zebra")
                .unwrap();
            assert_eq!(printer.class, "native");
            assert_eq!(printer.values.options["Darkness"], "0");
            assert_eq!(printer.values.options["Speed"], "10");
            let ops = cups.ops.lock().unwrap().clone();
            assert!(!ops.contains(&0x0002));
            assert!(!ops.contains(&0x0013));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn an_ipp_printer_is_class_ipp() {
            let dir = home("ipp");
            let cups = Cups::open(&[]);
            let ipp = Ipp::open();
            let browse = Arc::new(no_browse());
            let search = Search::open(&cups.base, browse).unwrap();
            search.set_files(&dir, "event-1");
            let uri = format!("ipps://127.0.0.1:{}/ipp/print", ipp.port);
            search.add(&uri).unwrap();
            let printer = search
                .printers()
                .into_iter()
                .find(|printer| printer.class == "ipp")
                .unwrap();
            assert!(printer.device_uri.starts_with("ipps://"));
            for path in [
                dir.join("printer_conf")
                    .join(format!("{}.yml", printer.name)),
                dir.join("event-1").join(format!("pr_{}.yml", printer.name)),
            ] {
                let text = std::fs::read_to_string(&path).unwrap();
                let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
                assert_eq!(value["class"].as_str(), Some("ipp"), "{}", path.display());
            }
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn routing_roundtrip_without_a_second_name() {
            let dir = home("routing");
            let cups = Cups::open_held(&[zebra()], true);
            let browse = Arc::new(no_browse());
            let search = Search::start(&cups.base, browse);
            search.set_files(&dir, "event-9");
            cups.hold.store(false, std::sync::atomic::Ordering::SeqCst);
            search.wait().unwrap();
            let name = "zebra".to_string();
            search
                .save_routing(
                    &name,
                    Routing {
                        second_name: String::new(),
                        print_cert: true,
                        auto_print: true,
                        ..Routing::default()
                    },
                )
                .unwrap();
            let path = dir.join("event-9").join("pr_zebra.yml");
            let text = std::fs::read_to_string(&path).unwrap();
            let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
            assert!(value.get("second_name").is_some());
            assert_eq!(value["second_name"].as_str(), Some(""));
            assert_eq!(value["print_cert"].as_bool(), Some(true));
            assert_eq!(value["auto_print"].as_bool(), Some(true));
            assert_eq!(value["class"].as_str(), Some("native"));
            search.refresh();
            search.wait().unwrap();
            let printer = search
                .printers()
                .into_iter()
                .find(|printer| printer.name == "zebra")
                .unwrap();
            assert_eq!(printer.routing.second_name, "");
            assert!(printer.routing.print_cert);
            assert!(printer.routing.auto_print);
            assert_eq!(printer.class, "native");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn missing_routing_is_used() {
            let dir = home("missing-routing");
            let cups = Cups::open_held(&[zebra()], true);
            let browse = Arc::new(no_browse());
            let search = Search::start(&cups.base, browse);
            search.set_files(&dir, "event-3");
            cups.hold.store(false, std::sync::atomic::Ordering::SeqCst);
            search.wait().unwrap();
            let printer = search
                .printers()
                .into_iter()
                .find(|printer| printer.name == "zebra")
                .unwrap();
            assert!(printer.routing.used);
            assert!(printer.routing.zone.is_empty());
            assert!(printer.routing.category.is_empty());
            assert!(printer.routing.is_internet.is_none());
            assert!(printer.routing.second_name.is_empty());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    mod pick {
        use std::path::PathBuf;
        use std::sync::Arc;

        use super::super::{Job, Routing, Search};
        use super::queues::{no_browse, Cups, FoundDevice, Listed};

        fn queue(name: &'static str, serial: &'static str) -> Listed {
            Listed {
                name,
                uri: match serial {
                    "1" => "usb://Desk/A?serial=1",
                    "2" => "usb://Desk/B?serial=2",
                    "3" => "usb://Desk/C?serial=3",
                    _ => "usb://Desk/D?serial=4",
                },
                make_model: "HP LaserJet",
            }
        }

        fn home(label: &str) -> PathBuf {
            let path = std::env::temp_dir().join(format!(
                "rust-reg-pick-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn started(queues: &[Listed]) -> (Cups, Search, PathBuf) {
            let cups = Cups::open(queues);
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            *cups.devices.lock().unwrap() = queues
                .iter()
                .map(|printer| FoundDevice {
                    uri: printer.uri.to_string(),
                    make_model: printer.make_model.to_string(),
                })
                .collect();
            search.refresh();
            search.wait().unwrap();
            let dir = home("list");
            search.set_files(&dir, "event");
            (cups, search, dir)
        }

        fn route(search: &Search, name: &str, routing: Routing) {
            search.save_routing(name, routing).unwrap();
        }

        #[test]
        fn explicit_name_wins() {
            let queues = [queue("fresh", "1"), queue("stale", "2")];
            let (_cups, search, dir) = started(&queues);
            route(&search, "fresh", Routing::default());
            route(&search, "stale", Routing::default());
            search.set_last_print("fresh", 100);
            search.set_last_print("stale", 1);
            let chosen = search
                .pick(&Job {
                    name: Some("fresh".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(chosen.name, "fresh");
            assert_eq!(chosen.last_print, 100);
            assert_eq!(
                search
                    .printers()
                    .into_iter()
                    .find(|printer| printer.name == "fresh")
                    .unwrap()
                    .last_print,
                100
            );
            assert_eq!(
                search
                    .printers()
                    .into_iter()
                    .find(|printer| printer.name == "stale")
                    .unwrap()
                    .last_print,
                1
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn category_goes_to_its_printer() {
            let queues = [
                queue("bound", "1"),
                queue("open", "2"),
                queue("web", "3"),
                queue("local", "4"),
            ];
            let (_cups, search, dir) = started(&queues);
            route(
                &search,
                "bound",
                Routing {
                    category: "7".to_string(),
                    ..Routing::default()
                },
            );
            route(&search, "open", Routing::default());
            route(
                &search,
                "web",
                Routing {
                    is_internet: Some(true),
                    ..Routing::default()
                },
            );
            route(
                &search,
                "local",
                Routing {
                    is_internet: Some(false),
                    ..Routing::default()
                },
            );
            let category = search
                .pick(&Job {
                    category: Some("7".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(category.name, "bound");
            let internet = search
                .pick(&Job {
                    internet: true,
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(internet.name, "web");
            let local = search
                .pick(&Job {
                    internet: false,
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(local.name, "local");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_certificate_goes_to_the_certificate_printer() {
            let queues = [queue("certs", "1"), queue("badges", "2")];
            let (_cups, search, dir) = started(&queues);
            route(
                &search,
                "certs",
                Routing {
                    print_cert: true,
                    ..Routing::default()
                },
            );
            route(
                &search,
                "badges",
                Routing {
                    category: "7".to_string(),
                    ..Routing::default()
                },
            );
            let chosen = search
                .pick(&Job {
                    certificate: true,
                    category: Some("7".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(chosen.name, "certs");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_zone_round_robins() {
            let queues = [queue("early", "1"), queue("late", "2"), queue("other", "3")];
            let (_cups, search, dir) = started(&queues);
            route(
                &search,
                "early",
                Routing {
                    zone: "2".to_string(),
                    ..Routing::default()
                },
            );
            route(
                &search,
                "late",
                Routing {
                    zone: "2".to_string(),
                    ..Routing::default()
                },
            );
            route(
                &search,
                "other",
                Routing {
                    zone: "1".to_string(),
                    ..Routing::default()
                },
            );
            search.set_last_print("early", 10);
            search.set_last_print("late", 80);
            search.set_last_print("other", 1);
            let chosen = search
                .pick(&Job {
                    zone: Some("2".to_string()),
                    rule: Some("round_robin".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(chosen.name, "early");
            assert_eq!(chosen.last_print, 10);
            assert_eq!(
                search
                    .printers()
                    .into_iter()
                    .find(|printer| printer.name == "early")
                    .unwrap()
                    .last_print,
                10
            );
            assert_ne!(chosen.name, "other");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_zone_picks_the_shorter_queue() {
            let queues = [queue("busy", "1"), queue("idle", "2")];
            let (cups, search, dir) = started(&queues);
            for name in ["busy", "idle"] {
                route(
                    &search,
                    name,
                    Routing {
                        zone: "2".to_string(),
                        ..Routing::default()
                    },
                );
            }
            cups.jobs.lock().unwrap().insert("idle".to_string(), 1);
            cups.jobs.lock().unwrap().insert("busy".to_string(), 5);
            search.set_last_print("idle", 90);
            search.set_last_print("busy", 10);
            let shorter = search
                .pick(&Job {
                    zone: Some("2".to_string()),
                    rule: Some("fewest_jobs".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(shorter.name, "idle");
            let asked = cups.bodies.lock().unwrap().iter().any(|body| {
                let text = String::from_utf8_lossy(body);
                text.contains("requested-attributes") && text.contains("queued-job-count")
            });
            assert!(asked);
            cups.jobs.lock().unwrap().insert("idle".to_string(), 4);
            cups.jobs.lock().unwrap().insert("busy".to_string(), 4);
            search.set_last_print("idle", 90);
            search.set_last_print("busy", 10);
            let older = search
                .pick(&Job {
                    zone: Some("2".to_string()),
                    rule: Some("fewest_jobs".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(older.name, "busy");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_missing_job_count_uses_last_print() {
            let queues = [queue("busy", "1"), queue("idle", "2")];
            let (cups, search, dir) = started(&queues);
            for name in ["busy", "idle"] {
                route(
                    &search,
                    name,
                    Routing {
                        zone: "2".to_string(),
                        ..Routing::default()
                    },
                );
            }
            cups.jobs.lock().unwrap().insert("idle".to_string(), -1);
            cups.jobs.lock().unwrap().insert("busy".to_string(), -1);
            search.set_last_print("idle", 90);
            search.set_last_print("busy", 10);
            let older = search
                .pick(&Job {
                    zone: Some("2".to_string()),
                    rule: Some("fewest_jobs".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(older.name, "busy");
            cups.jobs.lock().unwrap().insert("idle".to_string(), 0);
            search.set_last_print("idle", 90);
            search.set_last_print("busy", 10);
            let still_older = search
                .pick(&Job {
                    zone: Some("2".to_string()),
                    rule: Some("fewest_jobs".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(still_older.name, "busy");
            cups.jobs.lock().unwrap().insert("busy".to_string(), 3);
            search.set_last_print("idle", 90);
            search.set_last_print("busy", 10);
            let empty = search
                .pick(&Job {
                    zone: Some("2".to_string()),
                    rule: Some("fewest_jobs".to_string()),
                    ..Job::default()
                })
                .unwrap();
            assert_eq!(empty.name, "idle");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn nothing_matching_is_an_error() {
            let cups = Cups::open(&[queue("desk", "1")]);
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            search.set_last_print("desk", 4);
            let err = search.pick(&Job::default()).unwrap_err();
            assert_eq!(err.to_string(), "no queue");
            assert!(!err.to_string().contains("desk"));
            assert_eq!(search.printers()[0].last_print, 4);

            let queues = [queue("desk", "1")];
            let (_cups, search, dir) = started(&queues);
            route(
                &search,
                "desk",
                Routing {
                    used: false,
                    ..Routing::default()
                },
            );
            search.set_last_print("desk", 4);
            let err = search
                .pick(&Job {
                    category: Some("7".to_string()),
                    ..Job::default()
                })
                .unwrap_err();
            assert_eq!(err.to_string(), "no queue");
            assert!(!err.to_string().contains("desk"));
            assert_eq!(search.printers()[0].last_print, 4);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    mod submit {
        use std::path::PathBuf;
        use std::sync::atomic::Ordering;
        use std::sync::Arc;

        use super::super::{Page, Routing, Search};
        use super::queues::{no_browse, Cups, Ipp, Listed};

        const PNG: &[u8] = b"\x89PNG\r\n\x1a\nbadge";

        fn home(label: &str) -> PathBuf {
            let path = std::env::temp_dir().join(format!(
                "rust-reg-submit-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn queue(name: &'static str, uri: &'static str, make_model: &'static str) -> Listed {
            Listed {
                name,
                uri,
                make_model,
            }
        }

        fn ready(printers: &[Listed]) -> (Cups, Search, PathBuf) {
            let dir = home("list");
            let cups = Cups::open_held(printers, true);
            let search = Search::start(&cups.base, Arc::new(no_browse()));
            search.set_files(&dir, "event");
            cups.hold.store(false, Ordering::SeqCst);
            search.wait().unwrap();
            (cups, search, dir)
        }

        fn named(search: &Search, name: &str) -> super::super::Printer {
            search
                .printers()
                .into_iter()
                .find(|printer| printer.name == name)
                .unwrap()
        }

        fn print_jobs(cups: &Cups) -> Vec<Vec<u8>> {
            let ops = cups.ops.lock().unwrap().clone();
            let bodies = cups.bodies.lock().unwrap().clone();
            ops.into_iter()
                .zip(bodies)
                .filter(|(op, _)| *op == 0x0002)
                .map(|(_, body)| body)
                .collect()
        }

        #[test]
        fn zpl_job_is_raw() {
            let printers = [queue(
                "zebra",
                "usb://Zebra/ZTC?serial=ABC",
                "Zebra Technologies ZTC GX430t",
            )];
            let (cups, search, dir) = ready(&printers);
            let printer = named(&search, "zebra");
            let id = search
                .submit(
                    &printer,
                    &Page::Graphic {
                        bytes: b"~DGBADGE".to_vec(),
                        width_px: 100,
                    },
                )
                .unwrap();
            assert_eq!(id, 1);
            let body = &print_jobs(&cups)[0];
            let text = String::from_utf8_lossy(body);
            assert!(text.contains("application/octet-stream"));
            assert!(text.contains("zeMediaTracking"));
            assert!(text.contains("Web"));
            let zpl = &body[body.windows(3).position(|window| window == b"^XA").unwrap()..];
            let zpl = std::str::from_utf8(zpl).unwrap();
            assert!(zpl.starts_with("^XA"), "{zpl}");
            assert!(zpl.contains("^PR10"), "{zpl}");
            assert!(zpl.contains("^MD0"), "{zpl}");
            assert!(zpl.contains("~DGBADGE"), "{zpl}");
            assert!(zpl.contains("^FO565,0"), "{zpl}");
            assert!(zpl.trim_end().ends_with("^XZ"), "{zpl}");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn raster_job_is_png() {
            let printers = [queue(
                "evolis",
                "usb://Evolis/Zenius?serial=E1",
                "Evolis Zenius",
            )];
            let (cups, search, dir) = ready(&printers);
            search
                .save_routing(
                    "evolis",
                    Routing {
                        no_postscript: true,
                        ..Routing::default()
                    },
                )
                .unwrap();
            let printer = named(&search, "evolis");
            assert!(printer.routing.no_postscript);
            let id = search
                .submit(
                    &printer,
                    &Page::Png {
                        bytes: PNG.to_vec(),
                        width_in: 2.125,
                        height_in: 3.375,
                    },
                )
                .unwrap();
            assert_eq!(id, 1);
            let body = &print_jobs(&cups)[0];
            let text = String::from_utf8_lossy(body);
            assert!(text.contains("image/png"));
            assert!(text.contains("InkType"));
            assert!(text.contains("Black"));
            assert!(text.contains("2.125x3.375in"));
            assert!(body.windows(PNG.len()).any(|window| window == PNG));
            let ops = cups.ops.lock().unwrap().clone();
            assert!(!ops.contains(&0x4003));
            assert!(!ops.contains(&0x0013));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_pdf_page_is_pdf() {
            let printers = [queue("hp", "socket://192.0.2.20:9100", "HP LaserJet")];
            let (cups, search, dir) = ready(&printers);
            let printer = named(&search, "hp");
            let pdf = b"%PDF-1.4 badge";
            let id = search
                .submit(
                    &printer,
                    &Page::Pdf {
                        bytes: pdf.to_vec(),
                    },
                )
                .unwrap();
            assert_eq!(id, 1);
            let body = &print_jobs(&cups)[0];
            let text = String::from_utf8_lossy(body);
            assert!(text.contains("application/pdf"));
            assert!(body.windows(pdf.len()).any(|window| window == pdf));
            assert!(!text.contains("application/postscript"));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn an_ipp_printer_uses_the_ipp_tool() {
            let dir = home("ipp");
            let cups = Cups::open(&[]);
            let ipp = Ipp::open();
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            search.set_files(&dir, "event");
            let uri = format!("ipps://127.0.0.1:{}/ipp/print", ipp.port);
            search.add(&uri).unwrap();
            let printer = search
                .printers()
                .into_iter()
                .find(|printer| printer.class == "ipp")
                .unwrap();
            let id = search
                .submit(
                    &printer,
                    &Page::Png {
                        bytes: PNG.to_vec(),
                        width_in: 4.0,
                        height_in: 6.0,
                    },
                )
                .unwrap();
            assert_eq!(id, 1);
            assert!(!cups.ops.lock().unwrap().contains(&0x0002));
            let bodies = ipp.bodies.lock().unwrap().clone();
            let sent = bodies
                .iter()
                .find(|body| body.len() >= 4 && u16::from_be_bytes([body[2], body[3]]) == 0x0002)
                .expect("Print-Job");
            let text = String::from_utf8_lossy(sent);
            assert!(text.contains("image/png"));
            assert!(sent.windows(PNG.len()).any(|window| window == PNG));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_stopped_ipp_printer_is_resumed_before_the_job() {
            let dir = home("ipp-resume");
            let cups = Cups::open(&[]);
            let ipp = Ipp::open();
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            search.set_files(&dir, "event");
            let uri = format!("ipps://127.0.0.1:{}/ipp/print", ipp.port);
            search.add(&uri).unwrap();
            let mut printer = search
                .printers()
                .into_iter()
                .find(|printer| printer.class == "ipp")
                .unwrap();
            printer.state = "stopped".to_string();
            let before = ipp.ops.lock().unwrap().len();
            search
                .submit(
                    &printer,
                    &Page::Png {
                        bytes: PNG.to_vec(),
                        width_in: 4.0,
                        height_in: 6.0,
                    },
                )
                .unwrap();
            let ops = ipp.ops.lock().unwrap()[before..].to_vec();
            let resume = ops.iter().position(|op| *op == 0x0011).unwrap();
            let print = ops.iter().position(|op| *op == 0x0002).unwrap();
            assert!(resume < print);
            assert!(!ops.contains(&0x4008));
            assert!(!cups.ops.lock().unwrap().contains(&0x0011));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_stopped_ipp_job_is_resumed_before_the_next_copy() {
            let dir = home("ipp-stopped-job");
            let cups = Cups::open(&[]);
            let ipp = Ipp::open();
            *ipp.job_script.lock().unwrap() = vec![6, 6, 9];
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            search.set_files(&dir, "event");
            let uri = format!("ipps://127.0.0.1:{}/ipp/print", ipp.port);
            search.add(&uri).unwrap();
            let printer = search
                .printers()
                .into_iter()
                .find(|printer| printer.class == "ipp")
                .unwrap();
            assert_ne!(printer.state, "stopped");
            let before = ipp.ops.lock().unwrap().len();
            search
                .submit(
                    &printer,
                    &Page::Png {
                        bytes: PNG.to_vec(),
                        width_in: 4.0,
                        height_in: 6.0,
                    },
                )
                .unwrap();
            let ops = ipp.ops.lock().unwrap()[before..].to_vec();
            let prints: Vec<usize> = ops
                .iter()
                .enumerate()
                .filter(|(_, op)| **op == 0x0002)
                .map(|(index, _)| index)
                .collect();
            assert_eq!(prints.len(), 2);
            let resume = ops.iter().position(|op| *op == 0x0011).unwrap();
            assert!(prints[0] < resume && resume < prints[1]);
            assert!(!ops.contains(&0x4008));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    mod retry {
        use std::sync::Arc;

        use super::super::{Page, Search};
        use super::queues::{no_browse, Cups, Listed};

        fn desk() -> Listed {
            Listed {
                name: "desk",
                uri: "socket://192.0.2.20:9100",
                make_model: "HP LaserJet",
            }
        }

        fn opened() -> (Cups, Search, super::super::Printer) {
            let cups = Cups::open(&[desk()]);
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            let printer = search
                .printers()
                .into_iter()
                .find(|printer| printer.name == "desk")
                .unwrap();
            (cups, search, printer)
        }

        fn page() -> Page {
            Page::Png {
                bytes: b"\x89PNG\r\n\x1a\nbadge".to_vec(),
                width_in: 4.0,
                height_in: 6.0,
            }
        }

        fn count(ops: &[u16], op: u16) -> usize {
            ops.iter().filter(|item| **item == op).count()
        }

        #[test]
        fn a_processing_job_is_not_cancelled() {
            let (cups, search, printer) = opened();
            *cups.job_script.lock().unwrap() = vec![5, 9];
            let id = search.submit(&printer, &page()).unwrap();
            assert_eq!(id, 1);
            let ops = cups.ops.lock().unwrap().clone();
            assert_eq!(count(&ops, 0x0002), 1);
            assert_eq!(count(&ops, 0x0009), 2);
            assert!(!ops.contains(&0x0008));
        }

        #[test]
        fn an_aborted_job_is_retried() {
            let (cups, search, printer) = opened();
            *cups.job_script.lock().unwrap() = vec![8, 9];
            let id = search.submit(&printer, &page()).unwrap();
            assert_eq!(id, 2);
            let ops = cups.ops.lock().unwrap().clone();
            assert_eq!(count(&ops, 0x0002), 2);
            assert!(ops.contains(&0x0008));
            let cancel = ops.iter().position(|op| *op == 0x0008).unwrap();
            let prints: Vec<usize> = ops
                .iter()
                .enumerate()
                .filter(|(_, op)| **op == 0x0002)
                .map(|(index, _)| index)
                .collect();
            assert!(prints[0] < cancel && cancel < prints[1]);
        }

        #[test]
        fn a_recovered_job_is_not_sent_again() {
            let (cups, search, printer) = opened();
            *cups.job_script.lock().unwrap() = vec![8];
            *cups.job_follow.lock().unwrap() = vec![9];
            let id = search.submit(&printer, &page()).unwrap();
            assert_eq!(id, 1);
            let ops = cups.ops.lock().unwrap().clone();
            assert_eq!(count(&ops, 0x0002), 1);
            assert!(!ops.contains(&0x0008));
            assert!(count(&ops, 0x0009) >= 2);
        }

        #[test]
        fn a_live_job_is_not_sent_again() {
            let (cups, search, printer) = opened();
            *cups.job_script.lock().unwrap() = vec![8];
            *cups.job_follow.lock().unwrap() = vec![5];
            let err = search.submit(&printer, &page()).unwrap_err();
            assert_eq!(err.to_string(), "job still processing");
            let ops = cups.ops.lock().unwrap().clone();
            assert_eq!(count(&ops, 0x0002), 1);
            assert!(!ops.contains(&0x0008));
        }

        #[test]
        fn ten_failures_stop() {
            let (cups, search, printer) = opened();
            *cups.job_script.lock().unwrap() = vec![8; 10];
            let err = search.submit(&printer, &page()).unwrap_err();
            assert_eq!(err.to_string(), "job aborted");
            let ops = cups.ops.lock().unwrap().clone();
            assert_eq!(count(&ops, 0x0002), 10);
            assert_eq!(count(&ops, 0x0008), 10);
        }

        #[test]
        fn a_stopped_queue_is_resumed() {
            let (cups, search, mut printer) = opened();
            printer.state = "stopped".to_string();
            let id = search.submit(&printer, &page()).unwrap();
            assert_eq!(id, 1);
            let ops = cups.ops.lock().unwrap().clone();
            let resume = ops.iter().position(|op| *op == 0x0011).unwrap();
            let print = ops.iter().position(|op| *op == 0x0002).unwrap();
            assert!(resume < print);
            assert_eq!(count(&ops, 0x0002), 1);
        }
    }

    mod worker {
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        use image::RgbaImage;
        use ticket_render::Layout;

        use super::super::print_job::capped_dpi;
        use super::super::{PrintRequest, Routing, Search};
        use super::queues::{no_browse, Cups, FoundDevice, Listed};
        use crate::registration_server::gate::Gate;
        use crate::registration_server::print::{prepare_page, Badges, PreparedLayout};

        fn home(label: &str) -> std::path::PathBuf {
            let path = std::env::temp_dir().join(format!(
                "rust-reg-worker-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn queue(name: &'static str, uri: &'static str, make_model: &'static str) -> Listed {
            Listed {
                name,
                uri,
                make_model,
            }
        }

        fn started(queues: &[Listed]) -> (Cups, Search, std::path::PathBuf) {
            let cups = Cups::open(queues);
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            *cups.devices.lock().unwrap() = queues
                .iter()
                .map(|printer| FoundDevice {
                    uri: printer.uri.to_string(),
                    make_model: printer.make_model.to_string(),
                })
                .collect();
            search.refresh();
            search.wait().unwrap();
            let dir = home("list");
            search.set_files(&dir, "event");
            (cups, search, dir)
        }

        fn disconnected(queues: &[Listed]) -> (Cups, Search, std::path::PathBuf) {
            let cups = Cups::open(queues);
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            let dir = home("off");
            search.set_files(&dir, "event");
            (cups, search, dir)
        }

        fn visitor(name: &str) -> HashMap<String, String> {
            HashMap::from([
                ("name".to_string(), name.to_string()),
                ("surname".to_string(), "Lee".to_string()),
                ("patronymic".to_string(), "P".to_string()),
                ("company".to_string(), "Acme".to_string()),
            ])
        }

        fn request(category: i64) -> PrintRequest {
            PrintRequest {
                id: "5".to_string(),
                category,
                ..PrintRequest::default()
            }
        }

        fn remember(search: &Search, name: &str) {
            search.set_visitor("5", visitor(name));
        }

        fn page_yaml(width: i64, height: i64) -> String {
            let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf");
            format!(
                r#"
page_size:
  width: {width}
  height: {height}
  units: points
  bleeds: {{top: 0, right: 0, bottom: 0, left: 0}}
resources:
  images: {{}}
  fonts:
    font1:
      name: font1
      embedded: true
      source_path: "{font}"
contents:
  - id: name
    posX: 10
    posY: 10
    width: 100
    height: 20
    alignment:
      horizontal: left
      vertical: top
    padding: {{top: 0, right: 0, bottom: 0, left: 0}}
    template: true
    text:
      font: font1
      font_size: 12
      leading: 0
      line_height: 14
      preentered: "{{{{name}}}}"
      content: null
"#,
                font = font.display()
            )
        }

        fn store_page(badges: &mut Badges, category: i64, width: i64, height: i64) {
            let dir = home(&format!("page-{category}"));
            std::fs::write(dir.join("page.yaml"), page_yaml(width, height)).unwrap();
            badges.replace_page(category, prepare_page(&dir));
            let _ = std::fs::remove_dir_all(&dir);
        }

        fn store_layout(badges: &mut Badges, category: i64, width: i32, height: i32) {
            badges.replace_layout(
                category,
                Some(Ok(PreparedLayout::memory(
                    Layout {
                        width,
                        height,
                        dots_per_point: 1.0,
                        areas: Vec::new(),
                    },
                    RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
                ))),
            );
        }

        fn print_bodies(cups: &Cups) -> Vec<Vec<u8>> {
            let ops = cups.ops.lock().unwrap().clone();
            let bodies = cups.bodies.lock().unwrap().clone();
            ops.into_iter()
                .zip(bodies)
                .filter(|(op, _)| *op == 0x0002)
                .map(|(_, body)| body)
                .collect()
        }

        fn png_size(bytes: &[u8]) -> (u32, u32) {
            let start = bytes
                .windows(8)
                .position(|window| window == b"\x89PNG\r\n\x1a\n")
                .unwrap();
            let png = &bytes[start..];
            (
                u32::from_be_bytes(png[16..20].try_into().unwrap()),
                u32::from_be_bytes(png[20..24].try_into().unwrap()),
            )
        }

        #[tokio::test]
        async fn print_submits_one_job() {
            let (cups, search, dir) =
                started(&[queue("hp", "socket://192.0.2.20:9100", "HP LaserJet")]);
            let mut badges = Badges::default();
            store_page(&mut badges, 7, 720, 72);
            search.set_badges(badges);
            search.set_formats(
                "hp",
                ipp_tool::Formats {
                    supported: vec!["image/png".to_string(), "application/pdf".to_string()],
                    default: None,
                    preferred: None,
                },
            );
            remember(&search, "Ann");
            let mut request = request(7);
            request.pixel_cap = Some(1200);
            let gate = Arc::new(Gate::new());
            let printed = search.print(&request, &gate).await.unwrap();
            assert_eq!(printed.job_id, Some(1));
            assert!(printed.png.is_none());
            let jobs = print_bodies(&cups);
            assert_eq!(jobs.len(), 1);
            let text = String::from_utf8_lossy(&jobs[0]);
            assert!(text.contains("image/png"), "{text}");
            let (width, height) = png_size(&jobs[0]);
            let dpi = capped_dpi(300, 10.0, 1.0, 1200);
            assert_eq!(dpi, 100);
            assert_eq!((width, height), (1000, 100));
            assert_eq!(search.held_pages(), 0);
            assert!(!gate.is_printing());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn an_empty_format_list_holds_the_prepared_page() {
            let (cups, search, dir) =
                started(&[queue("hp", "socket://192.0.2.20:9100", "HP LaserJet")]);
            let mut badges = Badges::default();
            store_page(&mut badges, 7, 144, 72);
            search.set_badges(badges);
            remember(&search, "Ann");
            let now = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1_000));
            let clock = std::sync::Arc::clone(&now);
            search.set_clock(std::sync::Arc::new(move || {
                clock.load(std::sync::atomic::Ordering::SeqCst)
            }));
            search.set_window_ms(500);
            let gate = Arc::new(Gate::new());
            let printed = search.print(&request(7), &gate).await.unwrap();
            assert!(printed.job_id.is_none());
            assert!(print_bodies(&cups).is_empty());
            assert_eq!(search.held_pages(), 1);
            now.store(1_501, std::sync::atomic::Ordering::SeqCst);
            let ids = search.flush_ready(&gate).unwrap();
            assert_eq!(ids, vec![1]);
            let jobs = print_bodies(&cups);
            assert_eq!(jobs.len(), 1);
            assert!(String::from_utf8_lossy(&jobs[0]).contains("application/pdf"));
            assert_eq!(search.held_pages(), 0);
            let _ = std::fs::remove_dir_all(&dir);

            let (cups, search, dir) = started(&[queue(
                "zebra",
                "usb://Zebra/ZTC?serial=ABC",
                "Zebra Technologies ZTC GX430t",
            )]);
            let mut badges = Badges::default();
            store_page(&mut badges, 7, 144, 72);
            search.set_badges(badges);
            remember(&search, "Ann");
            let gate = Arc::new(Gate::new());
            search.print(&request(7), &gate).await.unwrap();
            let jobs = print_bodies(&cups);
            assert_eq!(jobs.len(), 1);
            let text = String::from_utf8_lossy(&jobs[0]);
            assert!(text.contains("^XA"), "{text}");
            assert_eq!(search.held_pages(), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn two_held_pages_share_one_font() {
            let (cups, search, dir) =
                started(&[queue("hp", "socket://192.0.2.20:9100", "HP LaserJet")]);
            let mut badges = Badges::default();
            store_page(&mut badges, 7, 144, 72);
            search.set_badges(badges);
            let now = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(2_000));
            let clock = std::sync::Arc::clone(&now);
            search.set_clock(std::sync::Arc::new(move || {
                clock.load(std::sync::atomic::Ordering::SeqCst)
            }));
            search.set_window_ms(500);
            let gate = Arc::new(Gate::new());
            remember(&search, "Ann");
            search.print(&request(7), &gate).await.unwrap();
            remember(&search, "Bo");
            now.store(2_200, std::sync::atomic::Ordering::SeqCst);
            search.print(&request(7), &gate).await.unwrap();
            assert_eq!(search.held_pages(), 2);
            now.store(2_701, std::sync::atomic::Ordering::SeqCst);
            search.flush_ready(&gate).unwrap();
            let jobs = print_bodies(&cups);
            assert_eq!(jobs.len(), 1);
            let start = jobs[0]
                .windows(5)
                .position(|window| window == b"%PDF-")
                .unwrap();
            let doc = lopdf::Document::load_mem(&jobs[0][start..]).unwrap();
            assert_eq!(doc.get_pages().len(), 2);
            let text = doc.extract_text(&[1, 2]).unwrap();
            assert!(text.contains("Ann"), "{text}");
            assert!(text.contains("Bo"), "{text}");
            let faces = doc
                .objects
                .values()
                .filter(|object| {
                    object
                        .as_dict()
                        .ok()
                        .and_then(|dict| dict.get(b"Subtype").ok())
                        .and_then(|kind| kind.as_name().ok())
                        == Some(b"CIDFontType2")
                })
                .count();
            assert_eq!(faces, 1);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn a_zpl_queue_submits_raw() {
            let (cups, search, dir) = started(&[queue(
                "zebra",
                "usb://Zebra/ZTC?serial=ABC",
                "Zebra Technologies ZTC GX430t",
            )]);
            let mut badges = Badges::default();
            store_layout(&mut badges, 7, 72, 72);
            search.set_badges(badges);
            remember(&search, "Ann");
            let gate = Arc::new(Gate::new());
            search.print(&request(7), &gate).await.unwrap();
            let jobs = print_bodies(&cups);
            assert_eq!(jobs.len(), 1);
            let text = String::from_utf8_lossy(&jobs[0]);
            let zpl = &text[text.find("^XA").unwrap()..];
            assert!(zpl.starts_with("^XA"), "{zpl}");
            assert!(zpl.trim_end().ends_with("^XZ"), "{zpl}");
            assert_eq!(search.held_pages(), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn no_printer_writes_genbadges() {
            let (cups, search, dir) =
                disconnected(&[queue("hp", "socket://192.0.2.20:9100", "HP LaserJet")]);
            let mut badges = Badges::default();
            store_page(&mut badges, 3, 144, 72);
            search.set_badges(badges);
            remember(&search, "Ann");
            let gate = Arc::new(Gate::new());
            let printed = search.print(&request(3), &gate).await.unwrap();
            assert!(printed.job_id.is_none());
            let stem = "3_Ann_Lee_P_Acme";
            let png = dir
                .join("event")
                .join("genbadges-png")
                .join(format!("{stem}.png"));
            let pdf = dir
                .join("event")
                .join("genbadges-pdf")
                .join(format!("{stem}.pdf"));
            assert_eq!(printed.png.as_deref(), Some(png.as_path()));
            assert_eq!(printed.pdf.as_deref(), Some(pdf.as_path()));
            assert_eq!(&std::fs::read(&png).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
            let doc = lopdf::Document::load(&pdf).unwrap();
            assert_eq!(doc.get_pages().len(), 1);
            assert!(doc.extract_text(&[1]).unwrap().contains("Ann"));
            assert!(print_bodies(&cups).is_empty());
            assert!(super::super::Error::new("no queue").is_no_queue());
            assert!(!super::super::Error::new("queue stopped").is_no_queue());

            let mut layout_badges = Badges::default();
            store_layout(&mut layout_badges, 4, 144, 72);
            search.set_badges(layout_badges);
            let mut layout_request = request(4);
            layout_request.transliterate = true;
            search.set_visitor("5", visitor("Анна"));
            let printed = search.print(&layout_request, &gate).await.unwrap();
            assert!(printed.job_id.is_none());
            let stem = "4_Анна_Lee_P_Acme";
            let png = dir
                .join("event")
                .join("genbadges-png")
                .join(format!("{stem}.png"));
            let pdf = dir
                .join("event")
                .join("genbadges-pdf")
                .join(format!("{stem}.pdf"));
            assert_eq!(printed.png.as_deref(), Some(png.as_path()));
            assert_eq!(printed.pdf.as_deref(), Some(pdf.as_path()));
            let image = image::load_from_memory(&std::fs::read(&png).unwrap())
                .unwrap()
                .to_rgba8();
            assert_eq!(image.dimensions(), (1440, 720));
            let doc = lopdf::Document::load(&pdf).unwrap();
            assert_eq!(doc.get_pages().len(), 1);
            let page_id = doc.get_pages()[&1];
            let media = doc
                .get_dictionary(page_id)
                .unwrap()
                .get(b"MediaBox")
                .unwrap();
            let box_width = media.as_array().unwrap()[2].as_float().unwrap();
            let box_height = media.as_array().unwrap()[3].as_float().unwrap();
            assert!((box_width - 144.0).abs() < 0.1, "{box_width}");
            assert!((box_height - 72.0).abs() < 0.1, "{box_height}");
            let tr = dir
                .join("event")
                .join("genbadges-png")
                .join(format!("{stem}-tr.png"));
            let tr_pdf = dir
                .join("event")
                .join("genbadges-pdf")
                .join(format!("{stem}-tr.pdf"));
            assert!(tr.is_file());
            assert!(tr_pdf.is_file());

            let mut again = request(4);
            again.double_print = true;
            search.set_visitor("5", visitor("Ann"));
            search.print(&again, &gate).await.unwrap();
            assert!(dir
                .join("event")
                .join("genbadges-png")
                .join("4_Ann_Lee_P_Acme-d.png")
                .is_file());
            assert!(print_bodies(&cups).is_empty());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn a_second_page_yields() {
            let (cups, search, dir) =
                started(&[queue("hp", "socket://192.0.2.20:9100", "HP LaserJet")]);
            let mut badges = Badges::default();
            store_layout(&mut badges, 7, 72, 72);
            search.set_badges(badges);
            remember(&search, "Ann");
            let gate = Arc::new(Gate::new());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let flag = Arc::clone(&gate);
            let record = Arc::clone(&seen);
            *cups.on_job.lock().unwrap() = Some(Arc::new(move || {
                record.lock().unwrap().push(flag.is_printing());
            }));
            let mut request = request(7);
            request.double_print = true;
            search.print(&request, &gate).await.unwrap();
            assert_eq!(print_bodies(&cups).len(), 2);
            assert_eq!(seen.lock().unwrap().as_slice(), &[true, true]);
            assert!(!gate.is_printing());
            assert_eq!(search.held_pages(), 0);
            let _ = std::fs::remove_dir_all(&dir);

            let (cups, search, dir) =
                started(&[queue("hp", "socket://192.0.2.21:9100", "HP LaserJet")]);
            let mut badges = Badges::default();
            store_page(&mut badges, 8, 144, 72);
            search.set_badges(badges);
            remember(&search, "Ann");
            search.set_formats(
                "hp",
                ipp_tool::Formats {
                    supported: vec!["application/pdf".to_string()],
                    default: Some("application/pdf".to_string()),
                    preferred: None,
                },
            );
            let mut pdf_request = PrintRequest {
                id: "5".to_string(),
                category: 8,
                ..PrintRequest::default()
            };
            pdf_request.double_print = true;
            let gate = Arc::new(Gate::new());
            let printed = search.print(&pdf_request, &gate).await.unwrap();
            assert!(printed.job_id.is_none());
            assert!(print_bodies(&cups).is_empty());
            assert_eq!(search.held_pages(), 2);
            assert!(!gate.is_printing());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn preview_does_not_submit() {
            let (cups, search, dir) = started(&[
                queue("hp", "socket://192.0.2.20:9100", "HP LaserJet"),
                queue(
                    "zebra",
                    "usb://Zebra/ZTC?serial=ABC",
                    "Zebra Technologies ZTC GX430t",
                ),
            ]);
            let fields = visitor("Ann");
            let mut badges = Badges::default();
            store_page(&mut badges, 3, 144, 72);
            let expected_page = badges
                .page_entry(3)
                .unwrap()
                .unwrap()
                .png_bytes(&fields, &HashMap::new(), 300.0)
                .unwrap();
            store_layout(&mut badges, 4, 144, 72);
            let expected_layout = badges
                .layout_entry(4)
                .unwrap()
                .unwrap()
                .png(&fields, 300, None)
                .unwrap();
            search.set_badges(badges);
            search.set_visitor("5", fields);
            search.set_last_print("hp", 40);
            search.set_last_print("zebra", 7);
            let page = search.preview(&request(3)).unwrap();
            assert_eq!(page, expected_page);
            let mut layout_request = request(4);
            layout_request.name = Some("hp".to_string());
            let layout = search.preview(&layout_request).unwrap();
            assert_eq!(layout, expected_layout);
            let mut zebra = request(4);
            zebra.name = Some("zebra".to_string());
            let zebra_png = search.preview(&zebra).unwrap();
            assert_eq!(&zebra_png[..8], b"\x89PNG\r\n\x1a\n");
            assert!(!zebra_png.windows(3).any(|window| window == b"^XA"));
            assert!(print_bodies(&cups).is_empty());
            assert_eq!(
                search
                    .printers()
                    .into_iter()
                    .find(|printer| printer.name == "hp")
                    .unwrap()
                    .last_print,
                40
            );
            assert_eq!(
                search
                    .printers()
                    .into_iter()
                    .find(|printer| printer.name == "zebra")
                    .unwrap()
                    .last_print,
                7
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[tokio::test]
        async fn certificate_layout() {
            let (cups, search, dir) =
                started(&[queue("certs", "socket://192.0.2.30:9100", "HP LaserJet")]);
            search
                .save_routing(
                    "certs",
                    Routing {
                        print_cert: true,
                        ..Routing::default()
                    },
                )
                .unwrap();
            let mut badges = Badges::default();
            store_page(&mut badges, 7, 720, 72);
            store_layout(&mut badges, 999, 100, 50);
            search.set_badges(badges);
            remember(&search, "Ann");
            let mut request = request(7);
            request.certificate = true;
            request.pixel_cap = Some(1200);
            let gate = Arc::new(Gate::new());
            search.print(&request, &gate).await.unwrap();
            let jobs = print_bodies(&cups);
            assert_eq!(jobs.len(), 1);
            let (width, _) = png_size(&jobs[0]);
            let dpi = capped_dpi(300, 100.0 / 72.0, 50.0 / 72.0, 1200);
            assert_eq!(dpi, 300);
            assert_eq!(width, (100.0_f64 / 72.0 * 300.0).round() as u32);
            assert!(width < 600, "{width}");
            assert_eq!(search.held_pages(), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    mod pdf_window {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        use super::super::print_job::marker_pdf;
        use super::super::{PrintRequest, Search};
        use super::queues::{no_browse, Cups, FoundDevice, Listed};
        use crate::registration_server::gate::Gate;
        use crate::registration_server::print::{Badges, PreparedLayout};

        fn queue(name: &'static str, uri: &'static str) -> Listed {
            Listed {
                name,
                uri,
                make_model: "HP LaserJet",
            }
        }

        fn started(queues: &[Listed]) -> (Cups, Search) {
            let cups = Cups::open(queues);
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            *cups.devices.lock().unwrap() = queues
                .iter()
                .map(|printer| FoundDevice {
                    uri: printer.uri.to_string(),
                    make_model: printer.make_model.to_string(),
                })
                .collect();
            search.refresh();
            search.wait().unwrap();
            let dir = std::env::temp_dir().join(format!(
                "rust-reg-window-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            search.set_files(&dir, "event");
            search.set_window_ms(500);
            (cups, search)
        }

        fn clock(search: &Search, at: u64) -> Arc<AtomicU64> {
            let now = Arc::new(AtomicU64::new(at));
            let reading = Arc::clone(&now);
            search.set_clock(Arc::new(move || reading.load(Ordering::SeqCst)));
            now
        }

        fn jobs(cups: &Cups) -> Vec<Vec<u8>> {
            let ops = cups.ops.lock().unwrap().clone();
            let bodies = cups.bodies.lock().unwrap().clone();
            ops.into_iter()
                .zip(bodies)
                .filter(|(op, _)| *op == 0x0002)
                .map(|(_, body)| body)
                .collect()
        }

        fn pdf_pages(body: &[u8]) -> usize {
            let start = body
                .windows(5)
                .position(|window| window == b"%PDF-")
                .unwrap();
            lopdf::Document::load_mem(&body[start..])
                .unwrap()
                .get_pages()
                .len()
        }

        #[test]
        fn two_pages_inside_the_window_are_one_pdf() {
            let queues = [queue("alpha", "socket://192.0.2.41:9100")];
            let (cups, search) = started(&queues);
            let now = clock(&search, 10_000);
            let gate = Arc::new(Gate::new());
            search
                .accept_pdf("alpha", None, None, marker_pdf("one"), &gate)
                .unwrap();
            now.store(10_400, Ordering::SeqCst);
            search
                .accept_pdf("alpha", None, None, marker_pdf("two"), &gate)
                .unwrap();
            assert!(jobs(&cups).is_empty());
            assert_eq!(search.held_pages(), 2);
            now.store(10_899, Ordering::SeqCst);
            assert!(search.flush_ready(&gate).unwrap().is_empty());
            assert!(jobs(&cups).is_empty());
            now.store(10_901, Ordering::SeqCst);
            let ids = search.flush_ready(&gate).unwrap();
            assert_eq!(ids, vec![1]);
            let sent = jobs(&cups);
            assert_eq!(sent.len(), 1);
            assert!(String::from_utf8_lossy(&sent[0]).contains("application/pdf"));
            assert_eq!(pdf_pages(&sent[0]), 2);
            assert_eq!(search.held_pages(), 0);
        }

        #[test]
        fn a_late_page_starts_a_new_document() {
            let (cups, search) = started(&[queue("alpha", "socket://192.0.2.42:9100")]);
            let now = clock(&search, 20_000);
            let gate = Arc::new(Gate::new());
            search
                .accept_pdf("alpha", None, None, marker_pdf("one"), &gate)
                .unwrap();
            now.store(20_500, Ordering::SeqCst);
            search
                .accept_pdf("alpha", None, None, marker_pdf("two"), &gate)
                .unwrap();
            let sent = jobs(&cups);
            assert_eq!(sent.len(), 1);
            assert_eq!(pdf_pages(&sent[0]), 1);
            assert_eq!(search.held_pages(), 1);
            now.store(21_001, Ordering::SeqCst);
            search.flush_ready(&gate).unwrap();
            let sent = jobs(&cups);
            assert_eq!(sent.len(), 2);
            assert_eq!(pdf_pages(&sent[1]), 1);
        }

        #[test]
        fn printers_do_not_share_a_window() {
            let (cups, search) = started(&[
                queue("alpha", "socket://192.0.2.43:9100"),
                queue("beta", "socket://192.0.2.44:9100"),
            ]);
            let now = clock(&search, 30_000);
            let gate = Arc::new(Gate::new());
            search
                .accept_pdf("alpha", None, None, marker_pdf("a"), &gate)
                .unwrap();
            now.store(30_010, Ordering::SeqCst);
            search
                .accept_pdf("beta", None, None, marker_pdf("b"), &gate)
                .unwrap();
            assert_eq!(search.printer_window_at("alpha"), Some(30_000));
            assert_eq!(search.printer_window_at("beta"), Some(30_010));
            now.store(30_511, Ordering::SeqCst);
            search.flush_ready(&gate).unwrap();
            let sent = jobs(&cups);
            assert_eq!(sent.len(), 2);
            assert!(sent.iter().all(|body| pdf_pages(body) == 1));
        }

        #[test]
        fn a_zone_has_one_window() {
            let (cups, search) = started(&[
                queue("alpha", "socket://192.0.2.45:9100"),
                queue("beta", "socket://192.0.2.46:9100"),
            ]);
            search
                .save_routing(
                    "alpha",
                    super::super::Routing {
                        zone: "2".to_string(),
                        ..super::super::Routing::default()
                    },
                )
                .unwrap();
            search
                .save_routing(
                    "beta",
                    super::super::Routing {
                        zone: "2".to_string(),
                        ..super::super::Routing::default()
                    },
                )
                .unwrap();
            search.set_last_print("alpha", 50);
            search.set_last_print("beta", 10);
            let now = clock(&search, 40_000);
            let gate = Arc::new(Gate::new());
            search
                .accept_pdf("alpha", Some("2"), None, marker_pdf("a"), &gate)
                .unwrap();
            now.store(40_010, Ordering::SeqCst);
            search
                .accept_pdf("beta", Some("2"), None, marker_pdf("b"), &gate)
                .unwrap();
            assert_eq!(search.zone_window_at("2"), Some(40_010));
            assert!(search.printer_window_at("alpha").is_none());
            assert_eq!(search.held_pages(), 2);
            now.store(40_511, Ordering::SeqCst);
            search.flush_ready(&gate).unwrap();
            let sent = jobs(&cups);
            assert_eq!(sent.len(), 1);
            assert_eq!(pdf_pages(&sent[0]), 2);
            let text = String::from_utf8_lossy(&sent[0]);
            assert!(text.contains("printers/alpha"), "{text}");
            assert_eq!(search.held_pages(), 0);
            let listed = search.printers();
            let alpha = listed
                .iter()
                .find(|printer| printer.name == "alpha")
                .unwrap();
            let beta = listed
                .iter()
                .find(|printer| printer.name == "beta")
                .unwrap();
            assert_ne!(alpha.last_print, 50);
            assert_eq!(beta.last_print, 10);
        }

        #[test]
        fn the_window_slides_with_the_last_page() {
            let (cups, search) = started(&[queue("alpha", "socket://192.0.2.47:9100")]);
            let now = clock(&search, 50_000);
            let gate = Arc::new(Gate::new());
            search
                .accept_pdf("alpha", None, None, marker_pdf("one"), &gate)
                .unwrap();
            now.store(50_400, Ordering::SeqCst);
            search
                .accept_pdf("alpha", None, None, marker_pdf("two"), &gate)
                .unwrap();
            now.store(50_800, Ordering::SeqCst);
            search
                .accept_pdf("alpha", None, None, marker_pdf("three"), &gate)
                .unwrap();
            assert!(jobs(&cups).is_empty());
            now.store(51_300, Ordering::SeqCst);
            assert!(search.flush_ready(&gate).unwrap().is_empty());
            now.store(51_301, Ordering::SeqCst);
            search.flush_ready(&gate).unwrap();
            let sent = jobs(&cups);
            assert_eq!(sent.len(), 1);
            assert_eq!(pdf_pages(&sent[0]), 3);
        }

        #[tokio::test]
        async fn zpl_and_png_are_not_held() {
            let cups = Cups::open(&[
                Listed {
                    name: "zebra",
                    uri: "usb://Zebra/ZTC?serial=ABC",
                    make_model: "Zebra Technologies ZTC GX430t",
                },
                Listed {
                    name: "hp",
                    uri: "socket://192.0.2.48:9100",
                    make_model: "HP LaserJet",
                },
            ]);
            let search = Search::open(&cups.base, Arc::new(no_browse())).unwrap();
            *cups.devices.lock().unwrap() = vec![
                FoundDevice {
                    uri: "usb://Zebra/ZTC?serial=ABC".to_string(),
                    make_model: "Zebra Technologies ZTC GX430t".to_string(),
                },
                FoundDevice {
                    uri: "socket://192.0.2.48:9100".to_string(),
                    make_model: "HP LaserJet".to_string(),
                },
            ];
            search.refresh();
            search.wait().unwrap();
            let mut badges = Badges::default();
            badges.replace_layout(
                7,
                Some(Ok(PreparedLayout::memory(
                    ticket_render::Layout {
                        width: 72,
                        height: 72,
                        dots_per_point: 1.0,
                        areas: Vec::new(),
                    },
                    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
                ))),
            );
            search.set_badges(badges);
            search.set_visitor(
                "5",
                std::collections::HashMap::from([("name".to_string(), "Ann".to_string())]),
            );
            let gate = Arc::new(Gate::new());
            let mut zpl = PrintRequest {
                id: "5".to_string(),
                category: 7,
                name: Some("zebra".to_string()),
                ..PrintRequest::default()
            };
            search.print(&zpl, &gate).await.unwrap();
            zpl.name = Some("hp".to_string());
            search.print(&zpl, &gate).await.unwrap();
            assert_eq!(search.held_pages(), 0);
            let sent = jobs(&cups);
            assert_eq!(sent.len(), 2);
            let text = String::from_utf8_lossy(&sent[0]);
            assert!(text.contains("^XA"), "{text}");
            let text = String::from_utf8_lossy(&sent[1]);
            assert!(text.contains("image/png"), "{text}");
        }
    }
}
