//! The event the venue process has open.
//!
//! Startup opens it only when the credential file already holds both the
//! event id and the project token. This module does not call the remote
//! service.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use crate::registration_server::credentials::{CredentialError, CredentialFile};
use crate::registration_server::event_db::{
    check_event_id, EventDb, EventDbError, IndexState, ListOrder, OutgoingAttachment, OutgoingScan,
    ScanWrite, SearchPage, VisitorWrite, WaitingRegistration,
};
use crate::registration_server::print::{self, Badges};
use crate::registration_server::remote_server::RemoteServer;

/// At most one open event, shared by the server thread and the sync thread.
///
/// Writes and reads name the event they belong to. Init and switch refuse new
/// calls. A switch waits until calls already admitted for the previous event
/// finish, and drops remote calls still running for that event. A read returns
/// the event id those rows belong to.
pub struct CurrentEvent {
    state: Mutex<State>,
    idle: Condvar,
    remote: Mutex<Option<RemoteServer>>,
    #[cfg(test)]
    pause: Mutex<Option<Arc<Pause>>>,
}

struct State {
    base: PathBuf,
    db: Option<Arc<Mutex<EventDb>>>,
    event_id: Option<String>,
    phase: Phase,
    inflight: u32,
    badges: Badges,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Closed,
    Opening,
    Ready,
    Switching,
}

struct Admitted<'a> {
    owner: &'a CurrentEvent,
    db: Arc<Mutex<EventDb>>,
    event_id: String,
}

/// Rows or one visitor read from the event that was open for this call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection<T> {
    pub event_id: String,
    pub value: T,
}

impl Drop for Admitted<'_> {
    fn drop(&mut self) {
        self.owner.retire();
    }
}

/// Failure while reading the credential file or opening the event.
#[derive(Debug)]
pub enum CurrentError {
    Credentials(CredentialError),
    Event(EventDbError),
    /// `event_id` is not the open event, or init or a switch is in progress.
    NotOpen {
        event_id: String,
    },
    /// An init or a switch is already in progress.
    Busy,
}

impl std::fmt::Display for CurrentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CurrentError::Credentials(err) => write!(f, "{err}"),
            CurrentError::Event(err) => write!(f, "{err}"),
            CurrentError::NotOpen { event_id } => {
                write!(f, "event {event_id} is not open")
            }
            CurrentError::Busy => write!(f, "event is opening or switching"),
        }
    }
}

impl std::error::Error for CurrentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CurrentError::Credentials(err) => Some(err),
            CurrentError::Event(err) => Some(err),
            CurrentError::NotOpen { .. } | CurrentError::Busy => None,
        }
    }
}

impl From<CredentialError> for CurrentError {
    fn from(err: CredentialError) -> Self {
        CurrentError::Credentials(err)
    }
}

impl From<EventDbError> for CurrentError {
    fn from(err: EventDbError) -> Self {
        CurrentError::Event(err)
    }
}

/// Directory that holds `{event_id}/db.sqlite`, next to the credential file.
pub fn base_dir(credentials_path: &Path) -> PathBuf {
    match credentials_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

impl CurrentEvent {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self {
            state: Mutex::new(State {
                base: base.into(),
                db: None,
                event_id: None,
                phase: Phase::Closed,
                inflight: 0,
                badges: Badges::default(),
            }),
            idle: Condvar::new(),
            remote: Mutex::new(None),
            #[cfg(test)]
            pause: Mutex::new(None),
        }
    }

    /// Share the token client with the sync thread. A later switch drops that
    /// client's calls for the event being left.
    pub(crate) fn set_remote(&self, remote: RemoteServer) {
        *self.remote.lock().unwrap_or_else(|err| err.into_inner()) = Some(remote);
    }

    /// Open the event named in `file` when that file is logged in.
    ///
    /// Both `event_id` and `project_token` must be set. Otherwise the holder
    /// stays empty and no database file is created. A document with only one
    /// of the pair is refused by the credential file. Writes are refused until
    /// this open finishes.
    pub fn open_bound(&self, file: &CredentialFile) -> Result<(), CurrentError> {
        let loaded = file.load()?;
        match loaded.event_id() {
            Some(event_id) => self.replace(event_id).map(|_| ()),
            None => self.close(),
        }
    }

    /// Open `event_id`, then replace the current event.
    ///
    /// Calls for the previous event that were already admitted finish first.
    /// Calls that arrive during the switch are refused. Remote calls still
    /// running for the previous id are dropped once the next file is open.
    /// A failed open leaves the current event in place and does not drop those
    /// calls. The same id does nothing. The returned id is the event open
    /// after this call.
    pub fn switch_to(&self, event_id: &str) -> Result<String, CurrentError> {
        self.replace(event_id)
    }

    /// Remove the event binding from `file` and close the open database.
    ///
    /// `db.sqlite` stays on disk. Calls already admitted finish first. `pump`
    /// after this returns false and inserts nothing.
    pub fn clear(&self, file: &CredentialFile) -> Result<(), CurrentError> {
        let mut loaded = file.load()?;
        loaded.clear_binding();
        file.store(&loaded)?;
        self.close()
    }

    /// Store `document` on the open event. The database fills its search tables
    /// and the memory list from that JSON.
    pub fn save_document(
        &self,
        event_id: &str,
        document: &serde_json::Value,
    ) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let mut db = lock_db(&admitted.db);
            db.save_document(document).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Write `visitor` only when `event_id` is the open event.
    pub fn write(&self, event_id: &str, visitor: &VisitorWrite) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let mut db = lock_db(&admitted.db);
            db.write(visitor).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// The cursor stored for `name` on `event_id`, or `"0"` when it is missing.
    ///
    /// A different open event is [`CurrentError::NotOpen`].
    pub fn cursor(&self, event_id: &str, name: &str) -> Result<String, CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.cursor(name).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Store `value` for `name` only when `event_id` is the open event.
    pub fn set_cursor(&self, event_id: &str, name: &str, value: &str) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.set_cursor(name, value).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Store ticket barcodes for `form_id` only when `event_id` is the open event.
    pub fn store_ticket_barcodes(
        &self,
        event_id: &str,
        form_id: i64,
        clean_id: i64,
        barcodes: &[String],
    ) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let mut db = lock_db(&admitted.db);
            db.store_ticket_barcodes(form_id, clean_id, barcodes)
                .map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// How many barcode rows this pool has, when `event_id` is the open event.
    pub fn barcode_count(
        &self,
        event_id: &str,
        cat_id: i64,
        bc_type: i64,
        date: &str,
    ) -> Result<i64, CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.barcode_count(cat_id, bc_type, date)
                .map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// The stored `clean_id` for this pool, when `event_id` is the open event.
    pub fn barcode_clean_id(
        &self,
        event_id: &str,
        cat_id: i64,
        bc_type: i64,
        date: &str,
    ) -> Result<Option<i64>, CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.barcode_clean_id(cat_id, bc_type, date)
                .map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Remove one barcode pool only when `event_id` is the open event.
    pub fn clean_barcodes(
        &self,
        event_id: &str,
        cat_id: i64,
        bc_type: i64,
        date: &str,
    ) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let mut db = lock_db(&admitted.db);
            db.clean_barcodes(cat_id, bc_type, date)
                .map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Store barcode rows only when `event_id` is the open event.
    pub fn store_barcodes(
        &self,
        event_id: &str,
        cat_id: i64,
        clean_id: i64,
        bc_type: i64,
        date: &str,
        barcodes: &[String],
    ) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let mut db = lock_db(&admitted.db);
            db.store_barcodes(cat_id, clean_id, bc_type, date, barcodes)
                .map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Waiting rows, then rows already marked in sync. An empty holder is `(0, 0)`.
    pub fn registration_sync_counts(&self) -> Result<(i64, i64), CurrentError> {
        let Some(event_id) = self.event_id() else {
            return Ok((0, 0));
        };
        let admitted = self.admit(&event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.registration_sync_counts().map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// How many registrations still have `in_synch` unset. An empty holder is 0.
    pub fn waiting_registration_count(&self) -> Result<i64, CurrentError> {
        let Some(event_id) = self.event_id() else {
            return Ok(0);
        };
        let admitted = self.admit(&event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.waiting_registration_count().map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Whether this event still has a registration with `in_synch` unset.
    pub fn has_waiting_registration(&self, event_id: &str) -> Result<bool, CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.has_waiting_registration().map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// The directory that holds `{event_id}/db.sqlite`.
    pub fn base_dir(&self) -> PathBuf {
        self.lock_state().base.clone()
    }

    /// A downloaded registration is already on the server.
    pub fn note_on_server(&self, event_id: &str, uid: &str) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.note_on_server(uid).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Registrations whose `in_synch` is unset.
    pub fn waiting_registrations(
        &self,
        event_id: &str,
        limit: usize,
    ) -> Result<Vec<WaitingRegistration>, CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.waiting_registrations(limit).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Ids listed in `synch_res` are on the server.
    pub fn accept_registrations(&self, event_id: &str, uids: &[&str]) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.accept_registrations(uids).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Scans that can go up for this event.
    pub fn outgoing_scans(
        &self,
        event_id: &str,
        limit: usize,
    ) -> Result<Vec<OutgoingScan>, CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.outgoing_scans(limit).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    pub fn mark_scans_synched(&self, event_id: &str, scanids: &[i64]) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.mark_scans_synched(scanids).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Attachments that can go up for this event.
    pub fn outgoing_attachments(
        &self,
        event_id: &str,
        limit: usize,
    ) -> Result<Vec<OutgoingAttachment>, CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.outgoing_attachments(limit).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    pub fn mark_attachments_sent(
        &self,
        event_id: &str,
        rows: &[(i64, i64)],
    ) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let db = lock_db(&admitted.db);
            db.mark_attachments_sent(rows).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Store one downloaded scan only when `event_id` is the open event.
    pub fn store_scan(&self, event_id: &str, scan: &ScanWrite) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let mut db = lock_db(&admitted.db);
            db.store_scan(scan).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Record a print only when `event_id` is the open event.
    pub fn record_print(&self, event_id: &str, uid: &str, ts: i64) -> Result<(), CurrentError> {
        let admitted = self.admit(event_id)?;
        let result = {
            let mut db = lock_db(&admitted.db);
            db.record_print(uid, ts).map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    /// Search committed rows only when `event_id` is the open event.
    ///
    /// A switch in progress, or a different open event, is refused. The
    /// returned id is the event these rows belong to.
    pub fn search(
        &self,
        event_id: &str,
        text: &str,
        categories: &[i64],
        order: ListOrder,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Selection<SearchPage>, CurrentError> {
        let admitted = self.admit(event_id)?;
        let event_id = admitted.event_id.clone();
        let value = {
            let db = lock_db(&admitted.db);
            db.search(text, categories, order, limit, offset)
                .map_err(CurrentError::from)?
        };
        drop(admitted);
        Ok(Selection { event_id, value })
    }

    /// The stored visitor JSON only when `event_id` is the open event.
    ///
    /// An unknown uid is `None` inside the selection. A switch in progress, or
    /// a different open event, is refused.
    pub fn full_row(
        &self,
        event_id: &str,
        uid: &str,
    ) -> Result<Selection<Option<String>>, CurrentError> {
        let admitted = self.admit(event_id)?;
        let event_id = admitted.event_id.clone();
        let value = {
            let db = lock_db(&admitted.db);
            db.full_row(uid).map_err(CurrentError::from)?
        };
        drop(admitted);
        Ok(Selection { event_id, value })
    }

    /// One list page, admitted so a switch cannot drop the file under it.
    ///
    /// The returned id is the event this page belongs to.
    pub fn table(
        &self,
        event_id: &str,
        categories: &[i64],
        order: ListOrder,
        limit: Option<u64>,
        from: Option<u64>,
    ) -> Result<Selection<SearchPage>, CurrentError> {
        let admitted = self.admit(event_id)?;
        let event_id = admitted.event_id.clone();
        let value = {
            let mut db = lock_db(&admitted.db);
            db.table(categories, order, limit, from)
                .map_err(CurrentError::from)?
        };
        drop(admitted);
        Ok(Selection { event_id, value })
    }

    /// Copy one batch when the open event is still loading.
    ///
    /// An empty file is already `ready`, so this returns false and inserts nothing.
    /// During init or a switch this returns false and does not insert.
    pub fn pump(&self) -> Result<bool, CurrentError> {
        let admitted = match self.admit_open() {
            Ok(admitted) => admitted,
            Err(CurrentError::NotOpen { .. } | CurrentError::Busy) => return Ok(false),
            Err(err) => return Err(err),
        };
        let result = {
            let mut db = lock_db(&admitted.db);
            db.load_batch().map_err(CurrentError::from)
        };
        drop(admitted);
        result
    }

    pub fn event_id(&self) -> Option<String> {
        self.lock_state().event_id.clone()
    }

    pub fn index_state(&self) -> Result<Option<IndexState>, EventDbError> {
        let Some(db) = self.db_arc() else {
            return Ok(None);
        };
        let state = lock_db(&db).index_state()?;
        Ok(Some(state))
    }

    pub fn loaded_count(&self) -> Result<u64, EventDbError> {
        let Some(db) = self.db_arc() else {
            return Ok(0);
        };
        let count = lock_db(&db).loaded_count()?;
        Ok(count)
    }

    pub fn with_db<T>(&self, f: impl FnOnce(&EventDb) -> T) -> Option<T> {
        let db = self.db_arc()?;
        let guard = lock_db(&db);
        Some(f(&guard))
    }

    pub fn lock(this: &Mutex<Self>) -> std::sync::MutexGuard<'_, Self> {
        this.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn replace(&self, event_id: &str) -> Result<String, CurrentError> {
        check_event_id(event_id)?;
        let mut state = self.lock_state();
        if matches!(state.phase, Phase::Opening | Phase::Switching) {
            return Err(CurrentError::Busy);
        }
        if state.phase == Phase::Ready && state.event_id.as_deref() == Some(event_id) {
            return Ok(event_id.to_string());
        }
        state.phase = if state.db.is_none() {
            Phase::Opening
        } else {
            Phase::Switching
        };
        while state.inflight > 0 {
            state = self.idle.wait(state).unwrap_or_else(|err| err.into_inner());
        }
        let base = state.base.clone();
        drop(state);
        self.pause_point();
        let opened = EventDb::open(&base, event_id);
        let mut state = self.lock_state();
        let badges = opened
            .as_ref()
            .ok()
            .map(|_| print::prepare_event(&base.join(event_id)));
        match opened {
            Ok(db) => {
                let previous = state.event_id.clone();
                state.db = Some(Arc::new(Mutex::new(db)));
                state.event_id = Some(event_id.to_string());
                state.badges = badges.unwrap_or_default();
                state.phase = Phase::Ready;
                drop(state);
                self.finish_switch(previous, event_id);
                Ok(event_id.to_string())
            }
            Err(err) => {
                state.phase = if state.db.is_some() {
                    Phase::Ready
                } else {
                    Phase::Closed
                };
                Err(err.into())
            }
        }
    }

    /// Close the open database. The credential file is left as it is.
    ///
    /// Calls already admitted finish first. An init or a switch already in
    /// progress is refused.
    pub fn close(&self) -> Result<(), CurrentError> {
        let mut state = self.lock_state();
        if matches!(state.phase, Phase::Opening | Phase::Switching) {
            return Err(CurrentError::Busy);
        }
        if state.db.is_none() {
            state.phase = Phase::Closed;
            state.event_id = None;
            state.badges = Badges::default();
            return Ok(());
        }
        state.phase = Phase::Switching;
        while state.inflight > 0 {
            state = self.idle.wait(state).unwrap_or_else(|err| err.into_inner());
        }
        state.db = None;
        state.event_id = None;
        state.badges = Badges::default();
        state.phase = Phase::Closed;
        Ok(())
    }

    /// The prepared layout size for `category`, or the decode error kept for that directory.
    pub fn badge_layout(&self, category: i64) -> Option<Result<(i32, i32), String>> {
        self.lock_state().badges.layout_size(category)
    }

    /// Template field names of the prepared page for `category`.
    pub fn badge_fields(&self, category: i64) -> Option<Result<Vec<String>, String>> {
        self.lock_state().badges.page_fields(category)
    }

    /// Draw the prepared layout. The call uses the bytes kept at prepare time.
    pub fn badge_png(&self, category: i64) -> Option<Result<Vec<u8>, String>> {
        self.lock_state()
            .badges
            .layout_png(category, &std::collections::HashMap::new(), 72)
    }

    pub fn badge_graphic(&self, category: i64) -> Option<Result<Vec<u8>, String>> {
        self.lock_state()
            .badges
            .layout_graphic(category, &std::collections::HashMap::new(), 72)
    }

    /// Draw the prepared page. The call uses the bytes kept at prepare time.
    pub fn badge_page_pdf(&self, category: i64) -> Option<Result<Vec<u8>, String>> {
        self.lock_state().badges.page_pdf_bytes(
            category,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
    }

    pub fn badge_page_png(&self, category: i64) -> Option<Result<Vec<u8>, String>> {
        self.lock_state().badges.page_raster_png(
            category,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            72.0,
        )
    }

    /// Reload `badge/{category}/badge.cfg` and `bg.png` when `event_id` is the open event.
    pub fn store_badge_cfg(&self, event_id: &str, category: i64) {
        self.store_badge(event_id, category, true);
    }

    /// Reload `badge/{category}/page.yaml` when `event_id` is the open event.
    pub fn store_page_file(&self, event_id: &str, category: i64) {
        self.store_badge(event_id, category, false);
    }

    fn store_badge(&self, event_id: &str, category: i64, layout: bool) {
        let state = self.lock_state();
        if state.phase != Phase::Ready || state.event_id.as_deref() != Some(event_id) {
            return;
        }
        let dir = state
            .base
            .join(event_id)
            .join("badge")
            .join(category.to_string());
        drop(state);
        let next_layout = layout.then(|| print::prepare_layout(&dir));
        let next_page = (!layout).then(|| print::prepare_page(&dir));
        let mut state = self.lock_state();
        if state.phase != Phase::Ready || state.event_id.as_deref() != Some(event_id) {
            return;
        }
        if let Some(layout) = next_layout {
            state.badges.replace_layout(category, layout);
        }
        if let Some(page) = next_page {
            state.badges.replace_page(category, page);
        }
    }

    fn admit(&self, event_id: &str) -> Result<Admitted<'_>, CurrentError> {
        let (db, open_id) = {
            let mut state = self.lock_state();
            if state.phase != Phase::Ready || state.event_id.as_deref() != Some(event_id) {
                return Err(CurrentError::NotOpen {
                    event_id: event_id.to_string(),
                });
            }
            state.inflight += 1;
            let db = state.db.clone().expect("ready event has a database");
            let open_id = state.event_id.clone().expect("ready event has an id");
            (db, open_id)
        };
        self.pause_point();
        Ok(Admitted {
            owner: self,
            db,
            event_id: open_id,
        })
    }

    fn admit_open(&self) -> Result<Admitted<'_>, CurrentError> {
        let event_id = {
            let state = self.lock_state();
            if state.phase != Phase::Ready {
                return Err(CurrentError::NotOpen {
                    event_id: state.event_id.clone().unwrap_or_default(),
                });
            }
            state.event_id.clone().unwrap()
        };
        self.admit(&event_id)
    }

    fn finish_switch(&self, previous: Option<String>, next: &str) {
        let remote = self
            .remote
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let Some(remote) = remote else {
            return;
        };
        remote.allow_requests(next);
        if let Some(previous) = previous.as_deref() {
            if previous != next {
                remote.drop_requests(previous);
            }
        }
    }

    fn retire(&self) {
        let mut state = self.lock_state();
        state.inflight = state.inflight.saturating_sub(1);
        if state.inflight == 0 {
            self.idle.notify_all();
        }
    }

    fn db_arc(&self) -> Option<Arc<Mutex<EventDb>>> {
        self.lock_state().db.clone()
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn pause_point(&self) {
        #[cfg(test)]
        {
            let Some(pause) = self
                .pause
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .take()
            else {
                return;
            };
            {
                let mut entered = pause.entered.lock().unwrap_or_else(|err| err.into_inner());
                *entered = true;
                pause.entered_cv.notify_all();
            }
            let mut release = pause.release.lock().unwrap_or_else(|err| err.into_inner());
            while !*release {
                release = pause
                    .release_cv
                    .wait(release)
                    .unwrap_or_else(|err| err.into_inner());
            }
        }
    }
}

fn lock_db(db: &Mutex<EventDb>) -> std::sync::MutexGuard<'_, EventDb> {
    db.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
struct Pause {
    entered: Mutex<bool>,
    entered_cv: Condvar,
    release: Mutex<bool>,
    release_cv: Condvar,
}

#[cfg(test)]
impl Pause {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Mutex::new(false),
            entered_cv: Condvar::new(),
            release: Mutex::new(false),
            release_cv: Condvar::new(),
        })
    }

    fn wait_entered(&self) {
        let mut entered = self.entered.lock().unwrap_or_else(|err| err.into_inner());
        while !*entered {
            entered = self
                .entered_cv
                .wait(entered)
                .unwrap_or_else(|err| err.into_inner());
        }
    }

    fn release(&self) {
        let mut release = self.release.lock().unwrap_or_else(|err| err.into_inner());
        *release = true;
        self.release_cv.notify_all();
    }
}

#[cfg(test)]
impl CurrentEvent {
    fn arm_pause(&self, pause: Arc<Pause>) {
        *self.pause.lock().unwrap_or_else(|err| err.into_inner()) = Some(pause);
    }

    fn accepts_writes(&self) -> bool {
        self.lock_state().phase == Phase::Ready
    }
}

#[cfg(test)]
mod tests {
    mod open_bound {
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};

        use crate::registration_server::credentials::{
            CredentialError, CredentialFile, Credentials,
        };
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::IndexState;

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

        struct Scratch {
            dir: std::path::PathBuf,
        }

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let dir = std::env::temp_dir()
                    .join(format!("rust-reg-current-{}-{n}", std::process::id()));
                fs::create_dir_all(&dir).unwrap();
                Self { dir }
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }

        #[test]
        fn startup_opens_a_bound_file() {
            let scratch = Scratch::new();
            let file = CredentialFile::new(scratch.dir.join("credentials.yml"));
            let mut credentials = Credentials::default();
            credentials.set_device_id("device-1").unwrap();
            credentials
                .bind("EVENT", "token-value", Some("Event".into()))
                .unwrap();
            file.store(&credentials).unwrap();
            assert!(!scratch.dir.join("EVENT").join("db.sqlite").exists());

            let current = CurrentEvent::new(&scratch.dir);
            current.open_bound(&file).unwrap();

            assert_eq!(current.event_id().as_deref(), Some("EVENT"));
            assert_eq!(current.index_state().unwrap(), Some(IndexState::Ready));
        }

        #[test]
        fn startup_without_a_token_opens_nothing() {
            let scratch = Scratch::new();
            let path = scratch.dir.join("credentials.yml");
            fs::write(&path, "version: 1\ndevice_id: device-1\nevent_id: EVENT\n").unwrap();
            let file = CredentialFile::new(&path);
            let current = CurrentEvent::new(&scratch.dir);
            let err = current.open_bound(&file).unwrap_err();
            match err {
                crate::registration_server::current::CurrentError::Credentials(
                    CredentialError::IncompleteBinding { missing, .. },
                ) => assert_eq!(missing, "project_token"),
                other => panic!("expected a refused credential file, got {other}"),
            }
            assert!(current.event_id().is_none());
            assert!(!scratch.dir.join("EVENT").join("db.sqlite").exists());

            let mut credentials = Credentials::default();
            credentials.set_device_id("device-1").unwrap();
            file.store(&credentials).unwrap();
            current.open_bound(&file).unwrap();
            assert!(current.event_id().is_none());
            assert!(current.index_state().unwrap().is_none());
            assert!(!scratch.dir.join("EVENT").join("db.sqlite").exists());
        }
    }

    mod init {
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};

        use crate::registration_server::credentials::{CredentialFile, Credentials};
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::IndexState;

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

        struct Scratch {
            dir: std::path::PathBuf,
        }

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let dir = std::env::temp_dir()
                    .join(format!("rust-reg-current-init-{}-{n}", std::process::id()));
                fs::create_dir_all(&dir).unwrap();
                Self { dir }
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }

        fn open_missing() -> (Scratch, CurrentEvent) {
            let scratch = Scratch::new();
            let file = CredentialFile::new(scratch.dir.join("credentials.yml"));
            let mut credentials = Credentials::default();
            credentials.set_device_id("device-1").unwrap();
            credentials
                .bind("EVENT", "token-value", Some("Event".into()))
                .unwrap();
            file.store(&credentials).unwrap();
            let current = CurrentEvent::new(&scratch.dir);
            current.open_bound(&file).unwrap();
            (scratch, current)
        }

        fn key_flags(current: &CurrentEvent) -> Vec<i64> {
            current
                .with_db(|db| {
                    let mut stmt = db.file().prepare("SELECT is_admin FROM keys").unwrap();
                    stmt.query_map([], |row| row.get(0))
                        .unwrap()
                        .map(|row| row.unwrap())
                        .collect()
                })
                .unwrap()
        }

        #[test]
        fn missing_file_is_ready() {
            let (scratch, current) = open_missing();
            assert!(scratch.dir.join("EVENT").join("db.sqlite").exists());
            let flags = key_flags(&current);
            assert_eq!(flags.iter().filter(|flag| **flag == 1).count(), 1);
            assert_eq!(flags.iter().filter(|flag| **flag == 0).count(), 1);
            assert_eq!(flags.len(), 2);
            assert_eq!(current.loaded_count().unwrap(), 0);
            assert_eq!(current.index_state().unwrap(), Some(IndexState::Ready));
        }

        #[test]
        fn pump_on_an_empty_file_is_idle() {
            let (_scratch, current) = open_missing();
            assert!(!current.pump().unwrap());
            assert_eq!(current.loaded_count().unwrap(), 0);
            assert_eq!(current.index_state().unwrap(), Some(IndexState::Ready));
        }
    }

    mod pump {
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        use crate::registration_server::credentials::{CredentialFile, Credentials};
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::{IndexState, ListOrder, DB_FILE};

        const FIXTURE: &str = "/Users/mac/Documents/development/JJO/Irbis/temp-reg/registration_x86/base/GELCPAQSFL/db.sqlite";

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-current-pump-{}-{}-{n}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                fs::create_dir_all(&path).unwrap();
                Self(path)
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn open_fixture() -> (Scratch, CurrentEvent, i64) {
            let scratch = Scratch::new();
            let src = Path::new(FIXTURE);
            assert!(
                src.is_file(),
                "scala event database is missing at {}",
                src.display()
            );
            let event = scratch.0.join("GELCPAQSFL");
            fs::create_dir_all(&event).unwrap();
            fs::copy(src, event.join(DB_FILE)).unwrap();
            let file = CredentialFile::new(scratch.0.join("credentials.yml"));
            let mut credentials = Credentials::default();
            credentials.set_device_id("device-1").unwrap();
            credentials
                .bind("GELCPAQSFL", "token-value", Some("Event".into()))
                .unwrap();
            file.store(&credentials).unwrap();
            let current = CurrentEvent::new(&scratch.0);
            current.open_bound(&file).unwrap();
            let on_disk: i64 = current
                .with_db(|db| {
                    db.file()
                        .query_row("SELECT COUNT(*) FROM mem_u", [], |row| row.get(0))
                        .unwrap()
                })
                .unwrap();
            assert!(on_disk > 2500, "the copied database has {on_disk} visitors");
            (scratch, current, on_disk)
        }

        /// `uid` and surname in `last_add` order: newer `mem_u.ts` first, then `uid`.
        fn last_add(current: &CurrentEvent) -> Vec<(String, String)> {
            current
                .with_db(|db| {
                    let mut stmt = db
                        .file()
                        .prepare(
                            "SELECT mem_u.uid, IFNULL(u_fulltext.surname, '')
                             FROM mem_u
                             LEFT JOIN u_fulltext ON u_fulltext.uid = mem_u.uid
                             ORDER BY mem_u.ts DESC, mem_u.uid",
                        )
                        .unwrap();
                    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                        .unwrap()
                        .map(|row| row.unwrap())
                        .collect()
                })
                .unwrap()
        }

        fn unique_surname(
            order: &[(String, String)],
            range: std::ops::Range<usize>,
        ) -> (String, String) {
            order[range]
                .iter()
                .find(|(_, surname)| {
                    !surname.is_empty()
                        && order.iter().filter(|(_, other)| other == surname).count() == 1
                })
                .expect("the fixture has a unique surname in this range")
                .clone()
        }

        fn list_has(current: &CurrentEvent, uid: &str) -> bool {
            let count: i64 = current
                .with_db(|db| {
                    db.memory()
                        .query_row(
                            "SELECT COUNT(*) FROM list_row WHERE uid = ?1",
                            [uid],
                            |row| row.get(0),
                        )
                        .unwrap()
                })
                .unwrap();
            count > 0
        }

        #[test]
        fn open_does_not_copy() {
            let source_len = fs::metadata(FIXTURE).unwrap().len();
            let (_scratch, current, on_disk) = open_fixture();
            assert!(on_disk > 2500);
            assert_eq!(current.loaded_count().unwrap(), 0);
            assert_eq!(current.index_state().unwrap(), Some(IndexState::Loading));
            assert_eq!(fs::metadata(FIXTURE).unwrap().len(), source_len);
        }

        #[test]
        fn first_pump_is_five_pages() {
            let (_scratch, current, _) = open_fixture();
            let order = last_add(&current);
            assert!(order.len() > 100);
            assert!(current.pump().unwrap());
            assert_eq!(current.loaded_count().unwrap(), 100);
            let page = current
                .table("GELCPAQSFL", &[], ListOrder::LastAdd, Some(100), Some(0))
                .unwrap();
            assert_eq!(page.event_id, "GELCPAQSFL");
            let uids: Vec<&str> = page.value.rows.iter().map(|row| row.uid.as_str()).collect();
            let expected: Vec<&str> = order[..100].iter().map(|(uid, _)| uid.as_str()).collect();
            assert_eq!(uids, expected);
            assert!(!list_has(&current, &order[100].0));
            assert_eq!(current.loaded_count().unwrap(), 100);
        }

        #[test]
        fn pump_stops_when_ready() {
            let (_scratch, current, on_disk) = open_fixture();
            assert!(on_disk > 2500);
            while current.pump().unwrap() {}
            assert_eq!(current.index_state().unwrap(), Some(IndexState::Ready));
            assert_eq!(current.loaded_count().unwrap(), on_disk as u64);
            assert_eq!(current.loaded_count().unwrap(), 5176);
            assert!(!current.pump().unwrap());
            assert_eq!(current.loaded_count().unwrap(), 5176);
        }

        #[test]
        fn search_sees_the_first_pages() {
            let (_scratch, current, _) = open_fixture();
            let order = last_add(&current);
            let (early_uid, early_name) = unique_surname(&order, 0..100);
            let (later_uid, later_name) = unique_surname(&order, 100..300);
            assert!(current.pump().unwrap());
            let found = current
                .search(
                    "GELCPAQSFL",
                    &early_name,
                    &[],
                    ListOrder::LastAdd,
                    None,
                    None,
                )
                .unwrap();
            assert_eq!(found.event_id, "GELCPAQSFL");
            assert_eq!(found.value.rows.len(), 1);
            assert_eq!(found.value.rows[0].uid, early_uid);
            let missing = current
                .search(
                    "GELCPAQSFL",
                    &later_name,
                    &[],
                    ListOrder::LastAdd,
                    None,
                    None,
                )
                .unwrap();
            assert_eq!(missing.event_id, "GELCPAQSFL");
            assert_eq!(missing.value.total, 0);
            assert!(current.pump().unwrap());
            let later = current
                .search(
                    "GELCPAQSFL",
                    &later_name,
                    &[],
                    ListOrder::LastAdd,
                    None,
                    None,
                )
                .unwrap();
            assert_eq!(later.event_id, "GELCPAQSFL");
            assert!(later.value.rows.iter().any(|row| row.uid == later_uid));
        }
    }

    mod switch {
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};

        use rusqlite::Connection;

        use crate::registration_server::current::{CurrentError, CurrentEvent};
        use crate::registration_server::event_db::{
            EventDbError, IndexState, ListOrder, VisitorWrite, DB_FILE,
        };

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

        struct Scratch {
            dir: std::path::PathBuf,
        }

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let dir = std::env::temp_dir().join(format!(
                    "rust-reg-current-switch-{}-{n}",
                    std::process::id()
                ));
                fs::create_dir_all(&dir).unwrap();
                Self { dir }
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }

        fn visitor(uid: &str) -> VisitorWrite {
            VisitorWrite {
                uid: uid.to_string(),
                data: format!(r#"{{"uid":"{uid}"}}"#),
                name: "запись".to_string(),
                surname: "переключение".to_string(),
                c_name: String::new(),
                category: 7,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 10,
                org_id: None,
                email: format!("{uid}@example.com"),
            }
        }

        fn open_aaa() -> (Scratch, CurrentEvent) {
            let scratch = Scratch::new();
            let current = CurrentEvent::new(&scratch.dir);
            current.switch_to("AAA").unwrap();
            current.write("AAA", &visitor("1")).unwrap();
            (scratch, current)
        }

        fn file_has(path: &std::path::Path, uid: &str) -> bool {
            let conn = Connection::open(path).unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM mem_u WHERE uid = ?1", [uid], |row| {
                    row.get(0)
                })
                .unwrap();
            count > 0
        }

        #[test]
        fn switch_opens_the_next_before_dropping() {
            let (scratch, current) = open_aaa();
            assert_eq!(current.switch_to("BBB").unwrap(), "BBB");
            assert_eq!(current.event_id().as_deref(), Some("BBB"));
            assert_eq!(current.index_state().unwrap(), Some(IndexState::Ready));
            assert_eq!(current.loaded_count().unwrap(), 0);
            let page = current
                .search("BBB", "", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.event_id, "BBB");
            assert_eq!(page.value.total, 0);
            assert!(file_has(&scratch.dir.join("AAA").join(DB_FILE), "1"));
        }

        #[test]
        fn switch_does_not_copy_rows() {
            let (scratch, current) = open_aaa();
            current.switch_to("BBB").unwrap();
            assert!(!file_has(&scratch.dir.join("BBB").join(DB_FILE), "1"));
            let page = current
                .search("BBB", "переключение", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.event_id, "BBB");
            assert_eq!(page.value.total, 0);
            assert_eq!(current.loaded_count().unwrap(), 0);
        }

        #[test]
        fn failed_switch_keeps_the_current_file() {
            let (scratch, current) = open_aaa();
            let escaped = scratch.dir.join("..").join("x");
            let existed = escaped.exists();
            let err = current.switch_to("../x").unwrap_err();
            assert!(matches!(err, CurrentError::Event(EventDbError::BadExpoId)));
            assert_eq!(current.event_id().as_deref(), Some("AAA"));
            let page = current
                .search("AAA", "переключение", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.event_id, "AAA");
            assert_eq!(page.value.rows.len(), 1);
            assert_eq!(page.value.rows[0].uid, "1");
            if !existed {
                assert!(!escaped.exists());
            }
            assert!(!scratch.dir.join("x").exists());
        }

        #[test]
        fn switch_to_the_same_id_keeps_memory() {
            let (_scratch, current) = open_aaa();
            let uri = current.with_db(|db| db.memory_uri().to_string()).unwrap();
            assert_eq!(current.switch_to("AAA").unwrap(), "AAA");
            assert_eq!(current.event_id().as_deref(), Some("AAA"));
            assert_eq!(
                current.with_db(|db| db.memory_uri().to_string()).unwrap(),
                uri
            );
            assert_eq!(current.loaded_count().unwrap(), 1);
            let page = current
                .search("AAA", "переключение", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.event_id, "AAA");
            assert_eq!(page.value.rows.len(), 1);
            assert_eq!(page.value.rows[0].uid, "1");
        }
    }

    mod guard {
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        use std::thread;
        use std::time::{Duration, Instant};

        use rusqlite::Connection;

        use crate::registration_server::current::{CurrentError, CurrentEvent};
        use crate::registration_server::event_db::{ListOrder, VisitorWrite, DB_FILE};

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

        struct Scratch {
            dir: std::path::PathBuf,
        }

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let dir = std::env::temp_dir()
                    .join(format!("rust-reg-current-guard-{}-{n}", std::process::id()));
                fs::create_dir_all(&dir).unwrap();
                Self { dir }
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }

        fn visitor(uid: &str) -> VisitorWrite {
            VisitorWrite {
                uid: uid.to_string(),
                data: format!(r#"{{"uid":"{uid}"}}"#),
                name: "запись".to_string(),
                surname: "защита".to_string(),
                c_name: String::new(),
                category: 7,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 10,
                org_id: None,
                email: format!("{uid}@example.com"),
            }
        }

        fn file_has(path: &std::path::Path, uid: &str) -> bool {
            let conn = Connection::open(path).unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM mem_u WHERE uid = ?1", [uid], |row| {
                    row.get(0)
                })
                .unwrap();
            count > 0
        }

        #[test]
        fn write_names_the_open_event() {
            let scratch = Scratch::new();
            let current = CurrentEvent::new(&scratch.dir);
            current.switch_to("AAA").unwrap();
            let err = current.write("BBB", &visitor("1")).unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            let err = current.record_print("BBB", "1", 10).unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            assert!(!scratch.dir.join("BBB").exists());
            current.write("AAA", &visitor("1")).unwrap();
            current.record_print("AAA", "1", 10).unwrap();
            assert!(file_has(&scratch.dir.join("AAA").join(DB_FILE), "1"));
        }

        #[test]
        fn init_rejects_a_write() {
            let scratch = Scratch::new();
            let current = Arc::new(CurrentEvent::new(&scratch.dir));
            let pause = crate::registration_server::current::Pause::new();
            current.arm_pause(Arc::clone(&pause));
            let opening = {
                let current = Arc::clone(&current);
                thread::spawn(move || current.switch_to("AAA"))
            };
            pause.wait_entered();
            let err = current.write("AAA", &visitor("1")).unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            pause.release();
            opening.join().unwrap().unwrap();
            current.write("AAA", &visitor("1")).unwrap();
            assert!(file_has(&scratch.dir.join("AAA").join(DB_FILE), "1"));
        }

        #[test]
        fn switch_finishes_the_previous_write_and_rejects_the_rest() {
            let scratch = Scratch::new();
            let current = Arc::new(CurrentEvent::new(&scratch.dir));
            current.switch_to("AAA").unwrap();
            let pause = crate::registration_server::current::Pause::new();
            current.arm_pause(Arc::clone(&pause));
            let writing = {
                let current = Arc::clone(&current);
                thread::spawn(move || current.write("AAA", &visitor("1")))
            };
            pause.wait_entered();
            let switching = {
                let current = Arc::clone(&current);
                thread::spawn(move || current.switch_to("BBB"))
            };
            let started = Instant::now();
            while current.accepts_writes() {
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("switch did not refuse the next write");
                }
                thread::yield_now();
            }
            let err = current.write("AAA", &visitor("3")).unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            pause.release();
            writing.join().unwrap().unwrap();
            switching.join().unwrap().unwrap();
            assert!(file_has(&scratch.dir.join("AAA").join(DB_FILE), "1"));
            assert!(!file_has(&scratch.dir.join("BBB").join(DB_FILE), "1"));
            assert!(!file_has(&scratch.dir.join("AAA").join(DB_FILE), "3"));
            let err = current.write("AAA", &visitor("4")).unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            current.write("BBB", &visitor("5")).unwrap();
            assert_eq!(current.event_id().as_deref(), Some("BBB"));
        }

        #[test]
        fn search_returns_the_open_event() {
            let scratch = Scratch::new();
            let current = CurrentEvent::new(&scratch.dir);
            current.switch_to("AAA").unwrap();
            current.write("AAA", &visitor("1")).unwrap();
            let err = current
                .search("BBB", "защита", &[], ListOrder::LastAdd, None, None)
                .unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            let found = current
                .search("AAA", "защита", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(found.event_id, "AAA");
            assert_eq!(found.value.rows.len(), 1);
            assert_eq!(found.value.rows[0].uid, "1");
            let row = current.full_row("AAA", "1").unwrap();
            assert_eq!(row.event_id, "AAA");
            assert!(row.value.unwrap().contains("\"uid\":\"1\""));
            let err = current.full_row("BBB", "1").unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            let page = current
                .table("AAA", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.event_id, "AAA");
            assert_eq!(page.value.rows[0].uid, "1");
        }

        #[test]
        fn search_during_switch_is_rejected() {
            let scratch = Scratch::new();
            let current = Arc::new(CurrentEvent::new(&scratch.dir));
            current.switch_to("AAA").unwrap();
            current.write("AAA", &visitor("1")).unwrap();
            let pause = crate::registration_server::current::Pause::new();
            current.arm_pause(Arc::clone(&pause));
            let searching = {
                let current = Arc::clone(&current);
                thread::spawn(move || {
                    current.search("AAA", "защита", &[], ListOrder::LastAdd, None, None)
                })
            };
            pause.wait_entered();
            let switching = {
                let current = Arc::clone(&current);
                thread::spawn(move || current.switch_to("BBB"))
            };
            let started = Instant::now();
            while current.accepts_writes() {
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("switch did not refuse the next search");
                }
                thread::yield_now();
            }
            let err = current
                .search("AAA", "защита", &[], ListOrder::LastAdd, None, None)
                .unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            let err = current.full_row("AAA", "1").unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            pause.release();
            let found = searching.join().unwrap().unwrap();
            assert_eq!(found.event_id, "AAA");
            assert_eq!(found.value.rows[0].uid, "1");
            assert_eq!(switching.join().unwrap().unwrap(), "BBB");
            let err = current
                .search("AAA", "защита", &[], ListOrder::LastAdd, None, None)
                .unwrap_err();
            assert!(matches!(err, CurrentError::NotOpen { .. }));
            let found = current
                .search("BBB", "защита", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(found.event_id, "BBB");
            assert_eq!(found.value.total, 0);
        }
    }

    mod clear {
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};

        use rusqlite::Connection;

        use crate::registration_server::credentials::{CredentialFile, Credentials};
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::event_db::{IndexState, ListOrder, VisitorWrite, DB_FILE};

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

        struct Scratch {
            dir: std::path::PathBuf,
        }

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let dir = std::env::temp_dir()
                    .join(format!("rust-reg-current-clear-{}-{n}", std::process::id()));
                fs::create_dir_all(&dir).unwrap();
                Self { dir }
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }

        fn visitor(uid: &str) -> VisitorWrite {
            VisitorWrite {
                uid: uid.to_string(),
                data: format!(r#"{{"uid":"{uid}"}}"#),
                name: "запись".to_string(),
                surname: "очистка".to_string(),
                c_name: String::new(),
                category: 7,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 10,
                org_id: None,
                email: format!("{uid}@example.com"),
            }
        }

        fn file_has(path: &std::path::Path, uid: &str) -> bool {
            let conn = Connection::open(path).unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM mem_u WHERE uid = ?1", [uid], |row| {
                    row.get(0)
                })
                .unwrap();
            count > 0
        }

        fn bound_file(dir: &std::path::Path) -> CredentialFile {
            let file = CredentialFile::new(dir.join("credentials.yml"));
            let mut credentials = Credentials::default();
            credentials.set_device_id("device-1").unwrap();
            credentials.set_base_url("https://kuprin.su/").unwrap();
            credentials
                .bind("AAA", "token-value", Some("Мероприятие".into()))
                .unwrap();
            file.store(&credentials).unwrap();
            file
        }

        fn open_with_visitor(dir: &std::path::Path) -> (CredentialFile, CurrentEvent) {
            let file = bound_file(dir);
            let current = CurrentEvent::new(dir);
            current.open_bound(&file).unwrap();
            current.write("AAA", &visitor("1")).unwrap();
            (file, current)
        }

        #[test]
        fn clear_closes_the_pair() {
            let scratch = Scratch::new();
            let (file, current) = open_with_visitor(&scratch.dir);
            let db_path = scratch.dir.join("AAA").join(DB_FILE);
            current.clear(&file).unwrap();
            assert!(current.event_id().is_none());
            assert!(current.index_state().unwrap().is_none());
            assert!(!current.pump().unwrap());
            assert_eq!(current.loaded_count().unwrap(), 0);
            assert!(db_path.is_file());
            assert!(file_has(&db_path, "1"));
        }

        #[test]
        fn clear_keeps_device_and_url() {
            let scratch = Scratch::new();
            let (file, current) = open_with_visitor(&scratch.dir);
            current.clear(&file).unwrap();
            let loaded = file.load().unwrap();
            assert_eq!(loaded.device_id(), Some("device-1"));
            assert_eq!(loaded.base_url(), Some("https://kuprin.su/"));
            assert_eq!(loaded.event_id(), None);
            assert_eq!(loaded.event_name(), None);
            assert_eq!(loaded.project_token(), None);
            let text = fs::read_to_string(file.path()).unwrap();
            assert!(!text.contains("event_id"));
            assert!(!text.contains("event_name"));
            assert!(!text.contains("project_token"));
        }

        #[test]
        fn open_again_loads_the_same_file() {
            let scratch = Scratch::new();
            let (file, current) = open_with_visitor(&scratch.dir);
            let db_path = scratch.dir.join("AAA").join(DB_FILE);
            current.clear(&file).unwrap();
            let mut credentials = file.load().unwrap();
            credentials
                .bind("AAA", "token-again", Some("Мероприятие".into()))
                .unwrap();
            file.store(&credentials).unwrap();
            current.open_bound(&file).unwrap();
            assert_eq!(current.event_id().as_deref(), Some("AAA"));
            assert!(file_has(&db_path, "1"));
            assert_eq!(current.index_state().unwrap(), Some(IndexState::Loading));
            assert_eq!(current.loaded_count().unwrap(), 0);
            assert!(!current.pump().unwrap());
            assert_eq!(current.index_state().unwrap(), Some(IndexState::Ready));
            assert_eq!(current.loaded_count().unwrap(), 1);
            let page = current
                .search("AAA", "очистка", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.event_id, "AAA");
            assert_eq!(page.value.rows.len(), 1);
            assert_eq!(page.value.rows[0].uid, "1");
        }
    }
}
