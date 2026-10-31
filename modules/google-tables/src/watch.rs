//! Quiet-period loop, subscribe, and the watch RPCs. The loop is the only writer.

use std::collections::{BTreeMap, HashMap};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, Notify};
use tonic::Status;

use crate::cycle::{run_cycle, CycleStop, CycleWatch, MappedColumn};
use crate::google::{
    BookCell, ColRange, FakeBook, GoogleError, GoogleSheets, Meta, SheetsFuture, ValueRun,
};
use crate::irbis::google_tables::v1::{
    ChangeKind, GetSnapshotRequest, GetWatchRequest, ListSnapshotsRequest, ReadStoredRequest,
    RowChange, SnapshotBatch, SnapshotInfo, StartWatchRequest, StopWatchRequest, StoredBatch,
    SubscribeRequest, SyncNowRequest, SyncReport, Watch, WatchRecord,
};
use crate::irbis::table::v1::{Cell, Row};
use crate::limiter::{Kind, Limiter};
use crate::session::{proto_column, GoogleTablesService};
use crate::store::StoreError;

const QUIET: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_secs(60);
const BATCH: usize = 100;
const STREAM_BUFFER: usize = 1_000;
const TERMINAL_WAIT: Duration = Duration::from_secs(1);

struct QuietState {
    real: bool,
    now: Instant,
    fires: usize,
    blocked: usize,
    epoch: u64,
    waits: Vec<Duration>,
}

pub struct Quiet {
    state: Mutex<QuietState>,
    cv: Notify,
}

impl Quiet {
    pub fn real() -> Arc<Self> {
        Arc::new(Self::new(true))
    }

    pub fn manual() -> Arc<Self> {
        Arc::new(Self::new(false))
    }

    fn new(real: bool) -> Self {
        Self {
            state: Mutex::new(QuietState {
                real,
                now: Instant::now(),
                fires: 0,
                blocked: 0,
                epoch: 0,
                waits: Vec::new(),
            }),
            cv: Notify::new(),
        }
    }

    /// Wall time when the clock is real. The manual clock moves only when a wait finishes.
    pub fn now(&self) -> Instant {
        let state = self.state.lock().expect("quiet");
        if state.real {
            Instant::now()
        } else {
            state.now
        }
    }

    pub async fn wait(&self, duration: Duration) {
        if self.state.lock().expect("quiet").real {
            tokio::time::sleep(duration).await;
            return;
        }
        let started = {
            let mut state = self.state.lock().expect("quiet");
            let started = state.now;
            state.waits.push(duration);
            state.epoch += 1;
            state.blocked += 1;
            started
        };
        self.cv.notify_waiters();
        loop {
            if self.take_fire() {
                self.advance_to(started + duration);
                return;
            }
            let notified = self.cv.notified();
            if self.take_fire() {
                self.advance_to(started + duration);
                return;
            }
            notified.await;
        }
    }

    fn advance_to(&self, end: Instant) {
        let mut state = self.state.lock().expect("quiet");
        if end > state.now {
            state.now = end;
        }
    }

    fn take_fire(&self) -> bool {
        let mut state = self.state.lock().expect("quiet");
        if state.fires == 0 {
            return false;
        }
        state.fires -= 1;
        state.blocked = state.blocked.saturating_sub(1);
        true
    }

    pub fn fire(&self) {
        let mut state = self.state.lock().expect("quiet");
        if state.real {
            return;
        }
        state.fires += 1;
        drop(state);
        self.cv.notify_waiters();
    }

    pub async fn until_blocked(&self) {
        if self.state.lock().expect("quiet").real {
            return;
        }
        loop {
            if self.state.lock().expect("quiet").blocked > 0 {
                return;
            }
            let notified = self.cv.notified();
            if self.state.lock().expect("quiet").blocked > 0 {
                return;
            }
            notified.await;
        }
    }

    pub async fn advance(&self) {
        let epoch = {
            let state = self.state.lock().expect("quiet");
            assert!(state.blocked > 0, "watch is not waiting");
            state.epoch
        };
        self.fire();
        loop {
            if self.advanced(epoch) {
                return;
            }
            let notified = self.cv.notified();
            if self.advanced(epoch) {
                return;
            }
            notified.await;
        }
    }

    fn advanced(&self, epoch: u64) -> bool {
        let state = self.state.lock().expect("quiet");
        state.blocked > 0 && state.epoch != epoch
    }

    pub fn waits(&self) -> Vec<Duration> {
        self.state.lock().expect("quiet").waits.clone()
    }
}

impl GoogleTablesService {
    pub async fn quiet_blocked(&self) {
        self.inner.quiet.until_blocked().await;
    }

    pub async fn advance_quiet(&self) {
        self.inner.quiet.advance().await;
    }

    pub fn quiet_waits(&self) -> Vec<Duration> {
        self.inner.quiet.waits()
    }

    /// Subscribe tasks parked on an empty log, waiting for the next commit.
    pub fn subscribers_idle(&self) -> usize {
        self.inner.subscribers_idle.load(Ordering::SeqCst)
    }

    pub async fn shutdown(&self) {
        self.inner
            .halt
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let handles: Vec<_> = self
            .inner
            .tasks
            .lock()
            .expect("tasks")
            .drain()
            .map(|(_, handle)| handle)
            .collect();
        for handle in handles {
            handle.abort();
            let _ = handle.await;
        }
    }

    pub(crate) fn resume_spreadsheet(&self, spreadsheet_id: &str) {
        let Ok(watches) = self.inner.store.watches() else {
            return;
        };
        let stopped = self.inner.stopped.lock().expect("stopped").clone();
        for (wid, record) in watches {
            if record.spreadsheet_id == spreadsheet_id && !stopped.contains(&wid) {
                self.spawn(wid);
            }
        }
    }

    pub fn start_watch(&self, request: StartWatchRequest) -> Result<Watch, Status> {
        let session = self.session(&request.session_id)?;
        if session.columns.is_empty() {
            return Err(Status::failed_precondition("columns"));
        }
        let header_row = session.subcolumn_row.or(session.header_row).unwrap_or(0);
        {
            let books = self.inner.books.lock().expect("books");
            let book = books
                .get(&session.spreadsheet_id)
                .ok_or_else(|| Status::not_found("unknown spreadsheet"))?;
            let tab = book
                .tab(session.sheet_id)
                .ok_or_else(|| Status::not_found("unknown sheet"))?;
            if tab
                .metadata
                .iter()
                .all(|item| item.key != crate::google::KEY_UID)
                || tab
                    .metadata
                    .iter()
                    .all(|item| item.key != crate::google::KEY_FINGERPRINT)
            {
                return Err(Status::failed_precondition("identity"));
            }
        }
        let watches = self
            .inner
            .store
            .watches()
            .map_err(|err| Status::internal(err.to_string()))?;
        let existing = watches.into_iter().find(|(_, record)| {
            record.spreadsheet_id == session.spreadsheet_id && record.sheet_id == session.sheet_id
        });
        let (wid, mut record) = if let Some((wid, record)) = existing {
            (wid, record)
        } else {
            let wid = self
                .inner
                .store
                .watches()
                .map_err(|err| Status::internal(err.to_string()))?
                .into_iter()
                .map(|(wid, _)| wid)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            (
                wid,
                WatchRecord {
                    status: "running".into(),
                    ..WatchRecord::default()
                },
            )
        };
        if self.inner.tasks.lock().expect("tasks").contains_key(&wid) {
            return Err(Status::already_exists("watch"));
        }
        record.spreadsheet_id = session.spreadsheet_id;
        record.sheet_id = session.sheet_id;
        record.credentials_ref = session.credentials_ref;
        record.columns = session.columns.iter().map(proto_column).collect();
        record.exclude_from_hash = session
            .exclude
            .iter()
            .filter(|(_, excluded)| **excluded)
            .map(|(name, _)| name.clone())
            .collect();
        record.header_row = header_row;
        if record.status.is_empty() {
            record.status = "running".into();
        }
        self.inner
            .store
            .save_watch(wid, &record)
            .map_err(|err| Status::internal(err.to_string()))?;
        self.inner.stopped.lock().expect("stopped").remove(&wid);
        self.spawn(wid);
        Ok(watch_message(wid, &record))
    }

    pub fn stop_watch(&self, request: StopWatchRequest) -> Result<Watch, Status> {
        let wid = parse_wid(&request.watch_id)?;
        let record = self.saved(wid)?;
        self.inner.stopped.lock().expect("stopped").insert(wid);
        if let Some(handle) = self.inner.tasks.lock().expect("tasks").remove(&wid) {
            handle.abort();
        }
        Ok(watch_message(wid, &record))
    }

    pub async fn sync_now(&self, request: SyncNowRequest) -> Result<SyncReport, Status> {
        let wid = parse_wid(&request.watch_id)?;
        let before = self.saved(wid)?.last_seq;
        self.run_cycle(wid).await?;
        let record = self.saved(wid)?;
        let events = match self.inner.store.changes_after(wid, before) {
            Ok(events) => events,
            Err(StoreError::OutOfRange) => Vec::new(),
            Err(err) => return Err(Status::internal(err.to_string())),
        };
        if events.iter().any(|event| event.seq > before) {
            let _ = self.inner.store.trim(wid, self.inner.retain_events);
            self.notify_commits(wid);
        }
        Ok(report(&record, &events))
    }

    pub fn get_watch(&self, request: GetWatchRequest) -> Result<Watch, Status> {
        let wid = parse_wid(&request.watch_id)?;
        let record = self.saved(wid)?;
        Ok(watch_message(wid, &record))
    }

    pub fn subscribe(
        &self,
        request: SubscribeRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<RowChange, Status>> + Send>>, Status>
    {
        let wid = parse_wid(&request.watch_id)?;
        self.saved(wid)?;
        match self.inner.store.changes_after(wid, request.after_seq) {
            Err(StoreError::OutOfRange) => return Err(Status::out_of_range("event log")),
            Err(err) => return Err(Status::internal(err.to_string())),
            Ok(_) => {}
        }
        let (tx, rx) = mpsc::channel(STREAM_BUFFER);
        let service = self.clone();
        let after = request.after_seq;
        let commits = service.commits_for(wid);
        tokio::spawn(async move {
            let mut cursor = after;
            let mut following = false;
            loop {
                if service.inner.halt.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let notified = commits.notified();
                let events = match service.inner.store.changes_after(wid, cursor) {
                    Ok(events) => events,
                    Err(StoreError::OutOfRange) => {
                        send_terminal(&tx, Status::out_of_range("event log")).await;
                        return;
                    }
                    Err(err) => {
                        send_terminal(&tx, Status::internal(err.to_string())).await;
                        return;
                    }
                };
                if events.is_empty() {
                    following = true;
                    service
                        .inner
                        .subscribers_idle
                        .fetch_add(1, Ordering::SeqCst);
                    notified.await;
                    service
                        .inner
                        .subscribers_idle
                        .fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                if following {
                    let last_seq = service
                        .saved(wid)
                        .map(|record| record.last_seq)
                        .unwrap_or(cursor);
                    if last_seq.saturating_sub(cursor) > STREAM_BUFFER as u64 {
                        send_terminal(&tx, Status::resource_exhausted("event buffer")).await;
                        return;
                    }
                }
                for event in events {
                    cursor = event.seq;
                    if tx.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
            }
        });
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    pub fn changes(&self, watch_id: &str) -> Result<Vec<RowChange>, Status> {
        let wid = parse_wid(watch_id)?;
        self.saved(wid)?;
        self.inner
            .store
            .changes_after(wid, 0)
            .map_err(|err| match err {
                StoreError::OutOfRange => Status::out_of_range("event log"),
                other => Status::internal(other.to_string()),
            })
    }

    pub fn read_stored_batches(
        &self,
        request: ReadStoredRequest,
    ) -> Result<Vec<StoredBatch>, Status> {
        let wid = parse_wid(&request.watch_id)?;
        let record = self.saved(wid)?;
        let snapshot = self
            .inner
            .store
            .read_stored(wid)
            .map_err(|err| Status::internal(err.to_string()))?;
        let rows: Vec<Row> = snapshot
            .rows
            .iter()
            .map(|row| Row {
                excel_row: row.sheet_row,
                cells: row
                    .cells
                    .iter()
                    .map(|cell| Cell {
                        excel_id: record
                            .columns
                            .iter()
                            .find(|column| column.name == cell.name)
                            .map(|column| column.excel_id.clone())
                            .unwrap_or_default(),
                        value: cell.value.clone(),
                    })
                    .collect(),
            })
            .collect();
        Ok(rows
            .chunks(BATCH)
            .map(|chunk| StoredBatch {
                snapshot_seq: snapshot.snapshot_seq,
                rows: chunk.to_vec(),
            })
            .collect())
    }

    pub fn list_snapshots(
        &self,
        request: ListSnapshotsRequest,
    ) -> Result<Vec<SnapshotInfo>, Status> {
        let wid = parse_wid(&request.watch_id)?;
        self.saved(wid)?;
        let records = self
            .inner
            .store
            .list_snapshots(wid)
            .map_err(|err| Status::internal(err.to_string()))?;
        Ok(records
            .into_iter()
            .map(|record| SnapshotInfo {
                snapshot_id: record.snapshot_id,
                reason: record.reason,
                created_at_unix_ms: record.created_at_unix_ms,
                row_count: record.rows.len() as u32,
            })
            .collect())
    }

    pub fn get_snapshot_batches(
        &self,
        request: GetSnapshotRequest,
    ) -> Result<Vec<SnapshotBatch>, Status> {
        let wid = parse_wid(&request.watch_id)?;
        let record = self.saved(wid)?;
        let snapshot = self
            .inner
            .store
            .get_snapshot(wid, request.snapshot_id)
            .map_err(|err| Status::internal(err.to_string()))?
            .ok_or_else(|| {
                Status::not_found(format!("unknown snapshot {}", request.snapshot_id))
            })?;
        let rows: Vec<Row> = snapshot
            .rows
            .iter()
            .map(|row| Row {
                excel_row: row.sheet_row,
                cells: row
                    .cells
                    .iter()
                    .map(|cell| Cell {
                        excel_id: record
                            .columns
                            .iter()
                            .find(|column| column.name == cell.name)
                            .map(|column| column.excel_id.clone())
                            .unwrap_or_default(),
                        value: cell.value.clone(),
                    })
                    .collect(),
            })
            .collect();
        if rows.is_empty() {
            return Ok(vec![SnapshotBatch {
                snapshot_id: snapshot.snapshot_id,
                reason: snapshot.reason,
                created_at_unix_ms: snapshot.created_at_unix_ms,
                rows: Vec::new(),
            }]);
        }
        Ok(rows
            .chunks(BATCH)
            .map(|chunk| SnapshotBatch {
                snapshot_id: snapshot.snapshot_id,
                reason: snapshot.reason,
                created_at_unix_ms: snapshot.created_at_unix_ms,
                rows: chunk.to_vec(),
            })
            .collect())
    }

    fn saved(&self, wid: u32) -> Result<WatchRecord, Status> {
        self.inner
            .store
            .load_saved(wid)
            .map_err(|err| Status::internal(err.to_string()))?
            .ok_or_else(|| Status::not_found(format!("unknown watch {wid}")))
    }

    fn spawn(&self, wid: u32) {
        if self.inner.halt.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let mut tasks = self.inner.tasks.lock().expect("tasks");
        if tasks.contains_key(&wid) {
            return;
        }
        let service = self.clone();
        tasks.insert(
            wid,
            tokio::spawn(async move { service.watch_loop(wid).await }),
        );
    }

    async fn watch_loop(&self, wid: u32) {
        let mut started = false;
        loop {
            if self.stopping(wid) {
                return;
            }
            let Ok(record) = self.saved(wid) else {
                return;
            };
            let Ok(live) = self
                .live_version(&record.spreadsheet_id, &record.credentials_ref)
                .await
            else {
                continue;
            };
            let caught_up =
                started && !record.drive_version.is_empty() && live == record.drive_version;
            if caught_up {
                self.inner.quiet.wait(POLL).await;
                continue;
            }
            self.inner.quiet.wait(QUIET).await;
            if self.stopping(wid) {
                return;
            }
            let Ok(again) = self
                .live_version(&record.spreadsheet_id, &record.credentials_ref)
                .await
            else {
                continue;
            };
            if again != live {
                continue;
            }
            let before_seq = record.last_seq;
            let _ = self.run_cycle(wid).await;
            let after_seq = self
                .saved(wid)
                .map(|saved| saved.last_seq)
                .unwrap_or(before_seq);
            if after_seq > before_seq {
                let _ = self.inner.store.trim(wid, self.inner.retain_events);
                self.notify_commits(wid);
            }
            started = true;
        }
    }

    fn commits_for(&self, wid: u32) -> Arc<Notify> {
        self.inner
            .commits
            .lock()
            .expect("commits")
            .entry(wid)
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    fn notify_commits(&self, wid: u32) {
        if let Some(notify) = self.inner.commits.lock().expect("commits").get(&wid) {
            notify.notify_waiters();
        }
    }

    fn stopping(&self, wid: u32) -> bool {
        self.inner.halt.load(std::sync::atomic::Ordering::SeqCst)
            || self.inner.stopped.lock().expect("stopped").contains(&wid)
    }

    async fn live_version(
        &self,
        spreadsheet_id: &str,
        credentials_ref: &str,
    ) -> Result<String, ()> {
        loop {
            if self.inner.halt.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(());
            }
            match pace_book(
                &self.inner.books,
                &self.inner.limiter,
                &self.inner.quiet,
                credentials_ref,
                spreadsheet_id,
                Kind::Read,
                |book| book.version(spreadsheet_id),
            )
            .await
            {
                Ok(version) => return Ok(version),
                Err(_) => self.inner.quiet.wait(QUIET).await,
            }
        }
    }

    fn cycle_gate(&self, wid: u32) -> Arc<tokio::sync::Mutex<()>> {
        self.inner
            .cycles
            .lock()
            .expect("cycles")
            .entry(wid)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// The watch cycle for `watch_id` is in progress, including a backoff wait.
    pub fn cycle_busy(&self, watch_id: &str) -> bool {
        let Ok(wid) = watch_id.parse::<u32>() else {
            return false;
        };
        let gates = self.inner.cycles.lock().expect("cycles");
        let Some(gate) = gates.get(&wid) else {
            return false;
        };
        let busy = gate.try_lock().is_err();
        busy
    }

    async fn run_cycle(&self, wid: u32) -> Result<CycleStop, Status> {
        let gate = self.cycle_gate(wid);
        let _guard = gate.lock().await;
        let record = self.saved(wid)?;
        let watch = CycleWatch {
            wid,
            spreadsheet_id: record.spreadsheet_id.clone(),
            sheet_id: record.sheet_id,
            header_row: record.header_row,
            columns: record
                .columns
                .iter()
                .map(|column| MappedColumn {
                    name: column.name.clone(),
                    exclude_from_hash: record
                        .exclude_from_hash
                        .iter()
                        .any(|name| name == &column.name),
                })
                .collect(),
        };
        let quiet = Arc::clone(&self.inner.quiet);
        let mut client = LockedBook {
            books: &self.inner.books,
            limiter: &self.inner.limiter,
            quiet: Arc::clone(&self.inner.quiet),
            credentials_ref: record.credentials_ref.clone(),
        };
        let stop = run_cycle(
            &self.inner.store,
            &mut client,
            &watch,
            move |delay| {
                let quiet = Arc::clone(&quiet);
                Box::pin(async move {
                    if !delay.is_zero() {
                        quiet.wait(delay).await;
                    }
                })
            },
            || Duration::ZERO,
        )
        .await
        .map_err(|err| match err {
            GoogleError::Message(text) if text.contains("unknown spreadsheet") => {
                Status::not_found(text)
            }
            other => Status::internal(other.to_string()),
        })?;
        Ok(stop)
    }
}

struct LockedBook<'a> {
    books: &'a Mutex<HashMap<String, FakeBook>>,
    limiter: &'a Limiter,
    quiet: Arc<Quiet>,
    credentials_ref: String,
}

impl GoogleSheets for LockedBook<'_> {
    fn version<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
    ) -> SheetsFuture<'a, Result<String, GoogleError>> {
        Box::pin(async move {
            pace_book(
                self.books,
                self.limiter,
                &self.quiet,
                &self.credentials_ref,
                spreadsheet_id,
                Kind::Read,
                |book| book.version(spreadsheet_id),
            )
            .await
        })
    }

    fn metadata<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
    ) -> SheetsFuture<'a, Result<Vec<Meta>, GoogleError>> {
        Box::pin(async move {
            pace_book(
                self.books,
                self.limiter,
                &self.quiet,
                &self.credentials_ref,
                spreadsheet_id,
                Kind::Read,
                |book| book.metadata(spreadsheet_id, sheet_id),
            )
            .await
        })
    }

    fn batch_get<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        ranges: &'a [ColRange],
    ) -> SheetsFuture<'a, Result<BTreeMap<(u32, u32), BookCell>, GoogleError>> {
        Box::pin(async move {
            pace_book(
                self.books,
                self.limiter,
                &self.quiet,
                &self.credentials_ref,
                spreadsheet_id,
                Kind::Read,
                |book| book.batch_get(spreadsheet_id, sheet_id, ranges),
            )
            .await
        })
    }

    fn write_raw<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        runs: &'a [ValueRun],
    ) -> SheetsFuture<'a, Result<(), GoogleError>> {
        Box::pin(async move {
            pace_book(
                self.books,
                self.limiter,
                &self.quiet,
                &self.credentials_ref,
                spreadsheet_id,
                Kind::Write,
                |book| book.write_raw(spreadsheet_id, sheet_id, runs),
            )
            .await
        })
    }

    fn write_formula<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        row: u32,
        col: u32,
        formula: &'a str,
    ) -> SheetsFuture<'a, Result<(), GoogleError>> {
        Box::pin(async move {
            pace_book(
                self.books,
                self.limiter,
                &self.quiet,
                &self.credentials_ref,
                spreadsheet_id,
                Kind::Write,
                |book| book.write_formula(spreadsheet_id, sheet_id, row, col, formula),
            )
            .await
        })
    }

    fn set_meta_value<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        key: &'a str,
        value: &'a str,
    ) -> SheetsFuture<'a, Result<(), GoogleError>> {
        Box::pin(async move {
            pace_book(
                self.books,
                self.limiter,
                &self.quiet,
                &self.credentials_ref,
                spreadsheet_id,
                Kind::Write,
                |book| book.set_meta_value(spreadsheet_id, sheet_id, key, value),
            )
            .await
        })
    }

    fn ensure_service_column<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        key: &'a str,
        warning_only: bool,
    ) -> SheetsFuture<'a, Result<bool, GoogleError>> {
        Box::pin(async move {
            pace_book(
                self.books,
                self.limiter,
                &self.quiet,
                &self.credentials_ref,
                spreadsheet_id,
                Kind::Write,
                |book| book.ensure_service_column(spreadsheet_id, sheet_id, key, warning_only),
            )
            .await
        })
    }
}

async fn pace_book<T>(
    books: &Mutex<HashMap<String, FakeBook>>,
    limiter: &Limiter,
    quiet: &Quiet,
    credentials_ref: &str,
    spreadsheet_id: &str,
    kind: Kind,
    mut call: impl FnMut(&mut FakeBook) -> Result<T, GoogleError>,
) -> Result<T, GoogleError> {
    loop {
        match limiter.try_acquire(credentials_ref, kind, quiet.now()) {
            Ok(()) => {}
            Err(delay) => {
                quiet.wait(delay).await;
                continue;
            }
        }
        let result = {
            let mut books = books.lock().expect("books");
            let book = books.get_mut(spreadsheet_id).ok_or_else(|| {
                GoogleError::Message(format!("unknown spreadsheet {spreadsheet_id}"))
            })?;
            book.tokens = book.tokens.saturating_add(1);
            call(book)
        };
        match result {
            Err(GoogleError::RateLimited) => {
                let delay = limiter.penalize(credentials_ref, quiet.now());
                quiet.wait(delay).await;
            }
            Ok(value) => {
                limiter.succeed(credentials_ref);
                return Ok(value);
            }
            Err(err) => return Err(err),
        }
    }
}

async fn send_terminal(tx: &mpsc::Sender<Result<RowChange, Status>>, status: Status) {
    let _ = tokio::time::timeout(TERMINAL_WAIT, tx.send(Err(status))).await;
}

fn parse_wid(watch_id: &str) -> Result<u32, Status> {
    watch_id
        .parse()
        .map_err(|_| Status::not_found(format!("unknown watch {watch_id}")))
}

fn watch_message(wid: u32, record: &WatchRecord) -> Watch {
    Watch {
        watch_id: wid.to_string(),
        spreadsheet_id: record.spreadsheet_id.clone(),
        sheet_id: record.sheet_id,
        status: record.status.clone(),
        last_seq: record.last_seq,
        drive_version: record.drive_version.clone(),
        columns: record.columns.clone(),
    }
}

fn report(record: &WatchRecord, events: &[RowChange]) -> SyncReport {
    let mut added = 0;
    let mut updated = 0;
    let mut deleted = 0;
    for event in events {
        match ChangeKind::try_from(event.kind) {
            Ok(ChangeKind::Added) => added += 1,
            Ok(ChangeKind::Updated) => updated += 1,
            Ok(ChangeKind::Deleted) => deleted += 1,
            _ => {}
        }
    }
    SyncReport {
        last_seq: record.last_seq,
        added,
        updated,
        deleted,
    }
}
