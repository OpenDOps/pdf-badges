//! In-memory setup session. `ReadTable` stops after 50 empty rows. The watch does not.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tonic::Status;

use crate::columns::excel_id;
use crate::google::{
    FakeBook, GoogleError, Meta, KEY_COLUMN, KEY_EXCLUDE, KEY_FINGERPRINT, KEY_UID,
};
use crate::irbis::google_tables::v1::{
    CloseRequest, CloseResponse, ColumnOption, EnsureIdentityRequest,
    HeaderCandidate as ProtoCandidate, OpenSpreadsheetRequest, ReadTableRequest, RowBatch,
    SelectColumnsRequest, SelectHeaderRequest, SelectSheetRequest, SessionView,
    SetColumnOptionsRequest, SheetInfo,
};
use crate::irbis::table::v1::{value, Cell, Column as ProtoColumn, Row, Table, Value};
use crate::limiter::{Kind, Limiter};
use crate::model::{CellValue, Column, DataRow};
use crate::scan::{
    columns_for_header, columns_without_header, data_rows, header_candidates, rows_without_header,
    ScanError,
};
use crate::store::{Store, StoreError, RETAIN_EVENTS};
use crate::uid::{draw_uid, fingerprint_formula, formula_stamp, is_uid, FormulaColumn};
use crate::watch::Quiet;

const IDLE: Duration = Duration::from_secs(15 * 60);
const BATCH: usize = 100;

pub(crate) struct Session {
    pub(crate) spreadsheet_id: String,
    pub(crate) sheet_id: i32,
    pub(crate) header_row: Option<u32>,
    pub(crate) subcolumn_row: Option<u32>,
    pub(crate) columns: Vec<Column>,
    pub(crate) exclude: HashMap<String, bool>,
    pub(crate) credentials_ref: String,
    last_used: Instant,
}

pub(crate) struct Inner {
    pub(crate) sessions: Mutex<HashMap<String, Session>>,
    pub(crate) books: Mutex<HashMap<String, FakeBook>>,
    pub(crate) clock: Box<dyn Fn() -> Instant + Send + Sync>,
    pub(crate) store: Store,
    pub(crate) quiet: Arc<Quiet>,
    pub(crate) tasks: Mutex<HashMap<u32, tokio::task::JoinHandle<()>>>,
    pub(crate) stopped: Mutex<HashSet<u32>>,
    pub(crate) halt: AtomicBool,
    pub(crate) commits: Mutex<HashMap<u32, Arc<tokio::sync::Notify>>>,
    pub(crate) cycles: Mutex<HashMap<u32, Arc<tokio::sync::Mutex<()>>>>,
    pub(crate) retain_events: u64,
    pub(crate) subscribers_idle: AtomicUsize,
    pub(crate) limiter: Limiter,
}

#[derive(Clone)]
pub struct GoogleTablesService {
    pub(crate) inner: Arc<Inner>,
}

impl GoogleTablesService {
    pub fn new() -> Self {
        Self::with_clock(Instant::now)
    }

    pub fn with_clock(clock: impl Fn() -> Instant + Send + Sync + 'static) -> Self {
        Self::open_configured(temp_lmdb(), clock, false, RETAIN_EVENTS).expect("lmdb")
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_configured(
            path.as_ref().to_path_buf(),
            Instant::now,
            false,
            RETAIN_EVENTS,
        )
    }

    pub fn open_manual(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_configured(
            path.as_ref().to_path_buf(),
            Instant::now,
            true,
            RETAIN_EVENTS,
        )
    }

    pub fn open_manual_retain(
        path: impl AsRef<Path>,
        retain_events: u64,
    ) -> Result<Self, StoreError> {
        Self::open_configured(
            path.as_ref().to_path_buf(),
            Instant::now,
            true,
            retain_events,
        )
    }

    fn open_configured(
        path: PathBuf,
        clock: impl Fn() -> Instant + Send + Sync + 'static,
        manual_quiet: bool,
        retain_events: u64,
    ) -> Result<Self, StoreError> {
        let quiet = if manual_quiet {
            Quiet::manual()
        } else {
            Quiet::real()
        };
        Ok(Self {
            inner: Arc::new(Inner {
                sessions: Mutex::new(HashMap::new()),
                books: Mutex::new(HashMap::new()),
                clock: Box::new(clock),
                store: Store::open(&path)?,
                quiet,
                tasks: Mutex::new(HashMap::new()),
                stopped: Mutex::new(HashSet::new()),
                halt: AtomicBool::new(false),
                commits: Mutex::new(HashMap::new()),
                cycles: Mutex::new(HashMap::new()),
                retain_events,
                subscribers_idle: AtomicUsize::new(0),
                limiter: Limiter::new(),
            }),
        })
    }

    pub fn insert_book(&self, book: FakeBook) {
        let id = book.spreadsheet_id.clone();
        self.inner
            .books
            .lock()
            .expect("books")
            .insert(id.clone(), book);
        self.resume_spreadsheet(&id);
    }

    pub fn with_book<T>(
        &self,
        spreadsheet_id: &str,
        edit: impl FnOnce(&mut FakeBook) -> T,
    ) -> Option<T> {
        let mut books = self.inner.books.lock().expect("books");
        books.get_mut(spreadsheet_id).map(edit)
    }

    pub fn book<T>(&self, spreadsheet_id: &str, read: impl FnOnce(&FakeBook) -> T) -> Option<T> {
        let books = self.inner.books.lock().expect("books");
        books.get(spreadsheet_id).map(read)
    }

    pub fn open_spreadsheet(&self, request: OpenSpreadsheetRequest) -> Result<SessionView, Status> {
        if request.credentials_ref.contains('/') || request.credentials_ref.is_empty() {
            return Err(Status::invalid_argument("credentials_ref"));
        }
        let limit = if request.header_candidate_limit == 0 {
            10
        } else {
            request.header_candidate_limit as usize
        };
        let books = self.inner.books.lock().expect("books");
        let book = books
            .get(&request.spreadsheet_id)
            .ok_or_else(|| Status::not_found("unknown spreadsheet"))?;
        let Some(tab) = book.tabs.first() else {
            return Err(Status::not_found("unknown sheet"));
        };
        let grid = tab.grid();
        let candidates = header_candidates(&grid, limit);
        let sheet_id = tab.sheet_id;
        let sheets = book
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| SheetInfo {
                name: tab.title.clone(),
                index: index as u32,
                sheet_id: tab.sheet_id,
            })
            .collect();
        drop(books);
        let session_id =
            draw_uid(&HashSet::new()).map_err(|err| Status::internal(err.to_string()))?;
        self.inner.sessions.lock().expect("sessions").insert(
            session_id.clone(),
            Session {
                spreadsheet_id: request.spreadsheet_id,
                sheet_id,
                header_row: None,
                subcolumn_row: None,
                columns: Vec::new(),
                exclude: HashMap::new(),
                credentials_ref: request.credentials_ref,
                last_used: (self.inner.clock)(),
            },
        );
        Ok(SessionView {
            session_id,
            sheets,
            selected_sheet_index: 0,
            selected_sheet_id: sheet_id,
            header_candidates: proto_candidates(&candidates),
        })
    }

    pub fn select_sheet(&self, request: SelectSheetRequest) -> Result<SessionView, Status> {
        let mut session = self.session(&request.session_id)?;
        self.editable(&session.spreadsheet_id, request.sheet_id)?;
        let limit = if request.header_candidate_limit == 0 {
            10
        } else {
            request.header_candidate_limit as usize
        };
        let books = self.inner.books.lock().expect("books");
        let book = books
            .get(&session.spreadsheet_id)
            .ok_or_else(|| Status::not_found("unknown spreadsheet"))?;
        let tab = book
            .tab(request.sheet_id)
            .ok_or_else(|| Status::not_found("unknown sheet"))?;
        let grid = tab.grid();
        let candidates = header_candidates(&grid, limit);
        let sheets = book
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| SheetInfo {
                name: tab.title.clone(),
                index: index as u32,
                sheet_id: tab.sheet_id,
            })
            .collect();
        let index = book
            .tabs
            .iter()
            .position(|tab| tab.sheet_id == request.sheet_id)
            .unwrap_or(0) as u32;
        drop(books);
        session.sheet_id = request.sheet_id;
        session.header_row = None;
        session.subcolumn_row = None;
        session.columns.clear();
        let view = SessionView {
            session_id: request.session_id.clone(),
            sheets,
            selected_sheet_index: index,
            selected_sheet_id: request.sheet_id,
            header_candidates: proto_candidates(&candidates),
        };
        self.store(request.session_id, session);
        Ok(view)
    }

    pub fn select_header(&self, request: SelectHeaderRequest) -> Result<Table, Status> {
        let mut session = self.session(&request.session_id)?;
        self.editable(&session.spreadsheet_id, session.sheet_id)?;
        let grid = self.grid(&session)?;
        let candidates = header_candidates(&grid, 10);
        let subcolumn = if request.subcolumn_row == 0 {
            None
        } else {
            Some(request.subcolumn_row)
        };
        let columns = columns_for_header(&grid, request.excel_row, &candidates, subcolumn)
            .map_err(scan_status)?;
        session.header_row = Some(request.excel_row);
        session.subcolumn_row = subcolumn;
        session.columns = columns;
        let table = table_of(&session.columns);
        self.store(request.session_id, session);
        Ok(table)
    }

    pub fn select_columns(&self, request: SelectColumnsRequest) -> Result<Table, Status> {
        let mut session = self.session(&request.session_id)?;
        self.editable(&session.spreadsheet_id, session.sheet_id)?;
        let specs: Vec<(String, String)> = request
            .columns
            .iter()
            .map(|column| (column.excel_id.clone(), column.name.clone()))
            .collect();
        let refs: Vec<(&str, &str)> = specs
            .iter()
            .map(|(excel_id, name)| (excel_id.as_str(), name.as_str()))
            .collect();
        let columns = columns_without_header(&refs).map_err(scan_status)?;
        session.header_row = None;
        session.subcolumn_row = None;
        session.columns = columns;
        let table = table_of(&session.columns);
        self.store(request.session_id, session);
        Ok(table)
    }

    pub fn set_column_options(&self, request: SetColumnOptionsRequest) -> Result<Table, Status> {
        let mut session = self.session(&request.session_id)?;
        self.editable(&session.spreadsheet_id, session.sheet_id)?;
        apply_options(&mut session, &request.options);
        self.rewrite_formula(&session)?;
        let table = table_of(&session.columns);
        self.store(request.session_id, session);
        Ok(table)
    }

    pub fn ensure_identity(&self, request: EnsureIdentityRequest) -> Result<Table, Status> {
        let session = self.session(&request.session_id)?;
        self.editable(&session.spreadsheet_id, session.sheet_id)?;
        if session.columns.is_empty() {
            return Err(Status::failed_precondition("columns"));
        }
        let limiter = &self.inner.limiter;
        {
            let mut books = self.inner.books.lock().expect("books");
            let book = books
                .get_mut(&session.spreadsheet_id)
                .ok_or_else(|| Status::not_found("unknown spreadsheet"))?;
            let tab = book
                .tab_mut(session.sheet_id)
                .ok_or_else(|| Status::not_found("unknown sheet"))?;
            if column_of(&tab.metadata, KEY_UID).is_none() {
                let col = tab.append_hidden();
                tab.metadata.push(Meta {
                    key: KEY_UID.into(),
                    column: col,
                    value: String::new(),
                });
                tab.warning_only.insert(col);
            }
            if column_of(&tab.metadata, KEY_FINGERPRINT).is_none() {
                let col = tab.append_hidden();
                tab.metadata.push(Meta {
                    key: KEY_FINGERPRINT.into(),
                    column: col,
                    value: String::new(),
                });
            }
            tab.metadata
                .retain(|item| item.key != KEY_COLUMN && item.key != KEY_EXCLUDE);
            for column in &session.columns {
                tab.metadata.push(Meta {
                    key: KEY_COLUMN.into(),
                    column: column.index,
                    value: column.name.clone(),
                });
                if session.exclude.get(&column.name).copied().unwrap_or(false) {
                    tab.metadata.push(Meta {
                        key: KEY_EXCLUDE.into(),
                        column: column.index,
                        value: "1".into(),
                    });
                }
            }
            let formula = formula_for(&session, tab);
            let fp_col = column_of(&tab.metadata, KEY_FINGERPRINT).expect("fingerprint column");
            let sheet_id = session.sheet_id;
            let spreadsheet_id = session.spreadsheet_id.clone();
            let _ = tab;
            limiter
                .try_acquire(&session.credentials_ref, Kind::Write, Instant::now())
                .map_err(|_| Status::resource_exhausted("rate limit"))?;
            book.tokens = book.tokens.saturating_add(1);
            match book.write_formula(&spreadsheet_id, sheet_id, 1, fp_col, &formula) {
                Ok(()) => {
                    limiter.succeed(&session.credentials_ref);
                    if let Some(tab) = book.tab_mut(sheet_id) {
                        stamp_fingerprint(tab, &formula);
                    }
                }
                Err(GoogleError::RateLimited) => {
                    limiter.penalize(&session.credentials_ref, Instant::now());
                    return Err(Status::resource_exhausted("rate limit"));
                }
                Err(err) => return Err(Status::internal(err.to_string())),
            }
        }
        let table = table_of(&session.columns);
        self.store(request.session_id, session);
        Ok(table)
    }

    pub fn read_table(&self, request: ReadTableRequest) -> Result<Vec<RowBatch>, Status> {
        let session = self.session(&request.session_id)?;
        self.editable(&session.spreadsheet_id, session.sheet_id)?;
        if session.columns.is_empty() {
            return Err(Status::failed_precondition("columns"));
        }
        let grid = self.grid(&session)?;
        let data = match session.header_row {
            Some(header) => data_rows(&grid, header, &session.columns, session.subcolumn_row),
            None => rows_without_header(&grid, &session.columns),
        };
        let limiter = &self.inner.limiter;
        let rows = {
            let mut books = self.inner.books.lock().expect("books");
            let book = books
                .get_mut(&session.spreadsheet_id)
                .ok_or_else(|| Status::not_found("unknown spreadsheet"))?;
            let (uid_col, currents, mut taken) = {
                let tab = book
                    .tab(session.sheet_id)
                    .ok_or_else(|| Status::not_found("unknown sheet"))?;
                let uid_col = column_of(&tab.metadata, KEY_UID);
                let mut taken = HashSet::new();
                if let Some(col) = uid_col {
                    for ((_, cell_col), cell) in &tab.cells {
                        if *cell_col == col {
                            if let Some(CellValue::Text(text)) = &cell.value {
                                if is_uid(text) {
                                    taken.insert(text.clone());
                                }
                            }
                        }
                    }
                }
                let currents = data
                    .iter()
                    .map(|row| {
                        uid_col.and_then(|col| {
                            tab.cells.get(&(row.excel_row, col)).and_then(|cell| {
                                cell.value.as_ref().and_then(|value| match value {
                                    CellValue::Text(text) => Some(text.clone()),
                                    _ => None,
                                })
                            })
                        })
                    })
                    .collect::<Vec<_>>();
                (uid_col, currents, taken)
            };
            let mut writes = Vec::new();
            let mut uids = Vec::new();
            for (row, current) in data.iter().zip(currents) {
                let uid = match current {
                    Some(text) if is_uid(&text) => text,
                    _ => {
                        let fresh =
                            draw_uid(&taken).map_err(|err| Status::internal(err.to_string()))?;
                        taken.insert(fresh.clone());
                        if let Some(col) = uid_col {
                            writes.push((row.excel_row, col, fresh.clone()));
                        }
                        fresh
                    }
                };
                uids.push(uid);
            }
            let sheet_id = session.sheet_id;
            let spreadsheet_id = session.spreadsheet_id.clone();
            if !writes.is_empty() {
                limiter
                    .try_acquire(&session.credentials_ref, Kind::Write, Instant::now())
                    .map_err(|_| Status::resource_exhausted("rate limit"))?;
                let runs = crate::google::value_runs(&writes);
                book.tokens = book.tokens.saturating_add(1);
                match book.write_raw(&spreadsheet_id, sheet_id, &runs) {
                    Ok(()) => limiter.succeed(&session.credentials_ref),
                    Err(GoogleError::RateLimited) => {
                        limiter.penalize(&session.credentials_ref, Instant::now());
                        return Err(Status::resource_exhausted("rate limit"));
                    }
                    Err(err) => return Err(Status::internal(err.to_string())),
                }
            }
            let uid_letter = uid_col.map(excel_id);
            data.iter()
                .zip(uids)
                .map(|(row, uid)| proto_row(row, uid_letter.as_deref(), &uid))
                .collect::<Vec<_>>()
        };
        self.store(request.session_id, session);
        Ok(rows
            .chunks(BATCH)
            .map(|chunk| RowBatch {
                rows: chunk.to_vec(),
            })
            .collect())
    }

    pub fn close(&self, request: CloseRequest) -> Result<CloseResponse, Status> {
        self.session(&request.session_id)?;
        self.inner
            .sessions
            .lock()
            .expect("sessions")
            .remove(&request.session_id);
        Ok(CloseResponse {})
    }

    fn rewrite_formula(&self, session: &Session) -> Result<(), Status> {
        let limiter = &self.inner.limiter;
        let mut books = self.inner.books.lock().expect("books");
        let Some(book) = books.get_mut(&session.spreadsheet_id) else {
            return Ok(());
        };
        let prepared = {
            let Some(tab) = book.tab(session.sheet_id) else {
                return Ok(());
            };
            let Some(fp_col) = column_of(&tab.metadata, KEY_FINGERPRINT) else {
                return Ok(());
            };
            (
                fp_col,
                formula_for(session, tab),
                session.sheet_id,
                session.spreadsheet_id.clone(),
            )
        };
        limiter
            .try_acquire(&session.credentials_ref, Kind::Write, Instant::now())
            .map_err(|_| Status::resource_exhausted("rate limit"))?;
        book.tokens = book.tokens.saturating_add(1);
        match book.write_formula(&prepared.3, prepared.2, 1, prepared.0, &prepared.1) {
            Ok(()) => {
                limiter.succeed(&session.credentials_ref);
                if let Some(tab) = book.tab_mut(prepared.2) {
                    stamp_fingerprint(tab, &prepared.1);
                }
            }
            Err(GoogleError::RateLimited) => {
                limiter.penalize(&session.credentials_ref, Instant::now());
                return Err(Status::resource_exhausted("rate limit"));
            }
            Err(err) => return Err(Status::internal(err.to_string())),
        }
        Ok(())
    }

    fn grid(&self, session: &Session) -> Result<crate::model::Grid, Status> {
        let books = self.inner.books.lock().expect("books");
        let book = books
            .get(&session.spreadsheet_id)
            .ok_or_else(|| Status::not_found("unknown spreadsheet"))?;
        let tab = book
            .tab(session.sheet_id)
            .ok_or_else(|| Status::not_found("unknown sheet"))?;
        Ok(tab.grid())
    }

    pub(crate) fn session(&self, id: &str) -> Result<Session, Status> {
        let mut sessions = self.inner.sessions.lock().expect("sessions");
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| Status::not_found("unknown session"))?;
        let now = (self.inner.clock)();
        if now.saturating_duration_since(session.last_used) >= IDLE {
            sessions.remove(id);
            return Err(Status::not_found("unknown session"));
        }
        session.last_used = now;
        Ok(clone_session(session))
    }

    fn store(&self, id: String, session: Session) {
        self.inner
            .sessions
            .lock()
            .expect("sessions")
            .insert(id, session);
    }

    fn editable(&self, spreadsheet_id: &str, sheet_id: i32) -> Result<(), Status> {
        let tasks = self.inner.tasks.lock().expect("tasks");
        for wid in tasks.keys() {
            let Some(record) = self
                .inner
                .store
                .load_saved(*wid)
                .map_err(|err| Status::internal(err.to_string()))?
            else {
                continue;
            };
            if record.spreadsheet_id == spreadsheet_id && record.sheet_id == sheet_id {
                return Err(Status::failed_precondition("sheet is watched"));
            }
        }
        Ok(())
    }
}

fn clone_session(session: &Session) -> Session {
    Session {
        spreadsheet_id: session.spreadsheet_id.clone(),
        sheet_id: session.sheet_id,
        header_row: session.header_row,
        subcolumn_row: session.subcolumn_row,
        columns: session.columns.clone(),
        exclude: session.exclude.clone(),
        credentials_ref: session.credentials_ref.clone(),
        last_used: session.last_used,
    }
}

fn apply_options(session: &mut Session, options: &[ColumnOption]) {
    for option in options {
        if session
            .columns
            .iter()
            .any(|column| column.name == option.name)
        {
            session
                .exclude
                .insert(option.name.clone(), option.exclude_from_hash);
        }
    }
}

fn stamp_fingerprint(tab: &mut crate::google::SheetTab, formula: &str) {
    if let Some(item) = tab
        .metadata
        .iter_mut()
        .find(|item| item.key == KEY_FINGERPRINT)
    {
        item.value = formula_stamp(formula);
    }
}

fn formula_for(session: &Session, tab: &crate::google::SheetTab) -> String {
    let start = session
        .subcolumn_row
        .or(session.header_row)
        .map(|row| row + 1)
        .unwrap_or(tab.grid().first_row);
    let letters: Vec<String> = session
        .columns
        .iter()
        .map(|column| column.excel_id.clone())
        .collect();
    let inputs: Vec<FormulaColumn<'_>> = session
        .columns
        .iter()
        .zip(letters.iter())
        .map(|(column, letter)| FormulaColumn {
            excel_id: letter,
            name: &column.name,
            exclude_from_hash: session.exclude.get(&column.name).copied().unwrap_or(false),
        })
        .collect();
    fingerprint_formula(&inputs, start)
}

fn column_of(metadata: &[Meta], key: &str) -> Option<u32> {
    metadata
        .iter()
        .find(|item| item.key == key)
        .map(|item| item.column)
}

fn proto_candidates(candidates: &[crate::model::Candidate]) -> Vec<ProtoCandidate> {
    candidates
        .iter()
        .map(|candidate| ProtoCandidate {
            excel_row: candidate.excel_row,
            columns: candidate.columns.iter().map(proto_column).collect(),
        })
        .collect()
}

fn table_of(columns: &[Column]) -> Table {
    Table {
        columns: columns.iter().map(proto_column).collect(),
        rows: Vec::new(),
    }
}

pub(crate) fn proto_column(column: &Column) -> ProtoColumn {
    ProtoColumn {
        excel_id: column.excel_id.clone(),
        name: column.name.clone(),
        subcolumns: column.subcolumns.iter().map(proto_column).collect(),
    }
}

fn proto_row(row: &DataRow, uid_letter: Option<&str>, uid: &str) -> Row {
    let mut cells = row
        .cells
        .iter()
        .map(|cell| Cell {
            excel_id: excel_id(cell.col),
            value: cell.value.as_ref().map(proto_value),
        })
        .collect::<Vec<_>>();
    if let Some(letter) = uid_letter {
        cells.push(Cell {
            excel_id: letter.into(),
            value: Some(Value {
                kind: Some(value::Kind::Text(uid.into())),
            }),
        });
    }
    Row {
        excel_row: row.excel_row,
        cells,
    }
}

fn proto_value(value: &CellValue) -> Value {
    Value {
        kind: Some(match value {
            CellValue::Text(text) => value::Kind::Text(text.clone()),
            CellValue::Float(number) => value::Kind::FloatValue(*number),
            CellValue::Int(number) => value::Kind::IntValue(*number),
            CellValue::Bool(flag) => value::Kind::BoolValue(*flag),
            CellValue::Date(date) => value::Kind::Date(crate::irbis::table::v1::DateTimeValue {
                year: date.year,
                month: date.month as i32,
                day: date.day as i32,
                hour: date.hour as i32,
                minute: date.minute as i32,
                second: date.second as i32,
                has_time: date.has_time,
            }),
        }),
    }
}

fn scan_status(err: ScanError) -> Status {
    Status::invalid_argument(err.to_string())
}

fn temp_lmdb() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let root = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = root.join(format!("google-tables-svc-{n}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok();
    dir.join("google-tables.lmdb")
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::DuplicateColumn(name) => write!(f, "duplicate column {name}"),
            ScanError::HeaderNotCandidate(row) => write!(f, "header {row} is not a candidate"),
            ScanError::SubcolumnRowOutOfRange(row) => write!(f, "subcolumn row {row}"),
            ScanError::BadColumn(detail) => write!(f, "{detail}"),
        }
    }
}
