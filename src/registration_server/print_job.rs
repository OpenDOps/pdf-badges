//! `POST /print` and the PDF sliding window.
//! The worker picks, draws the one format that queue takes, then submits or appends.
//! ZPL and PNG are sent at once. A PDF page waits until the destination has been quiet.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lopdf::{dictionary, Document, Object, ObjectId, Stream};

use super::{Error, Job, Page, Printer, Search};
use crate::registration_server::gate::Gate;
use crate::registration_server::native_print::Kind;
use crate::registration_server::print::{Badges, PreparedLayout, PreparedPage};
use crate::struct_to_pdf::{self, media_size};

const TRANSLIT_KEYS: [&str; 12] = [
    "company",
    "cups_company",
    "position",
    "cups_position",
    "city",
    "cups_city",
    "name",
    "cups_name",
    "surname",
    "cups_surname",
    "patronymic",
    "cups_patronymic",
];

/// The body of `POST /print`. The worker loads the visitor fields for `id`.
#[derive(Debug, Clone)]
pub struct PrintRequest {
    pub id: String,
    pub category: i64,
    pub internet: bool,
    pub zone: Option<String>,
    pub name: Option<String>,
    pub certificate: bool,
    pub transliterate: bool,
    pub double_print: bool,
    pub rule: Option<String>,
    /// Longest side, in pixels. Absent uses 2400.
    pub pixel_cap: Option<u32>,
}

impl Default for PrintRequest {
    fn default() -> Self {
        Self {
            id: String::new(),
            category: 0,
            internet: false,
            zone: None,
            name: None,
            certificate: false,
            transliterate: false,
            double_print: false,
            rule: None,
            pixel_cap: None,
        }
    }
}

/// What `POST /print` returns. A sent page has a job id. A badge with no printer names both files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printed {
    pub job_id: Option<i32>,
    pub png: Option<PathBuf>,
    pub pdf: Option<PathBuf>,
}

pub(super) struct Desk {
    badges: Badges,
    visitors: HashMap<String, HashMap<String, String>>,
    windows: Windows,
    window_ms: u64,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

struct Windows {
    printers: BTreeMap<String, Slide>,
    zones: BTreeMap<String, Slide>,
}

struct Slide {
    pages: Vec<Held>,
    last_job_at: u64,
    printer: String,
    rule: Option<String>,
}

/// A page waiting in the window. A filled template is drawn once, with the other
/// pages, so repeated fonts and images stay one object. Raw bytes are an already
/// built PDF.
enum Held {
    Bytes(Vec<u8>),
    Filled(struct_to_pdf::Page),
}

enum Source {
    Layout(i64),
    Page(i64),
}

enum Drawn {
    Job(Page),
    Pdf(struct_to_pdf::Page),
}

impl Desk {
    pub(super) fn new() -> Self {
        let window_ms = std::env::var("SLIDING_WINDOW_TIME")
            .ok()
            .and_then(|text| text.parse().ok())
            .unwrap_or(500);
        Self {
            badges: Badges::default(),
            visitors: HashMap::new(),
            windows: Windows {
                printers: BTreeMap::new(),
                zones: BTreeMap::new(),
            },
            window_ms,
            now: Arc::new(unix_now),
        }
    }
}

impl Search {
    pub fn set_badges(&self, badges: Badges) {
        self.inner.desk.lock().expect("desk").badges = badges;
    }

    pub fn set_visitor(&self, id: &str, fields: HashMap<String, String>) {
        self.inner
            .desk
            .lock()
            .expect("desk")
            .visitors
            .insert(id.to_string(), fields);
    }

    pub fn set_clock(&self, now: Arc<dyn Fn() -> u64 + Send + Sync>) {
        self.inner.desk.lock().expect("desk").now = now;
    }

    pub fn set_window_ms(&self, window_ms: u64) {
        self.inner.desk.lock().expect("desk").window_ms = window_ms;
    }

    pub fn set_formats(&self, name: &str, formats: ipp_tool::Formats) {
        let mut slot = self.inner.slot.lock().expect("printers");
        for printer in &mut slot.current {
            if printer.name == name {
                printer.formats = formats.clone();
            }
        }
        for printer in &mut slot.devices {
            if printer.name == name {
                printer.formats = formats.clone();
            }
        }
        for printer in &mut slot.searching {
            if printer.name == name {
                printer.formats = formats.clone();
            }
        }
    }

    /// Pages still held. ZPL and PNG do not enter this count.
    pub fn held_pages(&self) -> usize {
        let desk = self.inner.desk.lock().expect("desk");
        desk.windows
            .printers
            .values()
            .chain(desk.windows.zones.values())
            .map(|slide| slide.pages.len())
            .sum()
    }

    pub fn printer_window_at(&self, name: &str) -> Option<u64> {
        self.inner
            .desk
            .lock()
            .expect("desk")
            .windows
            .printers
            .get(name)
            .map(|slide| slide.last_job_at)
    }

    pub fn zone_window_at(&self, zone: &str) -> Option<u64> {
        self.inner
            .desk
            .lock()
            .expect("desk")
            .windows
            .zones
            .get(zone)
            .map(|slide| slide.last_job_at)
    }

    /// Draw the PNG the picked queue would use. No `Print-Job` is sent.
    pub fn preview(&self, request: &PrintRequest) -> Result<Vec<u8>, Error> {
        let fields = self.visitor(&request.id)?;
        let printer = self.pick(&job_of(request))?;
        let cap = request.pixel_cap.unwrap_or(2400);
        let badges = self.badges();
        png_for(&badges, &printer, request, &fields, cap)
    }

    /// One registration. The draw does not hold the gate. A ZPL or PNG submit holds it
    /// on the blocking pool until that job returns. A PDF page appends and releases it.
    /// The quiet wait is not part of this call.
    pub async fn print(&self, request: &PrintRequest, gate: &Arc<Gate>) -> Result<Printed, Error> {
        let fields = self.visitor(&request.id)?;
        match self.pick(&job_of(request)) {
            Ok(printer) => self.print_on(request, &printer, &fields, gate).await,
            Err(err) if err.is_no_queue() => {
                let gate = Arc::clone(gate);
                let _hold = gate.enter_print();
                self.print_offline(request, &fields).await
            }
            Err(err) => Err(err),
        }
    }

    /// Append one PDF page that was already drawn for `printer`.
    /// A window that has been quiet for the sliding time is sent first.
    pub fn accept_pdf(
        &self,
        printer_name: &str,
        zone: Option<&str>,
        rule: Option<&str>,
        bytes: Vec<u8>,
        gate: &Arc<Gate>,
    ) -> Result<(), Error> {
        let printer = self.printer_named(printer_name)?;
        self.hold_pdf(&printer, zone, rule, Held::Bytes(bytes), gate, false)
    }

    /// Send every window whose last page is older than the sliding time.
    pub fn flush_ready(&self, gate: &Arc<Gate>) -> Result<Vec<i32>, Error> {
        let now = self.now_ms();
        self.flush_elapsed(now, gate, false, false)
    }

    /// Same flush as [`Self::flush_ready`], with each submit on the blocking pool.
    pub async fn flush_due(&self, gate: &Arc<Gate>) -> Result<Vec<i32>, Error> {
        let now = self.now_ms();
        self.flush_elapsed_async(now, gate, false).await
    }

    /// When the oldest open window should be sent. One millisecond past `last_job_at + window`,
    /// which is the first instant [`Self::flush_ready`] will send it.
    pub fn flush_wait(&self) -> Option<std::time::Duration> {
        let desk = self.inner.desk.lock().expect("desk");
        let window = desk.window_ms;
        let mut due = None;
        for slide in desk
            .windows
            .printers
            .values()
            .chain(desk.windows.zones.values())
        {
            if slide.pages.is_empty() {
                continue;
            }
            let at = slide.last_job_at.saturating_add(window).saturating_add(1);
            due = Some(due.map_or(at, |best: u64| best.min(at)));
        }
        let at = due?;
        let now = (desk.now)();
        Some(if at > now {
            std::time::Duration::from_millis(at - now)
        } else {
            std::time::Duration::ZERO
        })
    }

    async fn print_on(
        &self,
        request: &PrintRequest,
        printer: &Printer,
        fields: &HashMap<String, String>,
        gate: &Arc<Gate>,
    ) -> Result<Printed, Error> {
        let first = self.render_job(request, printer, fields)?;
        let mut job_id = self.dispatch(printer, request, first, gate).await?;
        if request.transliterate || request.double_print {
            tokio::task::yield_now().await;
            let next_fields = if request.transliterate {
                transliterate_fields(fields)
            } else {
                fields.clone()
            };
            let second = self.render_job(request, printer, &next_fields)?;
            let next = self.dispatch(printer, request, second, gate).await?;
            if job_id.is_none() {
                job_id = next;
            }
        }
        Ok(Printed {
            job_id,
            png: None,
            pdf: None,
        })
    }

    async fn print_offline(
        &self,
        request: &PrintRequest,
        fields: &HashMap<String, String>,
    ) -> Result<Printed, Error> {
        let cap = request.pixel_cap.unwrap_or(2400);
        let drawn = self.render_offline(request, fields, cap)?;
        let event = event_dir(self)?;
        let stem = file_stem(request.category, fields);
        let (png, pdf) = write_pair(&event, &stem, &drawn.png, &drawn.pdf)?;
        if request.transliterate || request.double_print {
            tokio::task::yield_now().await;
            let next_fields = if request.transliterate {
                transliterate_fields(fields)
            } else {
                fields.clone()
            };
            let drawn = self.render_offline(request, &next_fields, cap)?;
            let suffix = if request.transliterate { "tr" } else { "d" };
            write_pair(&event, &format!("{stem}-{suffix}"), &drawn.png, &drawn.pdf)?;
        }
        Ok(Printed {
            job_id: None,
            png: Some(png),
            pdf: Some(pdf),
        })
    }

    fn render_job(
        &self,
        request: &PrintRequest,
        printer: &Printer,
        fields: &HashMap<String, String>,
    ) -> Result<Drawn, Error> {
        let cap = request.pixel_cap.unwrap_or(2400);
        let badges = self.badges();
        let certificate = printer.routing.print_cert || request.certificate;
        let source = source_of(&badges, request.category, certificate)?;
        let (width_in, height_in) = inches_of(&badges, &source)?;
        let dpi = capped_dpi(i32::from(printer.dpi), width_in, height_in, cap);
        draw_for_queue(&badges, printer, &source, fields, dpi, width_in, height_in)
    }

    fn render_offline(
        &self,
        request: &PrintRequest,
        fields: &HashMap<String, String>,
        cap: u32,
    ) -> Result<Offline, Error> {
        let badges = self.badges();
        let source = source_of(&badges, request.category, false)?;
        let (width_in, height_in) = inches_of(&badges, &source)?;
        let dpi = capped_dpi(720, width_in, height_in, cap);
        let png = png_of(&badges, &source, fields, dpi)?;
        let pdf = match &source {
            Source::Page(category) => page_of(&badges, *category)?
                .pdf(fields, &HashMap::new())
                .map_err(page_err)?,
            Source::Layout(_) => pdf_of_png(&png, width_in, height_in)?,
        };
        Ok(Offline { png, pdf })
    }

    async fn dispatch(
        &self,
        printer: &Printer,
        request: &PrintRequest,
        drawn: Drawn,
        gate: &Arc<Gate>,
    ) -> Result<Option<i32>, Error> {
        match drawn {
            Drawn::Job(page) => self
                .submit_blocking(printer.clone(), page, gate)
                .await
                .map(Some),
            Drawn::Pdf(page) => {
                self.hold_pdf_async(
                    printer,
                    request.zone.as_deref(),
                    request.rule.as_deref(),
                    Held::Filled(page),
                    gate,
                )
                .await?;
                Ok(None)
            }
        }
    }

    /// The socket work runs on the blocking pool. The gate is held only for that submit.
    async fn submit_blocking(
        &self,
        printer: Printer,
        page: Page,
        gate: &Arc<Gate>,
    ) -> Result<i32, Error> {
        let search = self.clone();
        let gate = Arc::clone(gate);
        tokio::task::spawn_blocking(move || {
            let _hold = gate.enter_print();
            search.submit(&printer, &page)
        })
        .await
        .map_err(|err| Error::new(format!("print task stopped: {err}")))?
    }

    fn hold_pdf(
        &self,
        printer: &Printer,
        zone: Option<&str>,
        rule: Option<&str>,
        page: Held,
        gate: &Arc<Gate>,
        holding: bool,
    ) -> Result<(), Error> {
        let now = self.now_ms();
        self.flush_elapsed(now, gate, holding, true)?;
        self.append_page(printer, zone, rule, page, now);
        Ok(())
    }

    async fn hold_pdf_async(
        &self,
        printer: &Printer,
        zone: Option<&str>,
        rule: Option<&str>,
        page: Held,
        gate: &Arc<Gate>,
    ) -> Result<(), Error> {
        let now = self.now_ms();
        self.flush_elapsed_async(now, gate, true).await?;
        self.append_page(printer, zone, rule, page, now);
        Ok(())
    }

    fn append_page(
        &self,
        printer: &Printer,
        zone: Option<&str>,
        rule: Option<&str>,
        page: Held,
        now: u64,
    ) {
        let mut desk = self.inner.desk.lock().expect("desk");
        let slide = if let Some(zone) = zone {
            desk.windows.zones.entry(zone.to_string()).or_insert(Slide {
                pages: Vec::new(),
                last_job_at: now,
                printer: printer.name.clone(),
                rule: rule.map(str::to_string),
            })
        } else {
            desk.windows
                .printers
                .entry(printer.name.clone())
                .or_insert(Slide {
                    pages: Vec::new(),
                    last_job_at: now,
                    printer: printer.name.clone(),
                    rule: rule.map(str::to_string),
                })
        };
        if slide.rule.is_none() {
            slide.rule = rule.map(str::to_string);
        }
        slide.pages.push(page);
        slide.last_job_at = now;
    }

    fn flush_elapsed(
        &self,
        now: u64,
        gate: &Arc<Gate>,
        holding: bool,
        include_boundary: bool,
    ) -> Result<Vec<i32>, Error> {
        let due = {
            let desk = self.inner.desk.lock().expect("desk");
            let window = desk.window_ms;
            let mut due = Vec::new();
            for (key, slide) in &desk.windows.zones {
                if !slide.pages.is_empty()
                    && due_at(slide.last_job_at, window, now, include_boundary)
                {
                    due.push((true, key.clone()));
                }
            }
            for (key, slide) in &desk.windows.printers {
                if !slide.pages.is_empty()
                    && due_at(slide.last_job_at, window, now, include_boundary)
                {
                    due.push((false, key.clone()));
                }
            }
            due
        };
        let mut ids = Vec::new();
        for (is_zone, key) in due {
            ids.push(self.flush_key(is_zone, &key, gate, holding)?);
        }
        Ok(ids)
    }

    async fn flush_elapsed_async(
        &self,
        now: u64,
        gate: &Arc<Gate>,
        include_boundary: bool,
    ) -> Result<Vec<i32>, Error> {
        let due = self.due_windows(now, include_boundary);
        let mut ids = Vec::new();
        for (is_zone, key) in due {
            let ready = self.prepare_window(is_zone, &key)?;
            match self
                .submit_blocking(ready.printer, Page::Pdf { bytes: ready.bytes }, gate)
                .await
            {
                Ok(id) => ids.push(id),
                Err(err) => {
                    self.restore(ready.is_zone, &ready.key, ready.slide);
                    return Err(err);
                }
            }
        }
        Ok(ids)
    }

    fn due_windows(&self, now: u64, include_boundary: bool) -> Vec<(bool, String)> {
        let desk = self.inner.desk.lock().expect("desk");
        let window = desk.window_ms;
        let mut due = Vec::new();
        for (key, slide) in &desk.windows.zones {
            if !slide.pages.is_empty() && due_at(slide.last_job_at, window, now, include_boundary) {
                due.push((true, key.clone()));
            }
        }
        for (key, slide) in &desk.windows.printers {
            if !slide.pages.is_empty() && due_at(slide.last_job_at, window, now, include_boundary) {
                due.push((false, key.clone()));
            }
        }
        due
    }

    fn flush_key(
        &self,
        is_zone: bool,
        key: &str,
        gate: &Arc<Gate>,
        holding: bool,
    ) -> Result<i32, Error> {
        let ready = self.prepare_window(is_zone, key)?;
        let _hold = if holding {
            None
        } else {
            Some(gate.enter_print())
        };
        match self.submit(&ready.printer, &Page::Pdf { bytes: ready.bytes }) {
            Ok(id) => Ok(id),
            Err(err) => {
                self.restore(ready.is_zone, &ready.key, ready.slide);
                Err(err)
            }
        }
    }

    fn prepare_window(&self, is_zone: bool, key: &str) -> Result<ReadySend, Error> {
        let slide = {
            let mut desk = self.inner.desk.lock().expect("desk");
            if is_zone {
                desk.windows.zones.remove(key)
            } else {
                desk.windows.printers.remove(key)
            }
        };
        let Some(slide) = slide else {
            return Err(Error::new("window missing"));
        };
        let bytes = match assemble(&slide.pages) {
            Ok(bytes) => bytes,
            Err(message) => {
                self.restore(is_zone, key, slide);
                return Err(Error::new(message));
            }
        };
        let printer = match self.printer_named(&slide.printer) {
            Ok(printer) => printer,
            Err(err) => {
                self.restore(is_zone, key, slide);
                return Err(err);
            }
        };
        Ok(ReadySend {
            printer,
            bytes,
            is_zone,
            key: key.to_string(),
            slide,
        })
    }

    fn restore(&self, is_zone: bool, key: &str, slide: Slide) {
        let mut desk = self.inner.desk.lock().expect("desk");
        if is_zone {
            desk.windows.zones.insert(key.to_string(), slide);
        } else {
            desk.windows.printers.insert(key.to_string(), slide);
        }
    }

    fn visitor(&self, id: &str) -> Result<HashMap<String, String>, Error> {
        self.inner
            .desk
            .lock()
            .expect("desk")
            .visitors
            .get(id)
            .cloned()
            .ok_or_else(|| Error::new(format!("no registration {id}")))
    }

    fn printer_named(&self, name: &str) -> Result<Printer, Error> {
        self.printers()
            .into_iter()
            .find(|printer| printer.name == name)
            .ok_or_else(|| Error::new(format!("{name} is not on the current list")))
    }

    fn now_ms(&self) -> u64 {
        (self.inner.desk.lock().expect("desk").now)()
    }

    fn badges(&self) -> Badges {
        self.inner.desk.lock().expect("desk").badges.clone()
    }

    /// Field names of the page cached for `category`.
    pub fn prepared_fields(&self, category: i64) -> Option<Result<Vec<String>, String>> {
        self.badges().page_fields(category)
    }
}

struct Offline {
    png: Vec<u8>,
    pdf: Vec<u8>,
}

struct ReadySend {
    printer: Printer,
    bytes: Vec<u8>,
    is_zone: bool,
    key: String,
    slide: Slide,
}

fn job_of(request: &PrintRequest) -> Job {
    Job {
        name: request.name.clone(),
        category: Some(request.category.to_string()),
        internet: request.internet,
        certificate: request.certificate,
        zone: request.zone.clone(),
        rule: request.rule.clone(),
    }
}

fn source_of(badges: &Badges, category: i64, certificate: bool) -> Result<Source, Error> {
    if certificate {
        return Ok(Source::Layout(certificate_layout(badges, category)?));
    }
    if matches!(badges.page_entry(category), Some(Ok(_))) {
        return Ok(Source::Page(category));
    }
    if matches!(badges.layout_entry(category), Some(Ok(_))) {
        return Ok(Source::Layout(category));
    }
    Err(Error::new("no badge"))
}

fn certificate_layout(badges: &Badges, category: i64) -> Result<i64, Error> {
    for id in [category.saturating_add(1000), 999, -1] {
        if matches!(badges.layout_entry(id), Some(Ok(_))) {
            return Ok(id);
        }
    }
    badges
        .layout_ids()
        .into_iter()
        .filter(|id| *id >= 0)
        .min()
        .ok_or_else(|| Error::new("no layout"))
}

fn inches_of(badges: &Badges, source: &Source) -> Result<(f64, f64), Error> {
    match source {
        Source::Layout(category) => {
            let layout = layout_of(badges, *category)?;
            Ok(layout_inches(&layout.layout))
        }
        Source::Page(category) => {
            let page = page_of(badges, *category)?;
            let (width, height) = media_size(&page.page);
            Ok((f64::from(width) / 72.0, f64::from(height) / 72.0))
        }
    }
}

fn layout_inches(layout: &ticket_render::Layout) -> (f64, f64) {
    let dots = f64::from(layout.dots_per_point);
    (
        f64::from(layout.width) / dots / 72.0,
        f64::from(layout.height) / dots / 72.0,
    )
}

pub(super) fn capped_dpi(start: i32, width_in: f64, height_in: f64, cap: u32) -> i32 {
    let mut dpi = start.max(1);
    let mut divisor = 2i32;
    let cap = cap as f32;
    while long_side(width_in, height_in, dpi) > cap {
        let next = start / divisor;
        if next < 1 {
            return 1;
        }
        dpi = next;
        divisor += 1;
        if divisor > 64 {
            break;
        }
    }
    dpi.max(1)
}

fn long_side(width_in: f64, height_in: f64, dpi: i32) -> f32 {
    (width_in.max(height_in) * f64::from(dpi)) as f32
}

fn takes_pdf(printer: &Printer) -> bool {
    if printer.kind == Kind::Zpl {
        return false;
    }
    let formats = &printer.formats;
    if formats.supported.is_empty() && formats.preferred.is_none() && formats.default.is_none() {
        return true;
    }
    if formats.preferred.as_deref() == Some("application/pdf")
        || formats.default.as_deref() == Some("application/pdf")
    {
        return true;
    }
    let pdf = formats
        .supported
        .iter()
        .any(|item| item == "application/pdf");
    let png = formats.supported.iter().any(|item| item == "image/png");
    pdf && !png
}

fn draw_for_queue(
    badges: &Badges,
    printer: &Printer,
    source: &Source,
    fields: &HashMap<String, String>,
    dpi: i32,
    width_in: f64,
    height_in: f64,
) -> Result<Drawn, Error> {
    let pdf = matches!(source, Source::Page(_)) && takes_pdf(printer);
    if pdf {
        let Source::Page(category) = source else {
            return Err(Error::new("no page"));
        };
        return Ok(Drawn::Pdf(
            page_of(badges, *category)?
                .filled(fields, &HashMap::new())
                .map_err(page_err)?,
        ));
    }
    if printer.kind == Kind::Zpl {
        return match source {
            Source::Page(category) => {
                let (bytes, width_px) = page_of(badges, *category)?
                    .graphic(fields, &HashMap::new(), f64::from(dpi))
                    .map_err(page_err)?;
                Ok(Drawn::Job(Page::Graphic { bytes, width_px }))
            }
            Source::Layout(category) => {
                let layout = layout_of(badges, *category)?;
                let bytes = layout.graphic(fields, dpi).map_err(page_err)?;
                let width_px = pixels(layout.layout.width, layout.layout.dots_per_point, dpi);
                Ok(Drawn::Job(Page::Graphic { bytes, width_px }))
            }
        };
    }
    let bytes = png_of(badges, source, fields, dpi)?;
    Ok(Drawn::Job(Page::Png {
        bytes,
        width_in,
        height_in,
    }))
}

fn png_for(
    badges: &Badges,
    printer: &Printer,
    request: &PrintRequest,
    fields: &HashMap<String, String>,
    cap: u32,
) -> Result<Vec<u8>, Error> {
    let certificate = printer.routing.print_cert || request.certificate;
    let source = source_of(badges, request.category, certificate)?;
    let (width_in, height_in) = inches_of(badges, &source)?;
    let dpi = capped_dpi(i32::from(printer.dpi), width_in, height_in, cap);
    png_of(badges, &source, fields, dpi)
}

fn png_of(
    badges: &Badges,
    source: &Source,
    fields: &HashMap<String, String>,
    dpi: i32,
) -> Result<Vec<u8>, Error> {
    match source {
        Source::Page(category) => page_of(badges, *category)?
            .png_bytes(fields, &HashMap::new(), f64::from(dpi))
            .map_err(page_err),
        Source::Layout(category) => layout_of(badges, *category)?
            .png(fields, dpi, None)
            .map_err(page_err),
    }
}

fn layout_of(badges: &Badges, category: i64) -> Result<&PreparedLayout, Error> {
    match badges.layout_entry(category) {
        Some(Ok(layout)) => Ok(layout),
        Some(Err(message)) => Err(Error::new(message)),
        None => Err(Error::new(format!("no layout {category}"))),
    }
}

fn page_of(badges: &Badges, category: i64) -> Result<&PreparedPage, Error> {
    match badges.page_entry(category) {
        Some(Ok(page)) => Ok(page),
        Some(Err(message)) => Err(Error::new(message)),
        None => Err(Error::new(format!("no page {category}"))),
    }
}

fn pixels(length: i32, dots_per_point: f32, dpi: i32) -> u32 {
    (f64::from(length) / f64::from(dots_per_point) / 72.0 * f64::from(dpi)).round() as u32
}

fn page_err(error: impl std::fmt::Display) -> Error {
    Error::new(error.to_string())
}

fn event_dir(search: &Search) -> Result<PathBuf, Error> {
    let home =
        super::files_of(&search.inner).ok_or_else(|| Error::new("printer files are not set"))?;
    Ok(home.base.join(home.event_id))
}

fn file_stem(category: i64, fields: &HashMap<String, String>) -> String {
    let field = |key: &str| {
        fields
            .get(key)
            .map(|value| clean_file_part(value))
            .unwrap_or_default()
    };
    format!(
        "{category}_{}_{}_{}_{}",
        field("name"),
        field("surname"),
        field("patronymic"),
        field("company")
    )
}

fn clean_file_part(value: &str) -> String {
    value
        .chars()
        .filter(|ch| {
            !matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                && !ch.is_control()
                && !ch.is_whitespace()
        })
        .collect()
}

fn write_pair(
    event: &Path,
    stem: &str,
    png: &[u8],
    pdf: &[u8],
) -> Result<(PathBuf, PathBuf), Error> {
    let png_path = event.join("genbadges-png").join(format!("{stem}.png"));
    let pdf_path = event.join("genbadges-pdf").join(format!("{stem}.pdf"));
    if let Some(parent) = png_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| Error::new(err.to_string()))?;
    }
    if let Some(parent) = pdf_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| Error::new(err.to_string()))?;
    }
    std::fs::write(&png_path, png).map_err(|err| Error::new(err.to_string()))?;
    std::fs::write(&pdf_path, pdf).map_err(|err| Error::new(err.to_string()))?;
    Ok((png_path, pdf_path))
}

fn transliterate_fields(fields: &HashMap<String, String>) -> HashMap<String, String> {
    let mut next = fields.clone();
    for key in TRANSLIT_KEYS {
        if let Some(value) = next.get(key).cloned() {
            next.insert(key.to_string(), transliterate(&value));
        }
    }
    next
}

fn transliterate(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if let Some(latin) = latin(ch) {
            out.push_str(latin);
        } else {
            out.push(ch);
        }
    }
    out
}

fn latin(ch: char) -> Option<&'static str> {
    Some(match ch {
        'А' => "A",
        'а' => "a",
        'Б' => "B",
        'б' => "b",
        'В' => "V",
        'в' => "v",
        'Г' => "G",
        'г' => "g",
        'Д' => "D",
        'д' => "d",
        'Е' => "E",
        'е' => "e",
        'Ё' => "Yo",
        'ё' => "yo",
        'Ж' => "Zh",
        'ж' => "zh",
        'З' => "Z",
        'з' => "z",
        'И' => "I",
        'и' => "i",
        'Й' => "Y",
        'й' => "y",
        'К' => "K",
        'к' => "k",
        'Л' => "L",
        'л' => "l",
        'М' => "M",
        'м' => "m",
        'Н' => "N",
        'н' => "n",
        'О' => "O",
        'о' => "o",
        'П' => "P",
        'п' => "p",
        'Р' => "R",
        'р' => "r",
        'С' => "S",
        'с' => "s",
        'Т' => "T",
        'т' => "t",
        'У' => "U",
        'у' => "u",
        'Ф' => "F",
        'ф' => "f",
        'Х' => "Kh",
        'х' => "kh",
        'Ц' => "Ts",
        'ц' => "ts",
        'Ч' => "Ch",
        'ч' => "ch",
        'Ш' => "Sh",
        'ш' => "sh",
        'Щ' => "Sch",
        'щ' => "sch",
        'Ъ' | 'ъ' | 'Ь' | 'ь' => "",
        'Ы' => "Y",
        'ы' => "y",
        'Э' => "E",
        'э' => "e",
        'Ю' => "Yu",
        'ю' => "yu",
        'Я' => "Ya",
        'я' => "ya",
        _ => return None,
    })
}

/// A quiet window is due once `last + window` is before `now`.
/// A page arriving on that boundary does not extend the document, so the append includes it.
fn due_at(last_job_at: u64, window: u64, now: u64, include_boundary: bool) -> bool {
    let end = last_job_at.saturating_add(window);
    if include_boundary {
        end <= now
    } else {
        end < now
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn pdf_of_png(png: &[u8], width_in: f64, height_in: f64) -> Result<Vec<u8>, Error> {
    let image = image::load_from_memory(png)
        .map_err(|err| Error::new(err.to_string()))?
        .into_rgb8();
    let (width, height) = image.dimensions();
    let mut jpeg = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut jpeg);
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut cursor, image::ImageFormat::Jpeg)
        .map_err(|err| Error::new(err.to_string()))?;
    let width_pt = width_in * 72.0;
    let height_pt = height_in * 72.0;
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let image_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => width as i64,
            "Height" => height as i64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg,
    ));
    let content = format!("q\n{width_pt} 0 0 {height_pt} 0 0 cm\n/Im0 Do\nQ\n");
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), width_pt.into(), height_pt.into()],
        "Contents" => content_id,
        "Resources" => dictionary! {
            "XObject" => dictionary! {
                "Im0" => image_id,
            },
        },
    });
    doc.set_object(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        },
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|err| Error::new(err.to_string()))?;
    Ok(out)
}

#[cfg(test)]
pub(super) fn marker_pdf(text: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Courier",
    });
    let content = format!("BT\n/F1 24 Tf\n100 700 Td\n({text}) Tj\nET\n");
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Contents" => content_id,
        "Resources" => dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        },
    });
    doc.set_object(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        },
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("pdf");
    out
}

fn assemble(pages: &[Held]) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err("no pages".to_string());
    }
    if pages.iter().all(|page| matches!(page, Held::Filled(_))) {
        let filled = pages
            .iter()
            .filter_map(|page| match page {
                Held::Filled(page) => Some(page.clone()),
                Held::Bytes(_) => None,
            })
            .collect::<Vec<_>>();
        return struct_to_pdf::render_pages(&filled, Path::new(".")).map_err(|err| err.to_string());
    }
    let mut parts = Vec::new();
    let mut batch = Vec::new();
    for page in pages {
        match page {
            Held::Filled(page) => batch.push(page.clone()),
            Held::Bytes(bytes) => {
                flush_filled(&mut batch, &mut parts)?;
                parts.push(bytes.clone());
            }
        }
    }
    flush_filled(&mut batch, &mut parts)?;
    if parts.len() == 1 {
        Ok(parts.pop().unwrap())
    } else {
        join_pdfs(&parts)
    }
}

fn flush_filled(
    batch: &mut Vec<struct_to_pdf::Page>,
    parts: &mut Vec<Vec<u8>>,
) -> Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }
    let filled = std::mem::take(batch);
    parts
        .push(struct_to_pdf::render_pages(&filled, Path::new(".")).map_err(|err| err.to_string())?);
    Ok(())
}

fn join_pdfs(parts: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    if parts.is_empty() {
        return Err("no pages".to_string());
    }
    if parts.len() == 1 {
        return Ok(parts[0].clone());
    }
    let mut max_id = 1u32;
    let mut page_order: Vec<(ObjectId, Object)> = Vec::new();
    let mut objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut catalog: Option<(ObjectId, Object)> = None;
    let mut pages_root: Option<(ObjectId, Object)> = None;
    for part in parts {
        let mut doc = Document::load_mem(part).map_err(|err| err.to_string())?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;
        for id in doc.get_pages().into_values() {
            let object = doc.get_object(id).map_err(|err| err.to_string())?.clone();
            page_order.push((id, object));
        }
        for (id, object) in doc.objects {
            match object.type_name().unwrap_or(b"") {
                b"Catalog" if catalog.is_none() => catalog = Some((id, object)),
                b"Pages" if pages_root.is_none() => pages_root = Some((id, object)),
                b"Catalog" | b"Pages" | b"Page" | b"Outlines" | b"Outline" => {}
                _ => {
                    objects.insert(id, object);
                }
            }
        }
    }
    let (pages_id, pages_object) = pages_root.ok_or_else(|| "Pages root not found".to_string())?;
    let (catalog_id, catalog_object) =
        catalog.ok_or_else(|| "Catalog root not found".to_string())?;
    let mut document = Document::with_version("1.4");
    document.objects = objects;
    for (id, object) in &page_order {
        let Some(dict) = object.as_dict().ok() else {
            continue;
        };
        let mut dict = dict.clone();
        dict.set("Parent", pages_id);
        document.objects.insert(*id, Object::Dictionary(dict));
    }
    let mut pages = pages_object
        .as_dict()
        .map_err(|err| err.to_string())?
        .clone();
    pages.set("Count", page_order.len() as i64);
    pages.set(
        "Kids",
        page_order
            .iter()
            .map(|(id, _)| Object::Reference(*id))
            .collect::<Vec<_>>(),
    );
    document.objects.insert(pages_id, Object::Dictionary(pages));
    let mut catalog = catalog_object
        .as_dict()
        .map_err(|err| err.to_string())?
        .clone();
    catalog.set("Pages", pages_id);
    catalog.remove(b"Outlines");
    document
        .objects
        .insert(catalog_id, Object::Dictionary(catalog));
    document.trailer.set("Root", catalog_id);
    share_repeated(&mut document);
    document.renumber_objects();
    let mut out = Vec::new();
    document.save_to(&mut out).map_err(|err| err.to_string())?;
    Ok(out)
}

/// Identical font and image objects from separate page files become one object.
fn share_repeated(document: &mut Document) {
    loop {
        let mut seen: HashMap<Vec<u8>, ObjectId> = HashMap::new();
        let mut redirect: HashMap<ObjectId, ObjectId> = HashMap::new();
        let ids: Vec<ObjectId> = document.objects.keys().copied().collect();
        for id in ids {
            let Some(object) = document.objects.get(&id) else {
                continue;
            };
            if structural(object) {
                continue;
            }
            let key = signature(object);
            if let Some(first) = seen.get(&key) {
                redirect.insert(id, *first);
            } else {
                seen.insert(key, id);
            }
        }
        if redirect.is_empty() {
            break;
        }
        let objects = std::mem::take(&mut document.objects);
        for (id, object) in objects {
            if redirect.contains_key(&id) {
                continue;
            }
            document.objects.insert(id, rewrite(object, &redirect));
        }
    }
}

fn structural(object: &Object) -> bool {
    let Ok(dict) = object.as_dict() else {
        return false;
    };
    matches!(
        dict.get(b"Type")
            .ok()
            .and_then(|value| value.as_name().ok()),
        Some(b"Page" | b"Pages" | b"Catalog")
    )
}

fn signature(object: &Object) -> Vec<u8> {
    let mut out = Vec::new();
    write_signature(&mut out, object);
    out
}

fn write_signature(out: &mut Vec<u8>, object: &Object) {
    match object {
        Object::Null => out.push(0),
        Object::Boolean(value) => {
            out.push(1);
            out.push(u8::from(*value));
        }
        Object::Integer(value) => {
            out.push(2);
            out.extend_from_slice(&value.to_be_bytes());
        }
        Object::Real(value) => {
            out.push(3);
            out.extend_from_slice(&value.to_be_bytes());
        }
        Object::Name(value) => {
            out.push(4);
            out.extend_from_slice(value);
        }
        Object::String(value, _) => {
            out.push(5);
            out.extend_from_slice(value);
        }
        Object::Array(items) => {
            out.push(6);
            for item in items {
                write_signature(out, item);
            }
        }
        Object::Dictionary(dict) => {
            out.push(7);
            let mut pairs: Vec<_> = dict.iter().collect();
            pairs.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in pairs {
                out.extend_from_slice(key);
                write_signature(out, value);
            }
        }
        Object::Stream(stream) => {
            out.push(8);
            write_signature(out, &Object::Dictionary(stream.dict.clone()));
            out.extend_from_slice(&stream.content);
        }
        Object::Reference(id) => {
            out.push(9);
            out.extend_from_slice(&id.0.to_be_bytes());
            out.extend_from_slice(&id.1.to_be_bytes());
        }
    }
}

fn rewrite(object: Object, redirect: &HashMap<ObjectId, ObjectId>) -> Object {
    match object {
        Object::Array(items) => Object::Array(
            items
                .into_iter()
                .map(|item| rewrite(item, redirect))
                .collect(),
        ),
        Object::Dictionary(dict) => Object::Dictionary(rewrite_dict(dict, redirect)),
        Object::Stream(mut stream) => {
            stream.dict = rewrite_dict(stream.dict, redirect);
            Object::Stream(stream)
        }
        Object::Reference(id) => Object::Reference(follow(id, redirect)),
        other => other,
    }
}

fn rewrite_dict(
    dict: lopdf::Dictionary,
    redirect: &HashMap<ObjectId, ObjectId>,
) -> lopdf::Dictionary {
    let mut next = lopdf::Dictionary::new();
    for (key, value) in dict.iter() {
        next.set(key.to_vec(), rewrite(value.clone(), redirect));
    }
    next
}

fn follow(mut id: ObjectId, redirect: &HashMap<ObjectId, ObjectId>) -> ObjectId {
    for _ in 0..8 {
        let Some(next) = redirect.get(&id) else {
            break;
        };
        if *next == id {
            break;
        }
        id = *next;
    }
    id
}
