//! Event SQLite file and the memory copy of the registration list.
//!
//! The file is `{base}/{event_id}/db.sqlite`. Search and the list read a
//! private `mode=memory&cache=shared` database. `open` creates both. A file
//! that is not a usable database is moved to
//! `{event_id}/broken/{index}-{unix_millis}/` and an empty database is created.
//! That event id is appended to `full_reload` in `{base}/base.config`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection, OpenFlags, OptionalExtension};
use serde_yaml::{Mapping, Value};

/// `{base}/base.config`. Lists event ids whose database file was replaced.
pub const CONFIG_FILE: &str = "base.config";

/// File name inside `{base}/{event_id}/`.
pub const DB_FILE: &str = "db.sqlite";

const CONFIG_VERSION: u64 = 1;
const KEY_ALPHABET: &str = "0123456789ABCDEFGHIKLMNOPQRSTVXYZ";

static MEMORY_SEQ: AtomicU64 = AtomicU64::new(1);

const REQUIRED_TABLES: &[&str] = &[
    "expo_days",
    "mem_u",
    "prints",
    "barcodes",
    "ticketbarcodes",
    "u_fulltext",
    "keys",
    "scans",
    "phones",
    "emails",
    "scanner_ids",
    "print_stat",
    "lottery_wins",
    "man_to_user",
    "log",
];

/// Scala event tables, including `mem_u.new_id`. No full-text virtual table.
const FILE_TABLES: &str = "
CREATE TABLE IF NOT EXISTS expo_days (day INTEGER);

CREATE TABLE IF NOT EXISTS mem_u (
  uid TEXT,
  data TEXT,
  code TEXT,
  patronymic TEXT,
  hall_num TEXT,
  booth_id TEXT,
  in_synch INTEGER,
  synch_skipped INTEGER,
  ts INTEGER,
  barcode TEXT,
  repl_barcode TEXT,
  cert_code TEXT,
  category INTEGER,
  cat_priority INTEGER,
  is_import INTEGER,
  errors TEXT,
  internet INTEGER,
  ticket_status INTEGER,
  c_position TEXT,
  org_id INTEGER,
  new_id INTEGER
);

CREATE INDEX IF NOT EXISTS mem_u_uid_index ON mem_u (uid);
CREATE INDEX IF NOT EXISTS mem_u_org_id_index ON mem_u (org_id);

CREATE TABLE IF NOT EXISTS prints (
  uid TEXT,
  ts INTEGER,
  category INTEGER,
  is_cert INTEGER
);

CREATE TABLE IF NOT EXISTS barcodes (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  barcode TEXT,
  type INTEGER,
  clean_id INTEGER,
  repl INTEGER,
  date TEXT
);
CREATE INDEX IF NOT EXISTS type_index ON barcodes (type);
CREATE INDEX IF NOT EXISTS type_repl_index ON barcodes (type, repl);

CREATE TABLE IF NOT EXISTS ticketbarcodes (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  barcode TEXT,
  formId INTEGER
);
CREATE INDEX IF NOT EXISTS ticket_bc_form_index ON ticketbarcodes (formId);
CREATE INDEX IF NOT EXISTS ticket_bc_bc_index ON ticketbarcodes (barcode);

CREATE TABLE IF NOT EXISTS u_fulltext (
  uid TEXT,
  name TEXT,
  surname TEXT,
  c_name TEXT,
  category TEXT,
  ticket_status TEXT,
  gotsome TEXT,
  give_packet TEXT
);

CREATE TABLE IF NOT EXISTS keys (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  key TEXT NOT NULL UNIQUE,
  is_admin INTEGER,
  comment TEXT
);

CREATE TABLE IF NOT EXISTS scans (
  scanid INTEGER PRIMARY KEY,
  zone_id INTEGER,
  entrence_type INTEGER,
  barcode TEXT,
  time INTEGER,
  synched INTEGER
);
CREATE INDEX IF NOT EXISTS scans_zone_id_index ON scans (zone_id);

CREATE TABLE IF NOT EXISTS phones (uid INTEGER, phone TEXT);
CREATE TABLE IF NOT EXISTS emails (uid INTEGER, email TEXT);

CREATE TABLE IF NOT EXISTS scanner_ids (
  id TEXT,
  num INTEGER PRIMARY KEY AUTOINCREMENT
);
CREATE INDEX IF NOT EXISTS scanner_ids_index ON scanner_ids (id);

CREATE TABLE IF NOT EXISTS print_stat (
  date TEXT,
  category TEXT,
  prints INTEGER,
  prints_uniq INTEGER,
  is_cert INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS print_stat_index ON print_stat (date, category, is_cert);

CREATE TABLE IF NOT EXISTS lottery_wins (
  packet_id INTEGER,
  form INTEGER,
  wins INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS lottery_wins_index ON lottery_wins (packet_id, form);

CREATE TABLE IF NOT EXISTS man_to_user (
  man_id INTEGER,
  uid INTEGER,
  comment TEXT,
  zone INTEGER,
  time INTEGER,
  synched INTEGER
);
CREATE INDEX IF NOT EXISTS man_to_user_m_index ON man_to_user (man_id);
CREATE INDEX IF NOT EXISTS man_to_user_u_index ON man_to_user (uid);

CREATE TABLE IF NOT EXISTS log (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  key TEXT,
  time INTEGER,
  uid TEXT,
  action TEXT,
  diff TEXT,
  note TEXT
);

CREATE TABLE IF NOT EXISTS sync_cursor (
  name TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
";

/// Indexes the Scala file does not have. `open` adds them when they are missing.
const EXTRA_INDEXES: &str = "
CREATE INDEX IF NOT EXISTS mem_u_code_index ON mem_u (code);
CREATE INDEX IF NOT EXISTS mem_u_barcode_index ON mem_u (barcode);
CREATE INDEX IF NOT EXISTS mem_u_repl_barcode_index ON mem_u (repl_barcode);
CREATE INDEX IF NOT EXISTS mem_u_in_synch_index ON mem_u (in_synch);
";

const MEMORY_SCHEMA: &str = "
CREATE TABLE list_row (
  uid TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  surname TEXT NOT NULL,
  c_name TEXT NOT NULL,
  category INTEGER NOT NULL,
  ticket_status INTEGER NOT NULL,
  gotsome INTEGER NOT NULL,
  give_packet INTEGER NOT NULL,
  added_ts INTEGER NOT NULL,
  last_print_ts INTEGER NOT NULL,
  print_count INTEGER NOT NULL,
  org_id INTEGER,
  email TEXT NOT NULL
);
CREATE INDEX list_row_category ON list_row(category);
CREATE INDEX list_row_added ON list_row(added_ts);
CREATE INDEX list_row_printed ON list_row(last_print_ts);
CREATE VIRTUAL TABLE u_fts USING fts5(
  name,
  surname,
  c_name,
  content='list_row',
  content_rowid='rowid',
  tokenize='unicode61'
);
";

const FIRST_BATCH: usize = 100;
const NEXT_BATCH: usize = 200;
const DEFAULT_PAGE: u64 = 20;

const LIST_COLUMNS: &str = "uid, name, surname, c_name, category, ticket_status, gotsome, give_packet, added_ts, last_print_ts, print_count, org_id, email";

const LOAD_FROM: &str = "
SELECT
  mem_u.uid AS uid,
  IFNULL(u_fulltext.name, '') AS name,
  IFNULL(u_fulltext.surname, '') AS surname,
  IFNULL(u_fulltext.c_name, '') AS c_name,
  IFNULL(mem_u.category, 0) AS category,
  IFNULL(mem_u.ticket_status, 0) AS ticket_status,
  CASE
    WHEN u_fulltext.gotsome IS NULL OR TRIM(u_fulltext.gotsome) = '' THEN 0
    ELSE CAST(u_fulltext.gotsome AS INTEGER)
  END AS gotsome,
  CASE
    WHEN u_fulltext.give_packet IS NULL OR TRIM(u_fulltext.give_packet) = '' THEN 0
    ELSE CAST(u_fulltext.give_packet AS INTEGER)
  END AS give_packet,
  IFNULL(mem_u.ts, 0) AS added_ts,
  IFNULL(p.last_print_ts, 0) AS last_print_ts,
  IFNULL(p.print_count, 0) AS print_count,
  mem_u.org_id AS org_id,
  IFNULL(e.email, '') AS email
FROM mem_u
LEFT JOIN u_fulltext ON u_fulltext.uid = mem_u.uid
LEFT JOIN (
  SELECT uid, MAX(ts) AS last_print_ts, COUNT(*) AS print_count
  FROM prints
  GROUP BY uid
) p ON p.uid = mem_u.uid
LEFT JOIN (
  SELECT uid, MIN(email) AS email
  FROM emails
  WHERE email IS NOT NULL AND TRIM(email) != ''
  GROUP BY uid
) e ON e.uid = CAST(mem_u.uid AS INTEGER)
";

/// Sort of a search page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListOrder {
    /// Newest `added_ts` first, then `uid` ascending.
    LastAdd,
    /// Newest `last_print_ts` first. A visitor who was never printed sorts last.
    LastPrint,
}

/// One committed `list_row`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRow {
    pub uid: String,
    pub name: String,
    pub surname: String,
    pub c_name: String,
    pub category: i64,
    pub ticket_status: i64,
    pub gotsome: i64,
    pub give_packet: i64,
    pub added_ts: i64,
    pub last_print_ts: i64,
    pub print_count: i64,
    pub org_id: Option<i64>,
    pub email: String,
}

/// `search` count plus the requested page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPage {
    pub total: u64,
    pub rows: Vec<ListRow>,
}

/// One scan row written into `scans`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanWrite {
    pub scanid: i64,
    pub zone_id: i64,
    pub entrence_type: i64,
    pub barcode: String,
    pub time: i64,
}

/// One `mem_u` row whose `in_synch` is still unset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingRegistration {
    pub uid: String,
    pub data: String,
    pub ts: i64,
    pub barcode: String,
    pub org_id: Option<i64>,
    pub category: Option<i64>,
    pub ticket_status: Option<i64>,
}

/// One scan whose `synched` is unset and whose registration is already on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingScan {
    pub scanid: i64,
    pub zone_id: i64,
    pub entrence_type: i64,
    pub barcode: String,
    pub time: i64,
}

/// One attachment whose `synched` is unset and whose registration is already on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingAttachment {
    pub man_id: i64,
    pub uid: i64,
    pub comment: String,
    pub zone: i64,
    pub time: i64,
}

/// One visitor written to the file and then to the memory list.
///
/// `last_print_ts` and `print_count` are not fields. An update keeps the
/// values already in memory. A new uid starts them at `0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisitorWrite {
    pub uid: String,
    pub data: String,
    pub name: String,
    pub surname: String,
    pub c_name: String,
    pub category: i64,
    pub ticket_status: i64,
    pub gotsome: i64,
    pub give_packet: i64,
    pub added_ts: i64,
    pub org_id: Option<i64>,
    pub email: String,
}

/// Whether the memory copy has caught up with the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexState {
    /// Visitor rows are still being copied into memory.
    Loading,
    /// The file has no visitors, or every visitor is in memory.
    Ready,
}

/// One open event: the flash file and its memory database.
pub struct EventDb {
    base: PathBuf,
    event_id: String,
    file: Connection,
    memory: Connection,
    memory_uri: String,
    load: LoadCursor,
}

impl EventDb {
    /// Open `{base}/{event_id}/db.sqlite` and a new memory database.
    ///
    /// Creates the file when it is missing. A broken file, or a leftover
    /// `db.sqlite.creating` from a reboot during create, is moved to
    /// `broken/{index}-{unix_millis}/` and an empty database is created.
    /// That event id is then listed in `{base}/base.config`.
    pub fn open(base: impl Into<PathBuf>, event_id: &str) -> Result<Self, EventDbError> {
        let base = base.into();
        let event_id = normalize_event_id(event_id)?;
        let file = prepare_file(&base, &event_id)?;
        let (memory, memory_uri) = open_memory()?;
        let load = LoadCursor::from_file(&file, &event_dir(&base, &event_id).join(DB_FILE))?;
        Ok(Self {
            base,
            event_id,
            file,
            memory,
            memory_uri,
            load,
        })
    }

    /// Open `event_id` in this base directory.
    ///
    /// The next file is opened before the previous connections are dropped.
    /// If that open fails, this event stays as it was.
    pub fn switch(&mut self, event_id: &str) -> Result<(), EventDbError> {
        let event_id = normalize_event_id(event_id)?;
        if event_id == self.event_id {
            return Ok(());
        }
        let next = Self::open(self.base.clone(), &event_id)?;
        *self = next;
        Ok(())
    }

    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    pub fn file_path(&self) -> PathBuf {
        event_dir(&self.base, &self.event_id).join(DB_FILE)
    }

    /// Shared-cache URI for this open. Another connection with the same URI
    /// sees these pages. A different open uses a different name.
    pub fn memory_uri(&self) -> &str {
        &self.memory_uri
    }

    pub fn file(&self) -> &Connection {
        &self.file
    }

    pub fn memory(&self) -> &Connection {
        &self.memory
    }

    /// `ready` when the file has no visitors, or `load_batch` has copied them.
    /// A file that already has visitors stays `loading` until that copy finishes.
    pub fn index_state(&self) -> Result<IndexState, EventDbError> {
        if self.load.done {
            Ok(IndexState::Ready)
        } else {
            Ok(IndexState::Loading)
        }
    }

    /// Copy the next page of visitors from the file into `list_row` and `u_fts`.
    ///
    /// The first call inserts 100 rows, or fewer when the file is smaller, in the
    /// table `last_add` order (`mem_u.ts` descending, then `uid` ascending).
    /// That is the first five pages. Later calls continue in that order.
    /// A uid already in `list_row` is skipped.
    /// Returns `true` when more visitors remain. The sync thread calls this;
    /// tests call it on the calling thread so the first commit is visible.
    pub fn load_batch(&mut self) -> Result<bool, EventDbError> {
        if self.load.done {
            return Ok(false);
        }
        let path = self.file_path();
        let target = if self.load.batches == 0 {
            FIRST_BATCH
        } else {
            NEXT_BATCH
        };
        let tx = self
            .memory
            .unchecked_transaction()
            .map_err(|err| sqlite_at(Path::new(self.memory_uri()), err))?;
        let mut inserted = 0usize;
        let mut exhausted = false;
        let mut after_ts = self.load.after_ts;
        let mut after_uid = self.load.after_uid.clone();
        while inserted < target {
            let want = target - inserted;
            let rows = fetch_visitors(&self.file, &path, after_ts, after_uid.as_deref(), want)?;
            if rows.is_empty() {
                exhausted = true;
                break;
            }
            let short = rows.len() < want;
            let mut stopped_early = false;
            for (index, row) in rows.iter().enumerate() {
                after_ts = Some(row.added_ts);
                after_uid = Some(row.uid.clone());
                if list_has_uid(&tx, &row.uid).map_err(|err| sqlite_at(&path, err))? {
                    continue;
                }
                insert_listed(&tx, row).map_err(|err| sqlite_at(&path, err))?;
                inserted += 1;
                if inserted == target && index + 1 < rows.len() {
                    stopped_early = true;
                    break;
                }
            }
            if short && !stopped_early {
                exhausted = true;
                break;
            }
        }
        if exhausted {
            tx.execute_batch("INSERT INTO u_fts(u_fts) VALUES('optimize')")
                .map_err(|err| sqlite_at(Path::new(self.memory_uri()), err))?;
        }
        tx.commit()
            .map_err(|err| sqlite_at(Path::new(self.memory_uri()), err))?;
        self.load.after_ts = after_ts;
        self.load.after_uid = after_uid;
        self.load.batches += 1;
        self.load.done = exhausted;
        Ok(!exhausted)
    }

    /// Rows committed to `list_row`. An empty file is `0`.
    pub fn loaded_count(&self) -> Result<u64, EventDbError> {
        let count: i64 = self
            .memory
            .query_row("SELECT COUNT(*) FROM list_row", [], |row| row.get(0))
            .map_err(|err| sqlite_at(Path::new(self.memory_uri()), err))?;
        Ok(count as u64)
    }

    /// True when `base.config` lists at least one event whose file was deleted.
    pub fn needs_full_reload(&self) -> Result<bool, EventDbError> {
        let path = self.base.join(CONFIG_FILE);
        match read_config(&path)? {
            None => Ok(false),
            Some(map) => Ok(!reload_ids(&path, &map)?.is_empty()),
        }
    }

    /// Search the committed `list_row` rows.
    ///
    /// `limit` defaults to 20 and `offset` to 0. Empty `text` does not query
    /// `u_fts`. Each word is a quoted FTS5 phrase bound as a parameter.
    pub fn search(
        &self,
        text: &str,
        categories: &[i64],
        order: ListOrder,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<SearchPage, EventDbError> {
        let limit = limit.unwrap_or(DEFAULT_PAGE).min(i64::MAX as u64) as i64;
        let offset = offset.unwrap_or(0).min(i64::MAX as u64) as i64;
        let matched = match match_query(text) {
            TextQuery::All => None,
            TextQuery::None => {
                return Ok(SearchPage {
                    total: 0,
                    rows: Vec::new(),
                });
            }
            TextQuery::Match(expr) => Some(expr),
        };
        let path = Path::new(self.memory_uri());
        let total = count_listed(&self.memory, path, matched.as_deref(), categories)?;
        let rows = if total == 0 || offset as u64 >= total {
            Vec::new()
        } else {
            select_listed(
                &self.memory,
                path,
                matched.as_deref(),
                categories,
                order,
                limit,
                offset,
            )?
        };
        Ok(SearchPage { total, rows })
    }

    /// Store one visitor on the file, then in `list_row` and `u_fts`.
    ///
    /// The file transaction commits before memory is touched. A uid the loader
    /// has not copied yet is searchable immediately, and a later batch skips it.
    pub fn write(&mut self, visitor: &VisitorWrite) -> Result<(), EventDbError> {
        let path = self.file_path();
        let tx = self
            .file
            .unchecked_transaction()
            .map_err(|err| sqlite_at(&path, err))?;
        upsert_visitor_file(&tx, visitor).map_err(|err| sqlite_at(&path, err))?;
        tx.commit().map_err(|err| sqlite_at(&path, err))?;

        let memory_path = Path::new(self.memory_uri());
        let tx = self
            .memory
            .unchecked_transaction()
            .map_err(|err| sqlite_at(memory_path, err))?;
        upsert_visitor_memory(&tx, visitor).map_err(|err| sqlite_at(memory_path, err))?;
        tx.commit().map_err(|err| sqlite_at(memory_path, err))?;
        Ok(())
    }

    /// Append one `prints` row, then update the memory print columns.
    ///
    /// `category` and `is_cert` are null. `print_count` increases by one.
    /// `last_print_ts` moves to `ts` only when `ts` is newer. A uid that is
    /// not in `list_row` yet is left for the loader, which reads `prints`.
    pub fn record_print(&mut self, uid: &str, ts: i64) -> Result<(), EventDbError> {
        let path = self.file_path();
        let tx = self
            .file
            .unchecked_transaction()
            .map_err(|err| sqlite_at(&path, err))?;
        tx.execute(
            "INSERT INTO prints (uid, ts, category, is_cert) VALUES (?1, ?2, NULL, NULL)",
            rusqlite::params![uid, ts],
        )
        .map_err(|err| sqlite_at(&path, err))?;
        tx.commit().map_err(|err| sqlite_at(&path, err))?;

        let memory_path = Path::new(self.memory_uri());
        let tx = self
            .memory
            .unchecked_transaction()
            .map_err(|err| sqlite_at(memory_path, err))?;
        tx.execute(
            "UPDATE list_row
             SET print_count = print_count + 1,
                 last_print_ts = CASE WHEN ?1 > last_print_ts THEN ?1 ELSE last_print_ts END
             WHERE uid = ?2",
            rusqlite::params![ts, uid],
        )
        .map_err(|err| sqlite_at(memory_path, err))?;
        tx.commit().map_err(|err| sqlite_at(memory_path, err))?;
        Ok(())
    }

    /// One page of the registration list.
    ///
    /// `from` is the first row, default 0. `limit` defaults to 20. When the
    /// copy is `ready`, the page is read from memory. Until then the order is
    /// not complete in memory, so the page and `total` are read from the file.
    /// `total` is the on-disk count, and the rows of that page are inserted
    /// into `list_row` and `u_fts` when they are not already there.
    pub fn table(
        &mut self,
        categories: &[i64],
        order: ListOrder,
        limit: Option<u64>,
        from: Option<u64>,
    ) -> Result<SearchPage, EventDbError> {
        let limit = limit.unwrap_or(DEFAULT_PAGE).min(i64::MAX as u64);
        let from = from.unwrap_or(0).min(i64::MAX as u64);
        if self.load.done {
            return self.search("", categories, order, Some(limit), Some(from));
        }
        self.table_from_file(categories, order, limit as i64, from as i64)
    }

    fn table_from_file(
        &mut self,
        categories: &[i64],
        order: ListOrder,
        limit: i64,
        from: i64,
    ) -> Result<SearchPage, EventDbError> {
        let path = self.file_path();
        let total = count_file(&self.file, &path, categories)?;
        let rows = if total == 0 || from as u64 >= total {
            Vec::new()
        } else {
            fetch_file_page(&self.file, &path, categories, order, limit, from)?
        };
        if !rows.is_empty() {
            let memory_path = Path::new(self.memory_uri());
            let tx = self
                .memory
                .unchecked_transaction()
                .map_err(|err| sqlite_at(memory_path, err))?;
            index_listed(&tx, &rows).map_err(|err| sqlite_at(memory_path, err))?;
            tx.commit().map_err(|err| sqlite_at(memory_path, err))?;
        }
        Ok(SearchPage {
            total,
            rows: rows.into_iter().map(listed_to_list_row).collect(),
        })
    }

    /// The stored cursor for `name`, or `"0"` when that name has no row.
    pub fn cursor(&self, name: &str) -> Result<String, EventDbError> {
        let path = self.file_path();
        let value: Option<String> = self
            .file
            .query_row(
                "SELECT value FROM sync_cursor WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| sqlite_at(&path, err))?;
        Ok(value.unwrap_or_else(|| "0".to_string()))
    }

    /// Store `value` for `name`. The row changes only when this commit returns.
    pub fn set_cursor(&self, name: &str, value: &str) -> Result<(), EventDbError> {
        let path = self.file_path();
        self.file
            .execute(
                "INSERT INTO sync_cursor (name, value) VALUES (?1, ?2)
                 ON CONFLICT(name) DO UPDATE SET value = excluded.value",
                rusqlite::params![name, value],
            )
            .map_err(|err| sqlite_at(&path, err))?;
        Ok(())
    }

    /// Insert ticket barcodes for `form_id`. A negative `clean_id` removes that form's rows.
    ///
    /// A barcode already stored for the form is left as it is.
    pub fn store_ticket_barcodes(
        &mut self,
        form_id: i64,
        clean_id: i64,
        barcodes: &[String],
    ) -> Result<(), EventDbError> {
        let path = self.file_path();
        let tx = self
            .file
            .unchecked_transaction()
            .map_err(|err| sqlite_at(&path, err))?;
        if clean_id < 0 {
            tx.execute("DELETE FROM ticketbarcodes WHERE formId = ?1", [form_id])
                .map_err(|err| sqlite_at(&path, err))?;
        } else {
            for barcode in barcodes {
                tx.execute(
                    "INSERT INTO ticketbarcodes (barcode, formId)
                     SELECT ?1, ?2
                     WHERE NOT EXISTS (
                       SELECT 1 FROM ticketbarcodes WHERE barcode = ?1 AND formId = ?2
                     )",
                    rusqlite::params![barcode, form_id],
                )
                .map_err(|err| sqlite_at(&path, err))?;
            }
        }
        tx.commit().map_err(|err| sqlite_at(&path, err))?;
        Ok(())
    }

    /// How many barcode rows this pool already has.
    pub fn barcode_count(
        &self,
        cat_id: i64,
        bc_type: i64,
        date: &str,
    ) -> Result<i64, EventDbError> {
        let path = self.file_path();
        self.file
            .query_row(
                "SELECT COUNT(*) FROM barcodes WHERE type = ?1 AND repl = ?2 AND date = ?3",
                rusqlite::params![cat_id, bc_type, date],
                |row| row.get(0),
            )
            .map_err(|err| sqlite_at(&path, err))
    }

    /// The `clean_id` stored for this pool, when any row exists.
    pub fn barcode_clean_id(
        &self,
        cat_id: i64,
        bc_type: i64,
        date: &str,
    ) -> Result<Option<i64>, EventDbError> {
        let path = self.file_path();
        self.file
            .query_row(
                "SELECT clean_id FROM barcodes WHERE type = ?1 AND repl = ?2 AND date = ?3 LIMIT 1",
                rusqlite::params![cat_id, bc_type, date],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| sqlite_at(&path, err))
    }

    /// Remove one barcode pool.
    pub fn clean_barcodes(
        &mut self,
        cat_id: i64,
        bc_type: i64,
        date: &str,
    ) -> Result<(), EventDbError> {
        let path = self.file_path();
        self.file
            .execute(
                "DELETE FROM barcodes WHERE type = ?1 AND repl = ?2 AND date = ?3",
                rusqlite::params![cat_id, bc_type, date],
            )
            .map_err(|err| sqlite_at(&path, err))?;
        Ok(())
    }

    /// Insert barcode rows for one pool. A negative `clean_id` removes that pool instead.
    pub fn store_barcodes(
        &mut self,
        cat_id: i64,
        clean_id: i64,
        bc_type: i64,
        date: &str,
        barcodes: &[String],
    ) -> Result<(), EventDbError> {
        let path = self.file_path();
        let tx = self
            .file
            .unchecked_transaction()
            .map_err(|err| sqlite_at(&path, err))?;
        if clean_id < 0 {
            tx.execute(
                "DELETE FROM barcodes WHERE type = ?1 AND repl = ?2 AND date = ?3",
                rusqlite::params![cat_id, bc_type, date],
            )
            .map_err(|err| sqlite_at(&path, err))?;
        } else {
            for barcode in barcodes {
                tx.execute(
                    "INSERT INTO barcodes (barcode, type, clean_id, repl, date) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![barcode, cat_id, clean_id, bc_type, date],
                )
                .map_err(|err| sqlite_at(&path, err))?;
            }
        }
        tx.commit().map_err(|err| sqlite_at(&path, err))?;
        Ok(())
    }

    /// A registration still has to go up when `in_synch` is unset.
    pub fn has_waiting_registration(&self) -> Result<bool, EventDbError> {
        let path = self.file_path();
        let count: i64 = self
            .file
            .query_row(
                "SELECT COUNT(*) FROM mem_u WHERE in_synch IS NULL OR in_synch = 0",
                [],
                |row| row.get(0),
            )
            .map_err(|err| sqlite_at(&path, err))?;
        Ok(count > 0)
    }

    /// Insert one downloaded scan. A later row with the same `scanid` replaces it.
    pub fn store_scan(&mut self, scan: &ScanWrite) -> Result<(), EventDbError> {
        let path = self.file_path();
        self.file
            .execute(
                "INSERT OR REPLACE INTO scans (scanid, zone_id, entrence_type, barcode, time, synched)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                rusqlite::params![
                    scan.scanid,
                    scan.zone_id,
                    scan.entrence_type,
                    scan.barcode,
                    scan.time,
                ],
            )
            .map_err(|err| sqlite_at(&path, err))?;
        Ok(())
    }

    /// Mark a downloaded registration as already on the server and in sync.
    pub fn note_on_server(&self, uid: &str) -> Result<(), EventDbError> {
        let path = self.file_path();
        self.file
            .execute(
                "UPDATE mem_u SET on_server = 1, in_synch = 1 WHERE uid = ?1",
                [uid],
            )
            .map_err(|err| sqlite_at(&path, err))?;
        Ok(())
    }

    /// Registrations whose `in_synch` is unset, oldest uid first, at most `limit`.
    pub fn waiting_registrations(
        &self,
        limit: usize,
    ) -> Result<Vec<WaitingRegistration>, EventDbError> {
        let path = self.file_path();
        let mut stmt = self
            .file
            .prepare(
                "SELECT uid, IFNULL(data, ''), IFNULL(ts, 0), IFNULL(barcode, ''),
                        org_id, category, ticket_status
                 FROM mem_u
                 WHERE in_synch IS NULL OR in_synch = 0
                 ORDER BY CAST(uid AS INTEGER), uid
                 LIMIT ?1",
            )
            .map_err(|err| sqlite_at(&path, err))?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(WaitingRegistration {
                    uid: row.get(0)?,
                    data: row.get(1)?,
                    ts: row.get(2)?,
                    barcode: row.get(3)?,
                    org_id: row.get(4)?,
                    category: row.get(5)?,
                    ticket_status: row.get(6)?,
                })
            })
            .map_err(|err| sqlite_at(&path, err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| sqlite_at(&path, err))
    }

    /// `synch_res` accepted these ids. They are on the server and in sync.
    pub fn accept_registrations(&self, uids: &[&str]) -> Result<(), EventDbError> {
        let path = self.file_path();
        for uid in uids {
            self.file
                .execute(
                    "UPDATE mem_u SET in_synch = 1, on_server = 1 WHERE uid = ?1",
                    [uid],
                )
                .map_err(|err| sqlite_at(&path, err))?;
        }
        Ok(())
    }

    /// Scans waiting to go up, leaving out a scan of a registration not yet on the server.
    pub fn outgoing_scans(&self, limit: usize) -> Result<Vec<OutgoingScan>, EventDbError> {
        let path = self.file_path();
        let mut stmt = self
            .file
            .prepare(
                "SELECT scanid, IFNULL(zone_id, 0), IFNULL(entrence_type, 0),
                        IFNULL(barcode, ''), IFNULL(time, 0)
                 FROM scans
                 WHERE (synched IS NULL OR synched = 0)
                   AND NOT EXISTS (
                     SELECT 1 FROM mem_u
                     WHERE mem_u.barcode IS NOT NULL
                       AND mem_u.barcode != ''
                       AND mem_u.barcode = scans.barcode
                       AND (mem_u.on_server IS NULL OR mem_u.on_server = 0)
                   )
                 ORDER BY scanid
                 LIMIT ?1",
            )
            .map_err(|err| sqlite_at(&path, err))?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(OutgoingScan {
                    scanid: row.get(0)?,
                    zone_id: row.get(1)?,
                    entrence_type: row.get(2)?,
                    barcode: row.get(3)?,
                    time: row.get(4)?,
                })
            })
            .map_err(|err| sqlite_at(&path, err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| sqlite_at(&path, err))
    }

    pub fn mark_scans_synched(&self, scanids: &[i64]) -> Result<(), EventDbError> {
        let path = self.file_path();
        for scanid in scanids {
            self.file
                .execute("UPDATE scans SET synched = 1 WHERE scanid = ?1", [scanid])
                .map_err(|err| sqlite_at(&path, err))?;
        }
        Ok(())
    }

    /// Attachments waiting to go up, leaving out one whose registration is not yet on the server.
    pub fn outgoing_attachments(
        &self,
        limit: usize,
    ) -> Result<Vec<OutgoingAttachment>, EventDbError> {
        let path = self.file_path();
        let mut stmt = self
            .file
            .prepare(
                "SELECT man_id, uid, IFNULL(comment, ''), IFNULL(zone, 0), IFNULL(time, 0)
                 FROM man_to_user
                 WHERE (synched IS NULL OR synched = 0)
                   AND NOT EXISTS (
                     SELECT 1 FROM mem_u
                     WHERE mem_u.uid = CAST(man_to_user.uid AS TEXT)
                       AND (mem_u.on_server IS NULL OR mem_u.on_server = 0)
                   )
                 ORDER BY man_id, uid
                 LIMIT ?1",
            )
            .map_err(|err| sqlite_at(&path, err))?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(OutgoingAttachment {
                    man_id: row.get(0)?,
                    uid: row.get(1)?,
                    comment: row.get(2)?,
                    zone: row.get(3)?,
                    time: row.get(4)?,
                })
            })
            .map_err(|err| sqlite_at(&path, err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| sqlite_at(&path, err))
    }

    pub fn mark_attachments_sent(&self, rows: &[(i64, i64)]) -> Result<(), EventDbError> {
        let path = self.file_path();
        for (man_id, uid) in rows {
            self.file
                .execute(
                    "UPDATE man_to_user SET synched = 1 WHERE man_id = ?1 AND uid = ?2",
                    [man_id, uid],
                )
                .map_err(|err| sqlite_at(&path, err))?;
        }
        Ok(())
    }

    /// The visitor JSON stored in `mem_u.data` on the file.
    ///
    /// An unknown uid is `None`. This does not read or write the memory database.
    pub fn full_row(&self, uid: &str) -> Result<Option<String>, EventDbError> {
        let path = self.file_path();
        self.file
            .query_row("SELECT data FROM mem_u WHERE uid = ?1", [uid], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|err| sqlite_at(&path, err))
    }
}

/// Failure while opening an event database or its base config.
#[derive(Debug)]
pub enum EventDbError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Sqlite {
        path: PathBuf,
        source: rusqlite::Error,
    },
    BadExpoId,
    Config {
        path: PathBuf,
        detail: String,
    },
    UnsupportedConfigVersion {
        path: PathBuf,
        version: u64,
    },
}

impl std::fmt::Display for EventDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventDbError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            EventDbError::Sqlite { path, source } => write!(f, "{}: {source}", path.display()),
            EventDbError::BadExpoId => {
                write!(f, "event id must be a single non-empty path name")
            }
            EventDbError::Config { path, detail } => {
                write!(f, "{}: {detail}", path.display())
            }
            EventDbError::UnsupportedConfigVersion { path, version } => write!(
                f,
                "{} has version {version}; this library reads version {CONFIG_VERSION}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for EventDbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EventDbError::Io { source, .. } => Some(source),
            EventDbError::Sqlite { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn prepare_file(base: &Path, event_id: &str) -> Result<Connection, EventDbError> {
    let dir = event_dir(base, event_id);
    fs::create_dir_all(&dir).map_err(|err| io_at(&dir, err))?;
    let live = dir.join(DB_FILE);
    let creating = creating_path(&live);

    if live.exists() {
        if !fs::metadata(&live)
            .map_err(|err| io_at(&live, err))?
            .is_file()
        {
            return Err(io_at(
                &live,
                io::Error::new(io::ErrorKind::InvalidInput, "not a file"),
            ));
        }
        let header_ok = looks_like_sqlite(&live)?;
        let snapshot = if header_ok {
            snapshot_sidecars(&dir, &live)?
        } else {
            None
        };
        let healthy = if header_ok {
            match inspect(&live) {
                Ok(healthy) => healthy,
                Err(err) => {
                    restore_sidecars(&live, snapshot.as_deref())?;
                    return Err(err);
                }
            }
        } else {
            false
        };
        if healthy {
            discard_snapshot(snapshot.as_deref())?;
            archive_families_if_present(&dir, &[&creating])?;
            let conn = open_live(&live)?;
            migrate(&conn, &live)?;
            return Ok(conn);
        }
        if let Err(err) = mark_full_reload(base, event_id) {
            restore_sidecars(&live, snapshot.as_deref())?;
            return Err(err);
        }
        let archived = archive_families(&dir, &[&live, &creating])?;
        overlay_snapshot(&archived, snapshot.as_deref())?;
        log::warn!(
            "replaced event database {} for {event_id}; archived at {}; full reload required",
            live.display(),
            archived.display()
        );
        publish_new(&live)?;
    } else if creating.exists() {
        mark_full_reload(base, event_id)?;
        let archived = archive_families(&dir, &[&creating])?;
        log::warn!(
            "replaced incomplete event database {} for {event_id}; archived at {}; full reload required",
            creating.display(),
            archived.display()
        );
        publish_new(&live)?;
    } else {
        archive_families_if_present(&dir, &[&live, &creating])?;
        publish_new(&live)?;
    }

    let conn = open_live(&live)?;
    migrate(&conn, &live)?;
    Ok(conn)
}

fn publish_new(live: &Path) -> Result<(), EventDbError> {
    let creating = creating_path(&live);
    if let Err(err) = write_creating(&creating) {
        let _ = remove_family(&creating);
        return Err(err);
    }
    if let Err(err) = fs::rename(&creating, live) {
        let _ = remove_family(&creating);
        return Err(io_at(live, err));
    }
    sync_dir(&parent_dir(live))?;
    Ok(())
}

fn write_creating(creating: &Path) -> Result<(), EventDbError> {
    let conn = Connection::open(creating).map_err(|err| sqlite_at(creating, err))?;
    conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
        .map_err(|err| sqlite_at(creating, err))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|err| sqlite_at(creating, err))?;
    tx.execute_batch(FILE_TABLES)
        .map_err(|err| sqlite_at(creating, err))?;
    tx.execute_batch(EXTRA_INDEXES)
        .map_err(|err| sqlite_at(creating, err))?;
    tx.commit().map_err(|err| sqlite_at(creating, err))?;
    conn.pragma_update(None, "user_version", 1i64)
        .map_err(|err| sqlite_at(creating, err))?;
    drop(conn);
    for suffix in ["-wal", "-shm", "-journal"] {
        remove_if_exists(&with_suffix(creating, suffix))?;
    }
    sync_file(creating)?;
    Ok(())
}

fn open_live(path: &Path) -> Result<Connection, EventDbError> {
    let conn = Connection::open(path).map_err(|err| sqlite_at(path, err))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|err| sqlite_at(path, err))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(|err| sqlite_at(path, err))?;
    Ok(conn)
}

fn migrate(conn: &Connection, path: &Path) -> Result<(), EventDbError> {
    conn.execute_batch(EXTRA_INDEXES)
        .map_err(|err| sqlite_at(path, err))?;
    ensure_sync_cursor(conn, path)?;
    ensure_new_id(conn, path)?;
    ensure_on_server(conn, path)?;
    seed_keys(conn, path)?;
    Ok(())
}

fn ensure_sync_cursor(conn: &Connection, path: &Path) -> Result<(), EventDbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_cursor (
           name TEXT PRIMARY KEY,
           value TEXT NOT NULL
         )",
    )
    .map_err(|err| sqlite_at(path, err))?;
    Ok(())
}

fn ensure_new_id(conn: &Connection, path: &Path) -> Result<(), EventDbError> {
    if has_column(conn, path, "mem_u", "new_id")? {
        return Ok(());
    }
    conn.execute_batch("ALTER TABLE mem_u ADD COLUMN new_id INTEGER")
        .map_err(|err| sqlite_at(path, err))?;
    Ok(())
}

/// `on_server` stays set after an edit clears `in_synch`. A local insert leaves it unset.
fn ensure_on_server(conn: &Connection, path: &Path) -> Result<(), EventDbError> {
    if has_column(conn, path, "mem_u", "on_server")? {
        return Ok(());
    }
    conn.execute_batch("ALTER TABLE mem_u ADD COLUMN on_server INTEGER")
        .map_err(|err| sqlite_at(path, err))?;
    Ok(())
}

fn has_column(
    conn: &Connection,
    path: &Path,
    table: &str,
    column: &str,
) -> Result<bool, EventDbError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|err| sqlite_at(path, err))?;
    let mut rows = stmt.query([]).map_err(|err| sqlite_at(path, err))?;
    while let Some(row) = rows.next().map_err(|err| sqlite_at(path, err))? {
        let name: String = row.get(1).map_err(|err| sqlite_at(path, err))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn seed_keys(conn: &Connection, path: &Path) -> Result<(), EventDbError> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM keys", [], |row| row.get(0))
        .map_err(|err| sqlite_at(path, err))?;
    if count > 0 {
        return Ok(());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|err| sqlite_at(path, err))?;
    insert_key(&tx, path, 1)?;
    insert_key(&tx, path, 0)?;
    tx.commit().map_err(|err| sqlite_at(path, err))?;
    Ok(())
}

fn insert_key(conn: &Connection, path: &Path, is_admin: i64) -> Result<(), EventDbError> {
    for _ in 0..8 {
        let key = generate_key()?;
        match conn.execute(
            "INSERT INTO keys (key, is_admin, comment) VALUES (?1, ?2, NULL)",
            rusqlite::params![key, is_admin],
        ) {
            Ok(_) => return Ok(()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                continue;
            }
            Err(err) => return Err(sqlite_at(path, err)),
        }
    }
    Err(io_at(
        path,
        io::Error::new(
            io::ErrorKind::Other,
            "could not generate a unique operator key",
        ),
    ))
}

fn generate_key() -> Result<String, EventDbError> {
    let mut bytes = [0u8; 7];
    fill_random(&mut bytes)?;
    let alphabet: Vec<char> = KEY_ALPHABET.chars().collect();
    let mut out = String::with_capacity(7);
    for byte in bytes {
        let index = byte as usize % alphabet.len();
        out.push(alphabet[index]);
    }
    Ok(out)
}

fn fill_random(bytes: &mut [u8]) -> Result<(), EventDbError> {
    let mut file =
        File::open("/dev/urandom").map_err(|err| io_at(Path::new("/dev/urandom"), err))?;
    io::Read::read_exact(&mut file, bytes).map_err(|err| io_at(Path::new("/dev/urandom"), err))?;
    Ok(())
}

/// `true` when the live file can be kept. `false` when it must be replaced.
fn inspect(path: &Path) -> Result<bool, EventDbError> {
    let meta = fs::metadata(path).map_err(|err| io_at(path, err))?;
    if !meta.is_file() {
        return Err(io_at(
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "not a file"),
        ));
    }
    let conn = match Connection::open(path) {
        Ok(conn) => conn,
        Err(err) if is_corrupt(&err) => return Ok(false),
        Err(err) => return Err(sqlite_at(path, err)),
    };
    match file_is_healthy(&conn, path) {
        Ok(healthy) => Ok(healthy),
        Err(EventDbError::Sqlite { source, .. }) if is_corrupt(&source) => Ok(false),
        Err(err) => Err(err),
    }
}

fn file_is_healthy(conn: &Connection, path: &Path) -> Result<bool, EventDbError> {
    let mut broken = false;
    conn.pragma_query(None, "quick_check", |row| {
        let value: String = row.get(0)?;
        if value != "ok" {
            broken = true;
        }
        Ok(())
    })
    .map_err(|err| sqlite_at(path, err))?;
    if broken {
        return Ok(false);
    }
    for table in REQUIRED_TABLES {
        if !ordinary_table(conn, path, table)? {
            return Ok(false);
        }
    }
    let virtuals: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE sql LIKE '%virtual table%'",
            [],
            |row| row.get(0),
        )
        .map_err(|err| sqlite_at(path, err))?;
    Ok(virtuals == 0)
}

fn ordinary_table(conn: &Connection, path: &Path, name: &str) -> Result<bool, EventDbError> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = ?1 AND type = 'table'",
            [name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| sqlite_at(path, err))?;
    match sql {
        Some(sql) => Ok(!sql.to_ascii_lowercase().contains("virtual table")),
        None => Ok(false),
    }
}

fn is_corrupt(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(code, _) => matches!(
            code.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ),
        _ => false,
    }
}

fn open_memory() -> Result<(Connection, String), EventDbError> {
    let id = MEMORY_SEQ.fetch_add(1, Ordering::Relaxed);
    let uri = format!("file:event-mem-{id}?mode=memory&cache=shared");
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_URI;
    let conn =
        Connection::open_with_flags(&uri, flags).map_err(|err| sqlite_at(Path::new(&uri), err))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|err| sqlite_at(Path::new(&uri), err))?;
    conn.execute_batch(MEMORY_SCHEMA)
        .map_err(|err| sqlite_at(Path::new(&uri), err))?;
    conn.execute("INSERT INTO u_fts(u_fts, rank) VALUES('automerge', 0)", [])
        .map_err(|err| sqlite_at(Path::new(&uri), err))?;
    Ok((conn, uri))
}

struct LoadCursor {
    done: bool,
    after_ts: Option<i64>,
    after_uid: Option<String>,
    batches: u32,
}

impl LoadCursor {
    fn from_file(conn: &Connection, path: &Path) -> Result<Self, EventDbError> {
        let visitors: i64 = conn
            .query_row("SELECT COUNT(*) FROM mem_u", [], |row| row.get(0))
            .map_err(|err| sqlite_at(path, err))?;
        Ok(Self {
            done: visitors == 0,
            after_ts: None,
            after_uid: None,
            batches: 0,
        })
    }
}

struct ListedVisitor {
    uid: String,
    name: String,
    surname: String,
    c_name: String,
    category: i64,
    ticket_status: i64,
    gotsome: i64,
    give_packet: i64,
    added_ts: i64,
    last_print_ts: i64,
    print_count: i64,
    org_id: Option<i64>,
    email: String,
}

fn fetch_visitors(
    conn: &Connection,
    path: &Path,
    after_ts: Option<i64>,
    after_uid: Option<&str>,
    limit: usize,
) -> Result<Vec<ListedVisitor>, EventDbError> {
    let continued = after_ts.is_some() && after_uid.is_some();
    let mut sql = String::from(LOAD_FROM);
    if continued {
        // Same order as `table` `last_add`: newer `ts` first, then `uid` ascending.
        sql.push_str(" WHERE mem_u.ts < ?1 OR (mem_u.ts = ?1 AND mem_u.uid > ?2)");
        sql.push_str(" ORDER BY mem_u.ts DESC, mem_u.uid LIMIT ?3");
    } else {
        sql.push_str(" ORDER BY mem_u.ts DESC, mem_u.uid LIMIT ?1");
    }
    let mut stmt = conn.prepare(&sql).map_err(|err| sqlite_at(path, err))?;
    let map = |row: &rusqlite::Row<'_>| read_listed(row);
    let rows = match (after_ts, after_uid) {
        (Some(ts), Some(uid)) => stmt
            .query_map(rusqlite::params![ts, uid, limit as i64], map)
            .map_err(|err| sqlite_at(path, err))?,
        _ => stmt
            .query_map(rusqlite::params![limit as i64], map)
            .map_err(|err| sqlite_at(path, err))?,
    };
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| sqlite_at(path, err))
}

fn upsert_visitor_file(conn: &Connection, visitor: &VisitorWrite) -> Result<(), rusqlite::Error> {
    let updated = conn.execute(
        "UPDATE mem_u
         SET data = ?1, ts = ?2, category = ?3, ticket_status = ?4, org_id = ?5,
             in_synch = NULL
         WHERE uid = ?6",
        rusqlite::params![
            visitor.data,
            visitor.added_ts,
            visitor.category,
            visitor.ticket_status,
            visitor.org_id,
            visitor.uid,
        ],
    )?;
    if updated == 0 {
        conn.execute(
            "INSERT INTO mem_u (uid, data, ts, category, ticket_status, org_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                visitor.uid,
                visitor.data,
                visitor.added_ts,
                visitor.category,
                visitor.ticket_status,
                visitor.org_id,
            ],
        )?;
    }
    let updated = conn.execute(
        "UPDATE u_fulltext
         SET name = ?1, surname = ?2, c_name = ?3, category = ?4,
             ticket_status = ?5, gotsome = ?6, give_packet = ?7
         WHERE uid = ?8",
        rusqlite::params![
            visitor.name,
            visitor.surname,
            visitor.c_name,
            visitor.category.to_string(),
            visitor.ticket_status.to_string(),
            visitor.gotsome.to_string(),
            visitor.give_packet.to_string(),
            visitor.uid,
        ],
    )?;
    if updated == 0 {
        conn.execute(
            "INSERT INTO u_fulltext (
                uid, name, surname, c_name, category, ticket_status, gotsome, give_packet
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                visitor.uid,
                visitor.name,
                visitor.surname,
                visitor.c_name,
                visitor.category.to_string(),
                visitor.ticket_status.to_string(),
                visitor.gotsome.to_string(),
                visitor.give_packet.to_string(),
            ],
        )?;
    }
    conn.execute(
        "DELETE FROM emails WHERE uid = CAST(?1 AS INTEGER)",
        [&visitor.uid],
    )?;
    if !visitor.email.trim().is_empty() {
        conn.execute(
            "INSERT INTO emails (uid, email) VALUES (CAST(?1 AS INTEGER), ?2)",
            rusqlite::params![visitor.uid, visitor.email],
        )?;
    }
    Ok(())
}

fn upsert_visitor_memory(conn: &Connection, visitor: &VisitorWrite) -> Result<(), rusqlite::Error> {
    let existing = conn
        .query_row(
            "SELECT rowid, name, surname, c_name FROM list_row WHERE uid = ?1",
            [&visitor.uid],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((rowid, name, surname, c_name)) = existing else {
        return insert_listed(
            conn,
            &ListedVisitor {
                uid: visitor.uid.clone(),
                name: visitor.name.clone(),
                surname: visitor.surname.clone(),
                c_name: visitor.c_name.clone(),
                category: visitor.category,
                ticket_status: visitor.ticket_status,
                gotsome: visitor.gotsome,
                give_packet: visitor.give_packet,
                added_ts: visitor.added_ts,
                last_print_ts: 0,
                print_count: 0,
                org_id: visitor.org_id,
                email: visitor.email.clone(),
            },
        );
    };
    conn.execute(
        "INSERT INTO u_fts(u_fts, rowid, name, surname, c_name) VALUES('delete', ?1, ?2, ?3, ?4)",
        rusqlite::params![rowid, name, surname, c_name],
    )?;
    conn.execute(
        "UPDATE list_row
         SET name = ?1, surname = ?2, c_name = ?3, category = ?4, ticket_status = ?5,
             gotsome = ?6, give_packet = ?7, added_ts = ?8, org_id = ?9, email = ?10
         WHERE uid = ?11",
        rusqlite::params![
            visitor.name,
            visitor.surname,
            visitor.c_name,
            visitor.category,
            visitor.ticket_status,
            visitor.gotsome,
            visitor.give_packet,
            visitor.added_ts,
            visitor.org_id,
            visitor.email,
            visitor.uid,
        ],
    )?;
    conn.execute(
        "INSERT INTO u_fts(rowid, name, surname, c_name) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![rowid, visitor.name, visitor.surname, visitor.c_name],
    )?;
    Ok(())
}

fn read_listed(row: &rusqlite::Row<'_>) -> rusqlite::Result<ListedVisitor> {
    Ok(ListedVisitor {
        uid: row.get(0)?,
        name: row.get(1)?,
        surname: row.get(2)?,
        c_name: row.get(3)?,
        category: row.get(4)?,
        ticket_status: row.get(5)?,
        gotsome: row.get(6)?,
        give_packet: row.get(7)?,
        added_ts: row.get(8)?,
        last_print_ts: row.get(9)?,
        print_count: row.get(10)?,
        org_id: row.get(11)?,
        email: row.get(12)?,
    })
}

fn listed_to_list_row(row: ListedVisitor) -> ListRow {
    ListRow {
        uid: row.uid,
        name: row.name,
        surname: row.surname,
        c_name: row.c_name,
        category: row.category,
        ticket_status: row.ticket_status,
        gotsome: row.gotsome,
        give_packet: row.give_packet,
        added_ts: row.added_ts,
        last_print_ts: row.last_print_ts,
        print_count: row.print_count,
        org_id: row.org_id,
        email: row.email,
    }
}

fn file_order(order: ListOrder) -> &'static str {
    match order {
        ListOrder::LastAdd => " ORDER BY added_ts DESC, uid",
        ListOrder::LastPrint => " ORDER BY last_print_ts DESC, added_ts DESC, uid",
    }
}

fn category_placeholders(count: usize, start: usize) -> String {
    (0..count)
        .map(|offset| format!("?{}", start + offset))
        .collect::<Vec<_>>()
        .join(", ")
}

fn count_file(conn: &Connection, path: &Path, categories: &[i64]) -> Result<u64, EventDbError> {
    let sql = if categories.is_empty() {
        "SELECT COUNT(*) FROM mem_u".to_string()
    } else {
        format!(
            "SELECT COUNT(*) FROM mem_u WHERE IFNULL(category, 0) IN ({})",
            category_placeholders(categories.len(), 1)
        )
    };
    let args: Vec<SqlValue> = categories.iter().copied().map(SqlValue::Integer).collect();
    let total: i64 = conn
        .query_row(&sql, params_from_iter(args.iter()), |row| row.get(0))
        .map_err(|err| sqlite_at(path, err))?;
    Ok(total as u64)
}

fn fetch_file_page(
    conn: &Connection,
    path: &Path,
    categories: &[i64],
    order: ListOrder,
    limit: i64,
    from: i64,
) -> Result<Vec<ListedVisitor>, EventDbError> {
    let (filter, next) = if categories.is_empty() {
        (String::new(), 1usize)
    } else {
        (
            format!(
                " WHERE category IN ({})",
                category_placeholders(categories.len(), 1)
            ),
            categories.len() + 1,
        )
    };
    let sql = format!(
        "SELECT {LIST_COLUMNS} FROM ({LOAD_FROM}) AS listed{filter}{} LIMIT ?{next} OFFSET ?{}",
        file_order(order),
        next + 1
    );
    let mut args: Vec<SqlValue> = categories.iter().copied().map(SqlValue::Integer).collect();
    args.push(SqlValue::Integer(limit));
    args.push(SqlValue::Integer(from));
    let mut stmt = conn.prepare(&sql).map_err(|err| sqlite_at(path, err))?;
    let rows = stmt
        .query_map(params_from_iter(args.iter()), read_listed)
        .map_err(|err| sqlite_at(path, err))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| sqlite_at(path, err))
}

/// Copy a file page into memory. A uid already in `list_row` stays as it is,
/// so a write during the copy is not replaced by the older file image.
fn index_listed(conn: &Connection, rows: &[ListedVisitor]) -> Result<(), rusqlite::Error> {
    for row in rows {
        if list_has_uid(conn, &row.uid)? {
            continue;
        }
        insert_listed(conn, row)?;
    }
    Ok(())
}

fn list_has_uid(conn: &Connection, uid: &str) -> Result<bool, rusqlite::Error> {
    Ok(conn
        .query_row("SELECT 1 FROM list_row WHERE uid = ?1", [uid], |_| Ok(()))
        .optional()?
        .is_some())
}

fn insert_listed(conn: &Connection, row: &ListedVisitor) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO list_row (
            uid, name, surname, c_name, category, ticket_status, gotsome, give_packet,
            added_ts, last_print_ts, print_count, org_id, email
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            row.uid,
            row.name,
            row.surname,
            row.c_name,
            row.category,
            row.ticket_status,
            row.gotsome,
            row.give_packet,
            row.added_ts,
            row.last_print_ts,
            row.print_count,
            row.org_id,
            row.email,
        ],
    )?;
    let rowid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO u_fts(rowid, name, surname, c_name) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![rowid, row.name, row.surname, row.c_name],
    )?;
    Ok(())
}

/// How `text` becomes an FTS5 `MATCH` string.
#[derive(Debug)]
enum TextQuery {
    /// No words. Read `list_row` only.
    All,
    /// Words were typed, and none of them is a token.
    None,
    /// Bound `MATCH` text. The SQL statement does not contain this string.
    Match(String),
}

/// Split `text` into words. Each word that contains a letter or digit becomes
/// one phrase: internal `"` are doubled, the word is wrapped in `"`, and `*`
/// sits outside the quotes so it is a prefix. A word of only operators is
/// dropped, so it cannot be parsed as `AND`, `OR`, `NOT`, `NEAR`, or a column filter.
fn match_query(text: &str) -> TextQuery {
    let mut saw_word = false;
    let mut clauses = Vec::new();
    for word in text.split_whitespace() {
        saw_word = true;
        if let Some(phrase) = fts_phrase(word) {
            clauses.push(format!(
                "(name : {phrase} OR surname : {phrase} OR c_name : {phrase})"
            ));
        }
    }
    if clauses.is_empty() {
        if saw_word {
            TextQuery::None
        } else {
            TextQuery::All
        }
    } else {
        TextQuery::Match(clauses.join(" AND "))
    }
}

/// Quote `word` for FTS5, or `None` when it has no letter or digit.
fn fts_phrase(word: &str) -> Option<String> {
    let mut quoted = String::new();
    let mut token = false;
    for ch in word.chars() {
        if ch == '\0' {
            continue;
        }
        if ch.is_alphanumeric() {
            token = true;
        }
        if ch == '"' {
            quoted.push('"');
        }
        quoted.push(ch);
    }
    if !token {
        return None;
    }
    Some(format!("\"{quoted}\"*"))
}

fn list_where(has_match: bool, category_count: usize) -> (String, usize) {
    let mut next = 1usize;
    let mut filters = Vec::new();
    if has_match {
        filters.push(format!(
            "rowid IN (SELECT rowid FROM u_fts WHERE u_fts MATCH ?{next})"
        ));
        next += 1;
    }
    if category_count > 0 {
        let placeholders = (0..category_count)
            .map(|offset| format!("?{}", next + offset))
            .collect::<Vec<_>>()
            .join(", ");
        filters.push(format!("category IN ({placeholders})"));
        next += category_count;
    }
    let sql = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };
    (sql, next)
}

fn order_clause(order: ListOrder) -> &'static str {
    match order {
        ListOrder::LastAdd => " ORDER BY added_ts DESC, uid",
        ListOrder::LastPrint => " ORDER BY last_print_ts DESC, added_ts DESC, uid",
    }
}

fn list_select_sql(has_match: bool, category_count: usize, order: ListOrder) -> String {
    let (where_sql, next) = list_where(has_match, category_count);
    format!(
        "SELECT {LIST_COLUMNS} FROM list_row{where_sql}{} LIMIT ?{next} OFFSET ?{}",
        order_clause(order),
        next + 1
    )
}

fn list_count_sql(has_match: bool, category_count: usize) -> String {
    let (where_sql, _) = list_where(has_match, category_count);
    format!("SELECT COUNT(*) FROM list_row{where_sql}")
}

fn filter_args(match_expr: Option<&str>, categories: &[i64]) -> Vec<SqlValue> {
    let mut args = Vec::new();
    if let Some(expr) = match_expr {
        args.push(SqlValue::Text(expr.to_string()));
    }
    for category in categories {
        args.push(SqlValue::Integer(*category));
    }
    args
}

fn count_listed(
    conn: &Connection,
    path: &Path,
    match_expr: Option<&str>,
    categories: &[i64],
) -> Result<u64, EventDbError> {
    let sql = list_count_sql(match_expr.is_some(), categories.len());
    let args = filter_args(match_expr, categories);
    let total: i64 = conn
        .query_row(&sql, params_from_iter(args.iter()), |row| row.get(0))
        .map_err(|err| sqlite_at(path, err))?;
    Ok(total as u64)
}

fn select_listed(
    conn: &Connection,
    path: &Path,
    match_expr: Option<&str>,
    categories: &[i64],
    order: ListOrder,
    limit: i64,
    offset: i64,
) -> Result<Vec<ListRow>, EventDbError> {
    let sql = list_select_sql(match_expr.is_some(), categories.len(), order);
    let mut args = filter_args(match_expr, categories);
    args.push(SqlValue::Integer(limit));
    args.push(SqlValue::Integer(offset));
    let mut stmt = conn.prepare(&sql).map_err(|err| sqlite_at(path, err))?;
    let rows = stmt
        .query_map(params_from_iter(args.iter()), read_list_row)
        .map_err(|err| sqlite_at(path, err))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| sqlite_at(path, err))
}

fn read_list_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ListRow> {
    Ok(ListRow {
        uid: row.get(0)?,
        name: row.get(1)?,
        surname: row.get(2)?,
        c_name: row.get(3)?,
        category: row.get(4)?,
        ticket_status: row.get(5)?,
        gotsome: row.get(6)?,
        give_packet: row.get(7)?,
        added_ts: row.get(8)?,
        last_print_ts: row.get(9)?,
        print_count: row.get(10)?,
        org_id: row.get(11)?,
        email: row.get(12)?,
    })
}

fn mark_full_reload(base: &Path, event_id: &str) -> Result<(), EventDbError> {
    let path = base.join(CONFIG_FILE);
    let mut map = match read_config(&path)? {
        Some(map) => map,
        None => Mapping::new(),
    };
    let mut ids = reload_ids(&path, &map)?;
    if ids.iter().any(|id| id == event_id) {
        return Ok(());
    }
    ids.push(event_id.to_string());
    map.insert(
        Value::String("version".to_string()),
        Value::Number(CONFIG_VERSION.into()),
    );
    map.insert(
        Value::String("full_reload".to_string()),
        Value::Sequence(ids.into_iter().map(Value::String).collect()),
    );
    let text = serde_yaml::to_string(&Value::Mapping(map))
        .map_err(|err| config_err(&path, &err.to_string()))?;
    replace_file(&path, text.as_bytes())
}

fn read_config(path: &Path) -> Result<Option<Mapping>, EventDbError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).map_err(|err| io_at(path, err))?;
    if text.trim().is_empty() {
        return Err(config_err(path, "file is empty"));
    }
    let value: Value =
        serde_yaml::from_str(&text).map_err(|err| config_err(path, &err.to_string()))?;
    let Value::Mapping(map) = value else {
        return Err(config_err(path, "document is not a map"));
    };
    if let Some(version) = map.get(Value::String("version".to_string())) {
        let version = version
            .as_u64()
            .ok_or_else(|| config_err(path, "version is not a number"))?;
        if version != CONFIG_VERSION {
            return Err(EventDbError::UnsupportedConfigVersion {
                path: path.to_path_buf(),
                version,
            });
        }
    }
    Ok(Some(map))
}

fn reload_ids(path: &Path, map: &Mapping) -> Result<Vec<String>, EventDbError> {
    let Some(value) = map.get(Value::String("full_reload".to_string())) else {
        return Ok(Vec::new());
    };
    let Value::Sequence(items) = value else {
        return Err(config_err(path, "full_reload is not a list"));
    };
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let Value::String(id) = item else {
            return Err(config_err(path, "full_reload entry is not a string"));
        };
        ids.push(id.clone());
    }
    Ok(ids)
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), EventDbError> {
    let dir = parent_dir(path);
    fs::create_dir_all(&dir).map_err(|err| io_at(&dir, err))?;
    sync_dir(&dir)?;
    let tmp = temp_path(path);
    if let Err(err) = write_temp(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(io_at(path, err));
    }
    sync_dir(&dir)?;
    Ok(())
}

fn write_temp(tmp: &Path, bytes: &[u8]) -> Result<(), EventDbError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    let mut file = options.open(tmp).map_err(|err| io_at(tmp, err))?;
    file.write_all(bytes).map_err(|err| io_at(tmp, err))?;
    file.sync_all().map_err(|err| io_at(tmp, err))?;
    Ok(())
}

fn sync_file(path: &Path) -> Result<(), EventDbError> {
    let file = File::open(path).map_err(|err| io_at(path, err))?;
    file.sync_all().map_err(|err| io_at(path, err))?;
    Ok(())
}

fn sync_dir(dir: &Path) -> Result<(), EventDbError> {
    let file = File::open(dir).map_err(|err| io_at(dir, err))?;
    file.sync_all().map_err(|err| io_at(dir, err))?;
    Ok(())
}

fn looks_like_sqlite(path: &Path) -> Result<bool, EventDbError> {
    let meta = fs::metadata(path).map_err(|err| io_at(path, err))?;
    if !meta.is_file() || meta.len() < 100 {
        return Ok(false);
    }
    let mut file = File::open(path).map_err(|err| io_at(path, err))?;
    let mut header = [0u8; 16];
    io::Read::read_exact(&mut file, &mut header).map_err(|err| io_at(path, err))?;
    Ok(&header == b"SQLite format 3\0")
}

/// Copy `-wal`, `-shm`, and `-journal` before SQLite opens the file.
///
/// Opening a file that is not a database truncates those sidecars. The copy
/// is the bytes to keep for repair.
fn snapshot_sidecars(event_dir: &Path, family: &Path) -> Result<Option<PathBuf>, EventDbError> {
    let sidecars: Vec<PathBuf> = ["-wal", "-shm", "-journal"]
        .into_iter()
        .map(|suffix| with_suffix(family, suffix))
        .filter(|path| path.is_file())
        .collect();
    if sidecars.is_empty() {
        return Ok(None);
    }
    let n = MEMORY_SEQ.fetch_add(1, Ordering::Relaxed);
    let hold = event_dir.join(format!(".sidecar-{n}"));
    fs::create_dir(&hold).map_err(|err| io_at(&hold, err))?;
    for src in &sidecars {
        let to = hold.join(src.file_name().unwrap_or_default());
        fs::copy(src, &to).map_err(|err| io_at(&to, err))?;
        sync_file(&to)?;
    }
    sync_dir(&hold)?;
    Ok(Some(hold))
}

fn restore_sidecars(family: &Path, hold: Option<&Path>) -> Result<(), EventDbError> {
    let Some(hold) = hold else {
        return Ok(());
    };
    for entry in fs::read_dir(hold).map_err(|err| io_at(hold, err))? {
        let entry = entry.map_err(|err| io_at(hold, err))?;
        let src = entry.path();
        if !src.is_file() {
            continue;
        }
        let to = family.with_file_name(entry.file_name());
        fs::copy(&src, &to).map_err(|err| io_at(&to, err))?;
        sync_file(&to)?;
    }
    discard_snapshot(Some(hold))?;
    Ok(())
}

fn overlay_snapshot(backup: &Path, hold: Option<&Path>) -> Result<(), EventDbError> {
    let Some(hold) = hold else {
        return Ok(());
    };
    for entry in fs::read_dir(hold).map_err(|err| io_at(hold, err))? {
        let entry = entry.map_err(|err| io_at(hold, err))?;
        let src = entry.path();
        if !src.is_file() {
            continue;
        }
        let to = backup.join(entry.file_name());
        fs::copy(&src, &to).map_err(|err| io_at(&to, err))?;
        sync_file(&to)?;
    }
    discard_snapshot(Some(hold))?;
    sync_dir(backup)?;
    Ok(())
}

fn discard_snapshot(hold: Option<&Path>) -> Result<(), EventDbError> {
    let Some(hold) = hold else {
        return Ok(());
    };
    fs::remove_dir_all(hold).map_err(|err| io_at(hold, err))?;
    Ok(())
}

/// Move `db.sqlite` and `db.sqlite.creating`, with their `-wal`, `-shm`, and
/// `-journal` files, into `{event}/broken/{index}-{unix_millis}/`.
///
/// The index is one higher than any backup already in that directory. The
/// original file names stay, so a later repair can open the backup with its
/// journal beside it.
fn archive_families(event_dir: &Path, families: &[&Path]) -> Result<PathBuf, EventDbError> {
    let sources = family_files(families);
    if sources.is_empty() {
        return Err(io_at(
            event_dir,
            io::Error::new(io::ErrorKind::NotFound, "no database file to archive"),
        ));
    }
    let broken = event_dir.join("broken");
    let index = next_backup_index(&broken)?;
    let dest = broken.join(format!("{index}-{}", unix_millis()));
    fs::create_dir_all(&dest).map_err(|err| io_at(&dest, err))?;
    for src in &sources {
        let to = dest.join(src.file_name().unwrap_or_default());
        fs::rename(src, &to).map_err(|err| io_at(&to, err))?;
    }
    sync_dir(&dest)?;
    sync_dir(&broken)?;
    sync_dir(event_dir)?;
    Ok(dest)
}

fn archive_families_if_present(event_dir: &Path, families: &[&Path]) -> Result<(), EventDbError> {
    if family_files(families).is_empty() {
        return Ok(());
    }
    archive_families(event_dir, families)?;
    Ok(())
}

fn family_files(families: &[&Path]) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for family in families {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let path = with_suffix(family, suffix);
            if path.is_file() {
                sources.push(path);
            }
        }
    }
    sources
}

fn next_backup_index(broken: &Path) -> Result<u64, EventDbError> {
    if !broken.exists() {
        return Ok(1);
    }
    let mut max = 0u64;
    for entry in fs::read_dir(broken).map_err(|err| io_at(broken, err))? {
        let entry = entry.map_err(|err| io_at(broken, err))?;
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((index, time)) = name.split_once('-') else {
            continue;
        };
        if time.is_empty() || !time.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let Ok(index) = index.parse::<u64>() else {
            continue;
        };
        max = max.max(index);
    }
    Ok(max + 1)
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn remove_family(path: &Path) -> Result<(), EventDbError> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        remove_if_exists(&with_suffix(path, suffix))?;
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), EventDbError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(io_at(path, err)),
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn creating_path(live: &Path) -> PathBuf {
    with_suffix(live, ".creating")
}

fn event_dir(base: &Path, event_id: &str) -> PathBuf {
    base.join(event_id)
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    name.push(format!(".tmp-{}-{nanos}", std::process::id()));
    path.with_file_name(name)
}

pub(crate) fn check_event_id(event_id: &str) -> Result<(), EventDbError> {
    normalize_event_id(event_id).map(|_| ())
}

fn normalize_event_id(event_id: &str) -> Result<String, EventDbError> {
    if event_id.is_empty()
        || event_id == "."
        || event_id == ".."
        || event_id.contains('/')
        || event_id.contains('\\')
        || event_id.contains('\0')
    {
        return Err(EventDbError::BadExpoId);
    }
    Ok(event_id.to_string())
}

fn sqlite_at(path: &Path, source: rusqlite::Error) -> EventDbError {
    EventDbError::Sqlite {
        path: path.to_path_buf(),
        source,
    }
}

fn io_at(path: &Path, source: io::Error) -> EventDbError {
    EventDbError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn config_err(path: &Path, detail: &str) -> EventDbError {
    EventDbError::Config {
        path: path.to_path_buf(),
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    /// `(uid, name, surname)` in `last_add` order.
    fn last_add(db: &super::EventDb) -> Vec<(String, String, String)> {
        let mut stmt = db
            .file()
            .prepare(
                "SELECT mem_u.uid, IFNULL(u_fulltext.name, ''), IFNULL(u_fulltext.surname, '')
                 FROM mem_u
                 LEFT JOIN u_fulltext ON u_fulltext.uid = mem_u.uid
                 ORDER BY mem_u.ts DESC, mem_u.uid",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }

    fn unique_surname(
        order: &[(String, String, String)],
        range: std::ops::Range<usize>,
    ) -> (String, String) {
        order[range]
            .iter()
            .find(|(_, _, surname)| {
                !surname.is_empty() && order.iter().filter(|row| &row.2 == surname).count() == 1
            })
            .map(|(uid, _, surname)| (uid.clone(), surname.clone()))
            .expect("the fixture has a unique surname in this range")
    }

    mod open {
        use super::super::{
            EventDb, EventDbError, IndexState, CONFIG_FILE, DB_FILE, FILE_TABLES, KEY_ALPHABET,
        };
        use rusqlite::Connection;
        use serde::Deserialize;
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(1);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-event-db-{}-{}-{n}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        #[derive(Deserialize)]
        struct ConfigFile {
            version: u32,
            full_reload: Vec<String>,
        }

        fn config_file(base: &Path) -> Option<ConfigFile> {
            let path = base.join(CONFIG_FILE);
            if !path.exists() {
                return None;
            }
            let text = fs::read_to_string(path).unwrap();
            Some(serde_yaml::from_str(&text).unwrap())
        }

        fn key_rows(db: &EventDb) -> Vec<(String, i64)> {
            let mut stmt = db
                .file
                .prepare("SELECT key, is_admin FROM keys ORDER BY is_admin DESC, key")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        }

        fn count(conn: &Connection, table: &str) -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        }

        fn column_names(conn: &Connection, table: &str) -> Vec<String> {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            stmt.query_map([], |row| row.get(1))
                .unwrap()
                .collect::<Result<Vec<String>, _>>()
                .unwrap()
        }

        fn column_type(conn: &Connection, table: &str, column: &str) -> String {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let mut rows = stmt.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                let name: String = row.get(1).unwrap();
                if name == column {
                    return row.get(2).unwrap();
                }
            }
            panic!("{table}.{column} is missing");
        }

        fn insert_list_row(db: &EventDb, uid: &str) {
            db.memory
                .execute(
                    "INSERT INTO list_row (
                        uid, name, surname, c_name, category, ticket_status, gotsome,
                        give_packet, added_ts, last_print_ts, print_count, org_id, email
                    ) VALUES (?1, '', '', '', 0, 0, 0, 0, 0, 0, 0, NULL, '')",
                    [uid],
                )
                .unwrap();
        }

        fn write_scala_shaped(path: &Path) {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let conn = Connection::open(path).unwrap();
            conn.execute_batch(FILE_TABLES).unwrap();
            conn.execute(
                "INSERT INTO mem_u (uid, data, ts) VALUES ('keep', '{\"n\":1}', 10)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO keys (key, is_admin, comment) VALUES ('KEEPME1', 1, 'kept')",
                [],
            )
            .unwrap();
        }

        fn backups(event_dir: &Path) -> Vec<(u64, u128, PathBuf)> {
            let broken = event_dir.join("broken");
            if !broken.exists() {
                return Vec::new();
            }
            let mut found = Vec::new();
            for entry in fs::read_dir(&broken).unwrap() {
                let path = entry.unwrap().path();
                if !path.is_dir() {
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy();
                let (index, time) = name.split_once('-').unwrap();
                found.push((index.parse().unwrap(), time.parse().unwrap(), path));
            }
            found.sort_by_key(|(index, _, _)| *index);
            found
        }

        #[test]
        fn open_creates_file() {
            let scratch = Scratch::new();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert!(db.file_path().is_file());
            assert!(!db.file_path().with_file_name("db.sqlite.creating").exists());
            assert_eq!(count(db.file(), "mem_u"), 0);
            assert_eq!(db.index_state().unwrap(), IndexState::Ready);
            assert_eq!(db.loaded_count().unwrap(), 0);
            let mode: String = db
                .file()
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
            let synchronous: i64 = db
                .file()
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .unwrap();
            assert_eq!(synchronous, 2);
            assert!(config_file(scratch.path()).is_none());
            assert!(!db.needs_full_reload().unwrap());
            assert!(backups(&scratch.path().join("EVENT")).is_empty());
        }

        #[test]
        fn file_matches_scala() {
            let scratch = Scratch::new();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            let mem_u = column_names(db.file(), "mem_u");
            assert!(mem_u.contains(&"data".to_string()));
            assert!(mem_u.contains(&"ts".to_string()));
            assert!(mem_u.contains(&"new_id".to_string()));
            let prints = column_names(db.file(), "prints");
            assert!(prints.contains(&"category".to_string()));
            assert!(prints.contains(&"is_cert".to_string()));
            assert_eq!(
                column_type(db.file(), "man_to_user", "uid").to_ascii_lowercase(),
                "integer"
            );
            let prints_unique: i64 = db
                .file()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'prints_unique'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(prints_unique, 0);
            let virtuals: i64 = db
                .file()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE sql LIKE '%virtual table%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(virtuals, 0);
            let indexes: Vec<String> = db
                .file()
                .prepare("SELECT name FROM sqlite_master WHERE type = 'index'")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            for name in [
                "mem_u_uid_index",
                "mem_u_org_id_index",
                "mem_u_code_index",
                "mem_u_barcode_index",
                "mem_u_repl_barcode_index",
                "mem_u_in_synch_index",
            ] {
                assert!(indexes.iter().any(|index| index == name), "{name}");
            }
        }

        #[test]
        fn memory_indexes() {
            let scratch = Scratch::new();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            let index_sql: Vec<String> = db
                .memory()
                .prepare(
                    "SELECT sql FROM sqlite_master
                     WHERE type = 'index' AND tbl_name = 'list_row' AND sql IS NOT NULL",
                )
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(index_sql.iter().any(|sql| sql.contains("category")));
            assert!(index_sql.iter().any(|sql| sql.contains("added_ts")));
            assert!(index_sql.iter().any(|sql| sql.contains("last_print_ts")));
            let fts: String = db
                .memory()
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name = 'u_fts'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let fts = fts.to_ascii_lowercase();
            assert!(fts.contains("name"));
            assert!(fts.contains("surname"));
            assert!(fts.contains("c_name"));
            assert!(!fts.contains("category"));
            assert!(db.memory_uri().contains("mode=memory"));
            assert!(db.memory_uri().contains("cache=shared"));
        }

        #[test]
        fn empty_keys() {
            let scratch = Scratch::new();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            let rows = key_rows(&db);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows.iter().filter(|(_, admin)| *admin == 1).count(), 1);
            assert_eq!(rows.iter().filter(|(_, admin)| *admin == 0).count(), 1);
            assert_ne!(rows[0].0, rows[1].0);
            for (key, _) in &rows {
                assert_eq!(key.chars().count(), 7);
                assert!(key.chars().all(|ch| KEY_ALPHABET.contains(ch)));
            }
        }

        #[test]
        fn second_open_keeps_keys() {
            let scratch = Scratch::new();
            let first = EventDb::open(scratch.path(), "EVENT").unwrap();
            let keys = key_rows(&first);
            drop(first);
            let second = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(key_rows(&second), keys);
            assert!(config_file(scratch.path()).is_none());
        }

        #[test]
        fn adds_missing_indexes_and_keeps_rows() {
            let scratch = Scratch::new();
            let path = scratch.path().join("EVENT").join(DB_FILE);
            write_scala_shaped(&path);
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(count(db.file(), "mem_u"), 1);
            let uid: String = db
                .file()
                .query_row("SELECT uid FROM mem_u", [], |row| row.get(0))
                .unwrap();
            assert_eq!(uid, "keep");
            assert_eq!(key_rows(&db), vec![("KEEPME1".to_string(), 1)]);
            assert_eq!(db.index_state().unwrap(), IndexState::Loading);
            assert_eq!(db.loaded_count().unwrap(), 0);
            let code: i64 = db
                .file()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'mem_u_code_index'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(code, 1);
            assert!(config_file(scratch.path()).is_none());
        }

        #[test]
        fn adds_new_id_without_dropping_rows() {
            let scratch = Scratch::new();
            let path = scratch.path().join("EVENT").join(DB_FILE);
            write_scala_shaped(&path);
            let without_new_id =
                FILE_TABLES.replacen("org_id INTEGER,\n  new_id INTEGER", "org_id INTEGER", 1);
            let start = without_new_id
                .find("CREATE TABLE IF NOT EXISTS mem_u")
                .unwrap();
            let end = without_new_id
                .find("CREATE INDEX IF NOT EXISTS mem_u_uid_index")
                .unwrap();
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("DROP TABLE mem_u").unwrap();
            conn.execute_batch(&without_new_id[start..end]).unwrap();
            conn.execute(
                "INSERT INTO mem_u (uid, data, ts) VALUES ('keep', '{\"n\":1}', 10)",
                [],
            )
            .unwrap();
            assert!(!column_names(&conn, "mem_u").contains(&"new_id".to_string()));
            drop(conn);

            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert!(column_names(db.file(), "mem_u").contains(&"new_id".to_string()));
            assert_eq!(count(db.file(), "mem_u"), 1);
            assert!(config_file(scratch.path()).is_none());
        }

        #[test]
        fn adds_sync_cursor_without_dropping_rows() {
            let scratch = Scratch::new();
            let path = scratch.path().join("EVENT").join(DB_FILE);
            write_scala_shaped(&path);
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("DROP TABLE IF EXISTS sync_cursor")
                .unwrap();
            drop(conn);

            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(count(db.file(), "mem_u"), 1);
            let uid: String = db
                .file()
                .query_row("SELECT uid FROM mem_u", [], |row| row.get(0))
                .unwrap();
            assert_eq!(uid, "keep");
            assert_eq!(count(db.file(), "sync_cursor"), 0);
            assert_eq!(db.cursor("zones").unwrap(), "0");
        }

        #[test]
        fn switch_keeps_the_previous_file() {
            let scratch = Scratch::new();
            let mut db = EventDb::open(scratch.path(), "AAA").unwrap();
            db.file()
                .execute("INSERT INTO mem_u (uid, ts) VALUES ('keep', 1)", [])
                .unwrap();
            let keys = key_rows(&db);
            insert_list_row(&db, "keep");
            db.switch("BBB").unwrap();
            assert_eq!(db.event_id(), "BBB");
            assert_eq!(count(db.file(), "mem_u"), 0);
            assert_eq!(count(db.memory(), "list_row"), 0);
            assert_eq!(db.index_state().unwrap(), IndexState::Ready);

            let previous = Connection::open(scratch.path().join("AAA").join(DB_FILE)).unwrap();
            let uid: String = previous
                .query_row("SELECT uid FROM mem_u", [], |row| row.get(0))
                .unwrap();
            assert_eq!(uid, "keep");
            let mut stmt = previous
                .prepare("SELECT key, is_admin FROM keys ORDER BY is_admin DESC, key")
                .unwrap();
            let previous_keys: Vec<(String, i64)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(previous_keys, keys);
            assert!(config_file(scratch.path()).is_none());
        }

        #[test]
        fn two_events_do_not_share_memory() {
            let scratch = Scratch::new();
            let first = EventDb::open(scratch.path(), "AAA").unwrap();
            insert_list_row(&first, "keep");
            let second = EventDb::open(scratch.path(), "BBB").unwrap();
            assert_ne!(first.memory_uri(), second.memory_uri());
            assert_eq!(count(first.memory(), "list_row"), 1);
            assert_eq!(count(second.memory(), "list_row"), 0);
        }

        #[test]
        fn switch_to_the_same_event_is_a_noop() {
            let scratch = Scratch::new();
            let mut db = EventDb::open(scratch.path(), "AAA").unwrap();
            insert_list_row(&db, "keep");
            let uri = db.memory_uri().to_string();
            db.switch("AAA").unwrap();
            assert_eq!(db.memory_uri(), uri);
            assert_eq!(count(db.memory(), "list_row"), 1);
            assert!(config_file(scratch.path()).is_none());
        }

        fn assert_bad_event_id(base: &Path, event_id: &str) {
            match EventDb::open(base, event_id) {
                Err(EventDbError::BadExpoId) => {}
                Err(err) => panic!("expected a rejected event id, got {err}"),
                Ok(_) => panic!("expected a rejected event id"),
            }
        }

        #[test]
        fn rejects_an_event_id_that_is_not_one_path_component() {
            let scratch = Scratch::new();
            assert_bad_event_id(scratch.path(), "");
            assert_bad_event_id(scratch.path(), ".");
            assert_bad_event_id(scratch.path(), "..");
            assert_bad_event_id(scratch.path(), "a/b");
            assert_bad_event_id(scratch.path(), "a\\b");
        }

        #[test]
        fn broken_file_is_restored_and_marked() {
            let scratch = Scratch::new();
            let dir = scratch.path().join("EVENT");
            fs::create_dir_all(&dir).unwrap();
            let live = dir.join(DB_FILE);
            fs::write(&live, b"not a database").unwrap();
            fs::write(dir.join("db.sqlite-wal"), b"old-wal").unwrap();
            fs::write(dir.join("db.sqlite-shm"), b"old-shm").unwrap();

            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(count(db.file(), "mem_u"), 0);
            assert_eq!(db.index_state().unwrap(), IndexState::Ready);
            assert_eq!(key_rows(&db).len(), 2);
            let check: String = db
                .file()
                .query_row("PRAGMA quick_check", [], |row| row.get(0))
                .unwrap();
            assert_eq!(check, "ok");
            assert!(db.needs_full_reload().unwrap());
            let config = config_file(scratch.path()).unwrap();
            assert_eq!(config.version, 1);
            assert_eq!(config.full_reload, vec!["EVENT".to_string()]);
            let saved = backups(&dir);
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].0, 1);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis();
            assert!(saved[0].1 <= now);
            assert!(now - saved[0].1 < 60_000);
            assert_eq!(
                fs::read(saved[0].2.join(DB_FILE)).unwrap(),
                b"not a database"
            );
            assert_eq!(
                fs::read(saved[0].2.join("db.sqlite-wal")).unwrap(),
                b"old-wal"
            );
            assert_eq!(
                fs::read(saved[0].2.join("db.sqlite-shm")).unwrap(),
                b"old-shm"
            );
            let wal = fs::read(dir.join("db.sqlite-wal")).unwrap_or_default();
            assert!(!wal.windows(7).any(|window| window == b"old-wal"));
            assert_ne!(fs::read(&live).unwrap(), b"not a database");
        }

        #[test]
        fn corrupt_sqlite_header_keeps_sidecar_bytes() {
            let scratch = Scratch::new();
            let dir = scratch.path().join("EVENT");
            fs::create_dir_all(&dir).unwrap();
            let mut bytes = b"SQLite format 3\0".to_vec();
            bytes.resize(200, 0);
            fs::write(dir.join(DB_FILE), &bytes).unwrap();
            fs::write(dir.join("db.sqlite-wal"), b"old-wal").unwrap();
            fs::write(dir.join("db.sqlite-shm"), b"old-shm").unwrap();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(count(db.file(), "mem_u"), 0);
            let saved = backups(&dir);
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].0, 1);
            assert_eq!(fs::read(saved[0].2.join(DB_FILE)).unwrap(), bytes);
            assert_eq!(
                fs::read(saved[0].2.join("db.sqlite-wal")).unwrap(),
                b"old-wal"
            );
            assert_eq!(
                fs::read(saved[0].2.join("db.sqlite-shm")).unwrap(),
                b"old-shm"
            );
            assert!(db.needs_full_reload().unwrap());
            assert!(fs::read_dir(&dir).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".sidecar-")
            }));
        }

        #[test]
        fn truncated_header_is_restored_and_marked() {
            let scratch = Scratch::new();
            let dir = scratch.path().join("EVENT");
            fs::create_dir_all(&dir).unwrap();
            let mut bytes = b"SQLite format 3\0".to_vec();
            bytes.resize(64, 0);
            fs::write(dir.join(DB_FILE), &bytes).unwrap();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(count(db.file(), "mem_u"), 0);
            assert_eq!(
                config_file(scratch.path()).unwrap().full_reload,
                vec!["EVENT".to_string()]
            );
            let saved = backups(&dir);
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].0, 1);
            assert_eq!(fs::read(saved[0].2.join(DB_FILE)).unwrap(), bytes);
        }

        #[test]
        fn missing_table_is_restored_and_marked() {
            let scratch = Scratch::new();
            let path = scratch.path().join("EVENT").join(DB_FILE);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE junk (id INTEGER)")
                .unwrap();
            drop(conn);
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(count(db.file(), "mem_u"), 0);
            let junk: i64 = db
                .file()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'junk'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(junk, 0);
            let saved = backups(path.parent().unwrap());
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].0, 1);
            let archived = Connection::open(saved[0].2.join(DB_FILE)).unwrap();
            let archived_junk: i64 = archived
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'junk'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(archived_junk, 1);
            assert_eq!(
                config_file(scratch.path()).unwrap().full_reload,
                vec!["EVENT".to_string()]
            );
        }

        #[test]
        fn reload_mark_lists_the_event_once() {
            let scratch = Scratch::new();
            let live = scratch.path().join("EVENT").join(DB_FILE);
            fs::create_dir_all(live.parent().unwrap()).unwrap();
            fs::write(&live, b"not a database").unwrap();
            drop(EventDb::open(scratch.path(), "EVENT").unwrap());
            fs::write(&live, b"broken again").unwrap();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(count(db.file(), "mem_u"), 0);
            assert_eq!(
                config_file(scratch.path()).unwrap().full_reload,
                vec!["EVENT".to_string()]
            );
            let saved = backups(live.parent().unwrap());
            assert_eq!(saved.len(), 2);
            assert_eq!((saved[0].0, saved[1].0), (1, 2));
            assert!(saved[1].1 >= saved[0].1);
            assert_eq!(
                fs::read(saved[0].2.join(DB_FILE)).unwrap(),
                b"not a database"
            );
            assert_eq!(fs::read(saved[1].2.join(DB_FILE)).unwrap(), b"broken again");
        }

        #[test]
        fn creating_leftover_is_restored_and_marked() {
            let scratch = Scratch::new();
            let dir = scratch.path().join("EVENT");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("db.sqlite.creating"), b"partial").unwrap();
            fs::write(dir.join("db.sqlite.creating-wal"), b"partial-wal").unwrap();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert!(db.file_path().is_file());
            assert!(!dir.join("db.sqlite.creating").exists());
            assert!(!dir.join("db.sqlite.creating-wal").exists());
            let saved = backups(&dir);
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].0, 1);
            assert_eq!(
                fs::read(saved[0].2.join("db.sqlite.creating")).unwrap(),
                b"partial"
            );
            assert_eq!(
                fs::read(saved[0].2.join("db.sqlite.creating-wal")).unwrap(),
                b"partial-wal"
            );
            assert_eq!(count(db.file(), "mem_u"), 0);
            assert_eq!(db.index_state().unwrap(), IndexState::Ready);
            assert_eq!(
                config_file(scratch.path()).unwrap().full_reload,
                vec!["EVENT".to_string()]
            );
        }

        #[test]
        fn creating_leftover_keeps_a_good_file() {
            let scratch = Scratch::new();
            let first = EventDb::open(scratch.path(), "EVENT").unwrap();
            let keys = key_rows(&first);
            first
                .file()
                .execute("INSERT INTO mem_u (uid, ts) VALUES ('keep', 1)", [])
                .unwrap();
            drop(first);
            let dir = scratch.path().join("EVENT");
            fs::write(dir.join("db.sqlite.creating"), b"partial").unwrap();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            assert_eq!(key_rows(&db), keys);
            assert_eq!(count(db.file(), "mem_u"), 1);
            assert!(!dir.join("db.sqlite.creating").exists());
            let saved = backups(&dir);
            assert_eq!(saved.len(), 1);
            assert_eq!(
                fs::read(saved[0].2.join("db.sqlite.creating")).unwrap(),
                b"partial"
            );
            assert!(config_file(scratch.path()).is_none());
            assert!(!db.needs_full_reload().unwrap());
        }

        #[test]
        fn switch_of_a_broken_event_keeps_the_current_file() {
            let scratch = Scratch::new();
            let mut db = EventDb::open(scratch.path(), "AAA").unwrap();
            db.file()
                .execute("INSERT INTO mem_u (uid, ts) VALUES ('keep', 1)", [])
                .unwrap();
            let broken = scratch.path().join("BBB");
            fs::create_dir_all(&broken).unwrap();
            fs::write(broken.join(DB_FILE), b"not a database").unwrap();
            db.switch("BBB").unwrap();
            assert_eq!(db.event_id(), "BBB");
            assert_eq!(count(db.file(), "mem_u"), 0);
            let previous = Connection::open(scratch.path().join("AAA").join(DB_FILE)).unwrap();
            let uid: String = previous
                .query_row("SELECT uid FROM mem_u", [], |row| row.get(0))
                .unwrap();
            assert_eq!(uid, "keep");
            let saved = backups(&broken);
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].0, 1);
            assert_eq!(
                fs::read(saved[0].2.join(DB_FILE)).unwrap(),
                b"not a database"
            );
            assert!(backups(&scratch.path().join("AAA")).is_empty());
            assert_eq!(
                config_file(scratch.path()).unwrap().full_reload,
                vec!["BBB".to_string()]
            );
        }

        #[test]
        fn switch_does_not_clear_the_reload_mark() {
            let scratch = Scratch::new();
            let live = scratch.path().join("AAA").join(DB_FILE);
            fs::create_dir_all(live.parent().unwrap()).unwrap();
            fs::write(&live, b"not a database").unwrap();
            let mut db = EventDb::open(scratch.path(), "AAA").unwrap();
            db.switch("BBB").unwrap();
            assert_eq!(db.event_id(), "BBB");
            assert_eq!(count(db.file(), "mem_u"), 0);
            assert_eq!(
                config_file(scratch.path()).unwrap().full_reload,
                vec!["AAA".to_string()]
            );
            assert!(db.needs_full_reload().unwrap());
        }

        #[test]
        fn failed_mark_does_not_drop_or_switch() {
            let scratch = Scratch::new();
            let mut db = EventDb::open(scratch.path(), "AAA").unwrap();
            db.file()
                .execute("INSERT INTO mem_u (uid, ts) VALUES ('keep', 1)", [])
                .unwrap();
            let broken = scratch.path().join("BBB");
            fs::create_dir_all(&broken).unwrap();
            fs::write(broken.join(DB_FILE), b"not a database").unwrap();
            fs::create_dir(scratch.path().join(CONFIG_FILE)).unwrap();
            let err = match db.switch("BBB") {
                Err(err) => err,
                Ok(()) => panic!("expected switch to fail before deleting the broken file"),
            };
            assert!(matches!(
                err,
                EventDbError::Io { .. } | EventDbError::Config { .. }
            ));
            assert_eq!(db.event_id(), "AAA");
            assert_eq!(count(db.file(), "mem_u"), 1);
            assert_eq!(fs::read(broken.join(DB_FILE)).unwrap(), b"not a database");
            assert!(backups(&broken).is_empty());
        }
    }

    mod load {
        use super::super::{EventDb, IndexState, ListOrder, DB_FILE, FILE_TABLES};
        use rusqlite::Connection;
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        const FIXTURE: &str = "/Users/mac/Documents/development/JJO/Irbis/temp-reg/registration_x86/base/GELCPAQSFL/db.sqlite";

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(1);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-event-load-{}-{}-{n}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn open_fixture() -> (Scratch, EventDb) {
            let scratch = Scratch::new();
            let src = Path::new(FIXTURE);
            assert!(
                src.is_file(),
                "scala event database is missing at {}",
                src.display()
            );
            let event = scratch.path().join("GELCPAQSFL");
            fs::create_dir_all(&event).unwrap();
            fs::copy(src, event.join(DB_FILE)).unwrap();
            let db = EventDb::open(scratch.path(), "GELCPAQSFL").unwrap();
            (scratch, db)
        }

        fn finish(db: &mut EventDb) {
            while db.load_batch().unwrap() {}
        }

        fn list_count(db: &EventDb, uid: &str) -> i64 {
            db.memory()
                .query_row(
                    "SELECT COUNT(*) FROM list_row WHERE uid = ?1",
                    [uid],
                    |row| row.get(0),
                )
                .unwrap()
        }

        #[test]
        fn first_page_before_the_rest() {
            let source_len = fs::metadata(FIXTURE).unwrap().len();
            let (_scratch, mut db) = open_fixture();
            assert_eq!(db.index_state().unwrap(), IndexState::Loading);
            assert_eq!(db.loaded_count().unwrap(), 0);
            let order = super::last_add(&db);
            assert!(db.load_batch().unwrap());
            assert_eq!(db.loaded_count().unwrap(), 100);
            assert_eq!(db.index_state().unwrap(), IndexState::Loading);
            let first: String = db
                .memory()
                .query_row(
                    "SELECT uid FROM list_row ORDER BY rowid LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(first, order[0].0);
            assert_eq!(list_count(&db, &order[99].0), 1);
            assert_eq!(list_count(&db, &order[100].0), 0);
            let loaded: Vec<String> = {
                let mut stmt = db
                    .memory()
                    .prepare("SELECT uid FROM list_row ORDER BY added_ts DESC, uid")
                    .unwrap();
                stmt.query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            let page = db
                .table(&[], ListOrder::LastAdd, Some(100), Some(0))
                .unwrap();
            let shown: Vec<String> = page.rows.into_iter().map(|row| row.uid).collect();
            let expected: Vec<String> = order.iter().take(100).map(|row| row.0.clone()).collect();
            assert_eq!(loaded, shown);
            assert_eq!(loaded, expected);
            finish(&mut db);
            assert_eq!(db.loaded_count().unwrap(), 5176);
            assert_eq!(db.index_state().unwrap(), IndexState::Ready);
            assert_eq!(list_count(&db, &order[100].0), 1);
            let email: String = db
                .memory()
                .query_row("SELECT email FROM list_row WHERE uid = '732'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(email, "alfa23@inbox.ru");
            let (prints, printed): (i64, i64) = db
                .memory()
                .query_row(
                    "SELECT print_count, last_print_ts FROM list_row WHERE uid = '249'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(prints, 1);
            assert_eq!(printed, 1790784637258);
            let (gotsome, category): (i64, i64) = db
                .memory()
                .query_row(
                    "SELECT gotsome, category FROM list_row WHERE uid = '733'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(gotsome, 0);
            assert_eq!(category, 7);
            assert!(!db.load_batch().unwrap());
            assert_eq!(fs::metadata(FIXTURE).unwrap().len(), source_len);
        }

        #[test]
        fn existing_keys_stay() {
            let (_scratch, mut db) = open_fixture();
            let keys = |db: &EventDb| -> Vec<(String, i64)> {
                let mut stmt = db
                    .file()
                    .prepare("SELECT key, is_admin FROM keys ORDER BY is_admin DESC, key")
                    .unwrap();
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            assert_eq!(
                keys(&db),
                vec![("admin".to_string(), 1), ("IPTB2PA".to_string(), 0)]
            );
            finish(&mut db);
            assert_eq!(
                keys(&db),
                vec![("admin".to_string(), 1), ("IPTB2PA".to_string(), 0)]
            );
        }

        #[test]
        fn skip_uid_already_loaded() {
            let (_scratch, mut db) = open_fixture();
            db.memory()
                .execute(
                    "INSERT INTO list_row (
                        uid, name, surname, c_name, category, ticket_status, gotsome,
                        give_packet, added_ts, last_print_ts, print_count, org_id, email
                    ) VALUES ('733', 'already', '', '', 7, 0, 0, 0, 1, 0, 0, NULL, '')",
                    [],
                )
                .unwrap();
            finish(&mut db);
            assert_eq!(list_count(&db, "733"), 1);
            let name: String = db
                .memory()
                .query_row("SELECT name FROM list_row WHERE uid = '733'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(name, "already");
            assert_eq!(db.loaded_count().unwrap(), 5176);
            assert_eq!(db.index_state().unwrap(), IndexState::Ready);
        }

        #[test]
        fn short_file_commits_once() {
            let scratch = Scratch::new();
            let path = scratch.path().join("SMALL").join(DB_FILE);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(FILE_TABLES).unwrap();
            for (uid, ts) in [("3", 30), ("2", 20), ("1", 10)] {
                conn.execute(
                    "INSERT INTO mem_u (uid, ts, category) VALUES (?1, ?2, 1)",
                    rusqlite::params![uid, ts],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO u_fulltext (uid, name, surname, c_name) VALUES (?1, ?1, '', '')",
                    [uid],
                )
                .unwrap();
            }
            drop(conn);
            let mut db = EventDb::open(scratch.path(), "SMALL").unwrap();
            assert!(!db.load_batch().unwrap());
            assert_eq!(db.loaded_count().unwrap(), 3);
            assert_eq!(db.index_state().unwrap(), IndexState::Ready);
            let first: String = db
                .memory()
                .query_row(
                    "SELECT uid FROM list_row ORDER BY added_ts DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(first, "3");
        }

        #[test]
        fn tied_timestamp_uses_table_uid_order() {
            let scratch = Scratch::new();
            let path = scratch.path().join("TIE").join(DB_FILE);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(FILE_TABLES).unwrap();
            for n in 1..=99 {
                let uid = format!("{n:03}");
                let ts = 1000 - n;
                conn.execute(
                    "INSERT INTO mem_u (uid, ts, category) VALUES (?1, ?2, 1)",
                    rusqlite::params![uid, ts],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO u_fulltext (uid, name, surname, c_name) VALUES (?1, ?1, '', '')",
                    [&uid],
                )
                .unwrap();
            }
            for uid in ["a", "b"] {
                conn.execute(
                    "INSERT INTO mem_u (uid, ts, category) VALUES (?1, 10, 1)",
                    [uid],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO u_fulltext (uid, name, surname, c_name) VALUES (?1, ?1, '', '')",
                    [uid],
                )
                .unwrap();
            }
            drop(conn);
            let mut db = EventDb::open(scratch.path(), "TIE").unwrap();
            assert!(db.load_batch().unwrap());
            assert_eq!(db.loaded_count().unwrap(), 100);
            assert_eq!(list_count(&db, "a"), 1);
            assert_eq!(list_count(&db, "b"), 0);
            let page = db
                .table(&[], ListOrder::LastAdd, Some(2), Some(99))
                .unwrap();
            assert_eq!(page.rows[0].uid, "a");
            assert_eq!(page.rows[1].uid, "b");
            finish(&mut db);
            let tail: Vec<String> = {
                let mut stmt = db
                    .memory()
                    .prepare(
                        "SELECT uid FROM list_row WHERE added_ts = 10 ORDER BY added_ts DESC, uid",
                    )
                    .unwrap();
                stmt.query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            assert_eq!(tail, vec!["a".to_string(), "b".to_string()]);
            assert_eq!(db.loaded_count().unwrap(), 101);
        }
    }

    mod search {
        use super::super::{
            fts_phrase, list_select_sql, match_query, EventDb, ListOrder, TextQuery, DB_FILE,
        };
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        const FIXTURE: &str = "/Users/mac/Documents/development/JJO/Irbis/temp-reg/registration_x86/base/GELCPAQSFL/db.sqlite";

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(1);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-event-search-{}-{}-{n}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn open_fixture() -> (Scratch, EventDb) {
            let scratch = Scratch::new();
            let src = Path::new(FIXTURE);
            assert!(
                src.is_file(),
                "scala event database is missing at {}",
                src.display()
            );
            let event = scratch.path().join("GELCPAQSFL");
            fs::create_dir_all(&event).unwrap();
            fs::copy(src, event.join(DB_FILE)).unwrap();
            let db = EventDb::open(scratch.path(), "GELCPAQSFL").unwrap();
            (scratch, db)
        }

        fn finish(db: &mut EventDb) {
            while db.load_batch().unwrap() {}
        }

        fn ready() -> (Scratch, EventDb) {
            let (scratch, mut db) = open_fixture();
            finish(&mut db);
            (scratch, db)
        }

        fn uids(db: &EventDb, text: &str, categories: &[i64]) -> Vec<String> {
            db.search(text, categories, ListOrder::LastAdd, Some(5000), Some(0))
                .unwrap()
                .rows
                .into_iter()
                .map(|row| row.uid)
                .collect()
        }

        fn explain(db: &EventDb, sql: &str) -> String {
            let wrapped = format!("EXPLAIN QUERY PLAN {sql}");
            let mut stmt = db.memory().prepare(&wrapped).unwrap();
            let args = vec![0i64; sql.matches('?').count()];
            let rows = stmt
                .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap();
            rows.map(|row| row.unwrap()).collect::<Vec<_>>().join("\n")
        }

        #[test]
        fn match_name() {
            let (_scratch, db) = ready();
            let prefix = uids(&db, "иль", &[]);
            assert!(prefix.contains(&"733".to_string()));
            let folded = uids(&db, "Илья", &[]);
            assert!(folded.contains(&"733".to_string()));
        }

        #[test]
        fn match_surname_and_company() {
            let (_scratch, db) = ready();
            assert_eq!(uids(&db, "агранович", &[]), vec!["733".to_string()]);
            let company = uids(&db, "beautydrugs", &[]);
            assert!(company.contains(&"1".to_string()));
        }

        #[test]
        fn two_words_are_and() {
            let (_scratch, db) = ready();
            assert_eq!(uids(&db, "илья агранович", &[]), vec!["733".to_string()]);
            let name = db
                .search("илья", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert!(name.total > 1);
        }

        #[test]
        fn category_filter() {
            let (_scratch, db) = ready();
            let page = db
                .search("", &[7], ListOrder::LastAdd, Some(3000), None)
                .unwrap();
            assert_eq!(page.total, 2088);
            assert!(page.rows.iter().all(|row| row.uid != "249"));
            let sql = list_select_sql(false, 1, ListOrder::LastAdd);
            assert!(!sql.contains("u_fts"));
            let plan = explain(&db, &sql);
            assert!(plan.contains("list_row"), "{plan}");
            assert!(!plan.contains("u_fts"), "{plan}");
        }

        #[test]
        fn category_and_text() {
            let (_scratch, db) = ready();
            assert_eq!(uids(&db, "агранович", &[7]), vec!["733".to_string()]);
            assert!(uids(&db, "агранович", &[-2]).is_empty());
        }

        #[test]
        fn category_is_not_a_token() {
            let (_scratch, db) = ready();
            let page = db
                .search("7", &[], ListOrder::LastAdd, Some(5000), None)
                .unwrap();
            assert_ne!(page.total, 2088);
            assert!(page.rows.iter().all(|row| row.uid != "733"));
        }

        #[test]
        fn order_last_add() {
            let (_scratch, db) = ready();
            let order = super::last_add(&db);
            let page = db.search("", &[], ListOrder::LastAdd, None, None).unwrap();
            assert_eq!(page.total, 5176);
            assert_eq!(page.rows.len(), 20);
            assert_eq!(page.rows[0].uid, order[0].0);
            let next = db
                .search("   ", &[], ListOrder::LastAdd, None, Some(20))
                .unwrap();
            assert_eq!(next.total, 5176);
            assert_eq!(next.rows[0].uid, order[20].0);
        }

        #[test]
        fn order_last_print() {
            let (_scratch, db) = ready();
            let page = db
                .search("", &[], ListOrder::LastPrint, None, None)
                .unwrap();
            assert_eq!(page.rows[0].uid, "249");
            assert_eq!(page.rows[0].last_print_ts, 1790784637258);
            let newest = db
                .search("агранович", &[], ListOrder::LastPrint, None, None)
                .unwrap();
            assert_eq!(newest.rows[0].uid, "733");
            assert_eq!(newest.rows[0].last_print_ts, 0);
        }

        #[test]
        fn search_sees_committed_only() {
            let (_scratch, mut db) = open_fixture();
            let order = super::last_add(&db);
            let (early_uid, early_name) = super::unique_surname(&order, 0..100);
            let (later_uid, later_name) = super::unique_surname(&order, 100..order.len());
            assert!(db.load_batch().unwrap());
            let past = db
                .search(&later_name, &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(past.total, 0);
            let inside = db
                .search(&early_name, &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert!(inside.rows.iter().any(|row| row.uid == early_uid));
            finish(&mut db);
            let loaded = db
                .search(&later_name, &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert!(loaded.rows.iter().any(|row| row.uid == later_uid));
        }

        #[test]
        fn hostile_text_stays_a_query() {
            assert_eq!(fts_phrase("илья").as_deref(), Some("\"илья\"*"));
            assert_eq!(fts_phrase("a\"b").as_deref(), Some("\"a\"\"b\"*"));
            assert_eq!(fts_phrase("*"), None);
            assert_eq!(fts_phrase("\""), None);
            assert_eq!(fts_phrase("AND").as_deref(), Some("\"AND\"*"));
            assert_eq!(fts_phrase("name:илья").as_deref(), Some("\"name:илья\"*"));
            assert!(matches!(match_query("   "), TextQuery::All));
            assert!(matches!(match_query("*"), TextQuery::None));
            assert!(matches!(match_query("\0"), TextQuery::None));
            match match_query("илья\" OR surname:x") {
                TextQuery::Match(expr) => {
                    assert!(expr.contains("\"илья\"\"\"*"), "{expr}");
                    assert!(expr.contains("\"OR\"*"), "{expr}");
                    assert!(expr.contains("\"surname:x\"*"), "{expr}");
                    assert!(!expr.contains("surname : \"x\""), "{expr}");
                }
                other => panic!("expected a match query, got {other:?}"),
            }
            let sql = list_select_sql(true, 0, ListOrder::LastAdd);
            assert!(sql.contains("u_fts MATCH ?"));
            assert!(!sql.contains("илья"));

            let (_scratch, db) = ready();
            let samples = [
                "\"",
                "'",
                "*",
                ":",
                "(",
                ")",
                "AND",
                "OR",
                "NOT",
                "NEAR",
                "^",
                "\\",
                "%",
                "_",
                "=",
                ";",
                "?",
                ".",
                "илья\"",
                "\"илья",
                "name:илья",
                "(илья)",
                "илья*",
                "илья AND OR NOT",
                "NEAR(илья, 1)",
                "surname:агранович",
                "'; DROP TABLE list_row; --",
                "\" OR 1=1 --",
                "****",
                "a\"b\"c",
                "c_name:BEAUTYDRUGS",
                "\u{0000}",
                "илья\u{0000}агранович",
                "{илья}",
                "[илья]",
                "^^илья",
                "NOT илья",
                "AND OR NOT NEAR",
                "\"\"\"",
                "' OR '1'='1",
                "илья\\",
                "%_%",
                "(((",
                ")))",
                "NEAR(",
                "*\"*",
                "илья агранович\" OR surname:x",
            ];
            for text in samples {
                let page = db.search(text, &[], ListOrder::LastAdd, None, None);
                assert!(page.is_ok(), "{text:?} returned {page:?}");
            }
            assert_eq!(
                db.search("*", &[], ListOrder::LastAdd, None, None)
                    .unwrap()
                    .total,
                0
            );
            assert_eq!(
                db.search("\"", &[], ListOrder::LastAdd, None, None)
                    .unwrap()
                    .total,
                0
            );
            assert_eq!(
                db.search("AND OR NOT NEAR", &[], ListOrder::LastAdd, None, None)
                    .unwrap()
                    .total,
                0
            );
            let still = db
                .search("агранович", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(still.rows[0].uid, "733");
            assert_eq!(db.loaded_count().unwrap(), 5176);
            let tables: i64 = db
                .memory()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'list_row'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(tables, 1);
        }
    }

    mod write {
        use super::super::{EventDb, ListOrder, VisitorWrite, DB_FILE};
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        const FIXTURE: &str = "/Users/mac/Documents/development/JJO/Irbis/temp-reg/registration_x86/base/GELCPAQSFL/db.sqlite";

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(1);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-event-write-{}-{}-{n}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn open_fixture() -> (Scratch, EventDb) {
            let scratch = Scratch::new();
            let src = Path::new(FIXTURE);
            assert!(
                src.is_file(),
                "scala event database is missing at {}",
                src.display()
            );
            let event = scratch.path().join("GELCPAQSFL");
            fs::create_dir_all(&event).unwrap();
            fs::copy(src, event.join(DB_FILE)).unwrap();
            let db = EventDb::open(scratch.path(), "GELCPAQSFL").unwrap();
            (scratch, db)
        }

        fn finish(db: &mut EventDb) {
            while db.load_batch().unwrap() {}
        }

        fn ready() -> (Scratch, EventDb) {
            let (scratch, mut db) = open_fixture();
            finish(&mut db);
            (scratch, db)
        }

        fn visitor(uid: &str, surname: &str, added_ts: i64) -> VisitorWrite {
            VisitorWrite {
                uid: uid.to_string(),
                data: format!(r#"{{"uid":"{uid}"}}"#),
                name: "запись".to_string(),
                surname: surname.to_string(),
                c_name: String::new(),
                category: 7,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts,
                org_id: None,
                email: format!("{uid}@example.com"),
            }
        }

        fn file_count(db: &EventDb, sql: &str) -> i64 {
            db.file().query_row(sql, [], |row| row.get(0)).unwrap()
        }

        #[test]
        fn write_is_searchable() {
            let (_scratch, mut db) = ready();
            let written = visitor("9002", "zzwritetest", 50);
            db.write(&written).unwrap();
            let page = db
                .search("zzwritetest", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(page.total, 1);
            assert_eq!(page.rows[0].uid, "9002");
            assert_eq!(page.rows[0].email, "9002@example.com");
            let data: String = db
                .file()
                .query_row("SELECT data FROM mem_u WHERE uid = '9002'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(data, written.data);
            let email: String = db
                .file()
                .query_row("SELECT email FROM emails WHERE uid = 9002", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(email, "9002@example.com");
            assert_eq!(file_count(&db, "SELECT COUNT(*) FROM mem_u"), 5177);
            assert_eq!(
                file_count(&db, "SELECT COUNT(*) FROM mem_u WHERE uid = '733'"),
                1
            );
            assert_eq!(db.loaded_count().unwrap(), 5177);
        }

        #[test]
        fn write_replaces() {
            let (_scratch, mut db) = ready();
            let mut written = visitor("733", "имятест", 60);
            written.name = "новое".to_string();
            db.write(&written).unwrap();
            let found = db
                .search("имятест", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(found.rows.len(), 1);
            assert_eq!(found.rows[0].uid, "733");
            assert_eq!(found.rows[0].name, "новое");
            assert_eq!(found.rows[0].print_count, 0);
            assert_eq!(found.rows[0].last_print_ts, 0);
            let old = db
                .search("агранович", &[], ListOrder::LastAdd, Some(5000), None)
                .unwrap();
            assert!(old.rows.iter().all(|row| row.uid != "733"));
            assert_eq!(old.total, 0);
            assert_eq!(
                file_count(&db, "SELECT COUNT(*) FROM mem_u WHERE uid = '733'"),
                1
            );
            assert_eq!(
                file_count(&db, "SELECT COUNT(*) FROM u_fulltext WHERE uid = '733'"),
                1
            );
        }

        #[test]
        fn print_changes_order() {
            let (_scratch, mut db) = ready();
            let before = file_count(&db, "SELECT COUNT(*) FROM prints");
            db.record_print("733", 1790784637259).unwrap();
            let page = db
                .search("", &[], ListOrder::LastPrint, None, None)
                .unwrap();
            assert_eq!(page.rows[0].uid, "733");
            assert_eq!(page.rows[0].last_print_ts, 1790784637259);
            assert_eq!(page.rows[0].print_count, 1);
            assert_eq!(file_count(&db, "SELECT COUNT(*) FROM prints"), before + 1);
            assert_eq!(
                file_count(&db, "SELECT COUNT(*) FROM prints WHERE uid = '733'"),
                1
            );
            let (category, is_cert): (Option<i64>, Option<i64>) = db
                .file()
                .query_row(
                    "SELECT category, is_cert FROM prints WHERE uid = '733'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(category, None);
            assert_eq!(is_cert, None);
        }

        #[test]
        fn write_during_load() {
            let (_scratch, mut db) = open_fixture();
            assert!(db.load_batch().unwrap());
            // Older than uid 3535, and still ahead of the load cursor, so a later batch reads this file row and must skip it.
            db.write(&visitor("9001", "вовремя", 1)).unwrap();
            let early = db
                .search("вовремя", &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert_eq!(early.rows.len(), 1);
            assert_eq!(early.rows[0].uid, "9001");
            finish(&mut db);
            assert_eq!(db.loaded_count().unwrap(), 5177);
            let copies: i64 = db
                .memory()
                .query_row(
                    "SELECT COUNT(*) FROM list_row WHERE uid = '9001'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(copies, 1);
            assert_eq!(
                file_count(&db, "SELECT COUNT(*) FROM mem_u WHERE uid = '9001'"),
                1
            );
        }
    }

    mod table {
        use super::super::{EventDb, IndexState, ListOrder, DB_FILE};
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        const FIXTURE: &str = "/Users/mac/Documents/development/JJO/Irbis/temp-reg/registration_x86/base/GELCPAQSFL/db.sqlite";

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(1);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-event-table-{}-{}-{n}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn open_fixture() -> (Scratch, EventDb) {
            let scratch = Scratch::new();
            let src = Path::new(FIXTURE);
            assert!(
                src.is_file(),
                "scala event database is missing at {}",
                src.display()
            );
            let event = scratch.path().join("GELCPAQSFL");
            fs::create_dir_all(&event).unwrap();
            fs::copy(src, event.join(DB_FILE)).unwrap();
            let db = EventDb::open(scratch.path(), "GELCPAQSFL").unwrap();
            (scratch, db)
        }

        fn finish(db: &mut EventDb) {
            while db.load_batch().unwrap() {}
        }

        fn ready() -> (Scratch, EventDb) {
            let (scratch, mut db) = open_fixture();
            finish(&mut db);
            (scratch, db)
        }

        fn list_copies(db: &EventDb, uid: &str) -> i64 {
            db.memory()
                .query_row(
                    "SELECT COUNT(*) FROM list_row WHERE uid = ?1",
                    [uid],
                    |row| row.get(0),
                )
                .unwrap()
        }

        #[test]
        fn first_page() {
            let (_scratch, mut db) = ready();
            let order = super::last_add(&db);
            let page = db
                .table(&[], ListOrder::LastAdd, Some(20), Some(0))
                .unwrap();
            assert_eq!(page.total, 5176);
            assert_eq!(page.rows.len(), 20);
            assert_eq!(page.rows[0].uid, order[0].0);
            assert_eq!(page.rows[19].uid, order[19].0);
        }

        #[test]
        fn second_page() {
            let (_scratch, mut db) = ready();
            let order = super::last_add(&db);
            let page = db.table(&[], ListOrder::LastAdd, None, Some(20)).unwrap();
            assert_eq!(page.rows[0].uid, order[20].0);
            assert_eq!(page.rows.len(), 20);
            assert_eq!(page.total, 5176);
        }

        #[test]
        fn table_category_and_last_print() {
            let (_scratch, mut db) = ready();
            let page = db.table(&[7], ListOrder::LastPrint, None, None).unwrap();
            assert_eq!(page.total, 2088);
            assert!(page.rows.iter().all(|row| row.category == 7));
            assert_eq!(page.rows[0].uid, "1253");
            assert!(page.rows[0].last_print_ts > 0);
            assert!(page.rows.iter().all(|row| row.uid != "733"));
        }

        #[test]
        fn table_columns() {
            let (_scratch, mut db) = ready();
            let order = super::last_add(&db);
            let offset = order
                .iter()
                .position(|(uid, _, _)| uid == "732")
                .expect("visitor 732") as u64;
            let page = db
                .table(&[], ListOrder::LastAdd, Some(1), Some(offset))
                .unwrap();
            assert_eq!(page.rows.len(), 1);
            let row = &page.rows[0];
            assert_eq!(row.uid, "732");
            assert_eq!(row.email, "alfa23@inbox.ru");
            assert_eq!(row.gotsome, 0);
            assert_eq!(row.give_packet, 0);
            assert_eq!(row.print_count, 0);
        }

        #[test]
        fn page_from_and_limit() {
            let (_scratch, mut db) = open_fixture();
            assert_eq!(db.index_state().unwrap(), IndexState::Loading);
            assert_eq!(db.loaded_count().unwrap(), 0);
            let page = db
                .table(&[], ListOrder::LastAdd, Some(10), Some(40))
                .unwrap();
            assert_eq!(page.total, 5176);
            assert_eq!((page.total + 19) / 20, 259);
            let order = super::last_add(&db);
            assert_eq!(page.rows.len(), 10);
            assert_eq!(page.rows[0].uid, order[40].0);
            let last = db
                .table(&[], ListOrder::LastAdd, Some(20), Some(5160))
                .unwrap();
            assert_eq!(last.total, 5176);
            assert_eq!(last.rows.len(), 16);
            let past = db.table(&[], ListOrder::LastAdd, None, Some(5176)).unwrap();
            assert!(past.rows.is_empty());
            assert_eq!(past.total, 5176);
        }

        #[test]
        fn loading_page_and_count_come_from_the_file() {
            let (_scratch, mut db) = open_fixture();
            let printed = db.table(&[], ListOrder::LastPrint, None, None).unwrap();
            assert_eq!(printed.total, 5176);
            assert_eq!(printed.rows[0].uid, "249");
            assert_eq!(printed.rows[0].last_print_ts, 1790784637258);
            assert_eq!(db.loaded_count().unwrap(), 20);
            assert_eq!(list_copies(&db, "249"), 1);
        }

        #[test]
        fn file_row_wins_while_loading() {
            let (_scratch, mut db) = open_fixture();
            db.memory()
                .execute(
                    "INSERT INTO list_row (
                        uid, name, surname, c_name, category, ticket_status, gotsome,
                        give_packet, added_ts, last_print_ts, print_count, org_id, email
                    ) VALUES ('733', 'memory-only', '', '', 7, 0, 0, 0, 1, 0, 0, NULL, '')",
                    [],
                )
                .unwrap();
            let order = super::last_add(&db);
            let page = db.table(&[], ListOrder::LastAdd, None, None).unwrap();
            assert_eq!(page.total, 5176);
            assert_eq!(page.rows[0].uid, order[0].0);
            assert_eq!(page.rows[0].name, order[0].1);
            assert_eq!(page.rows[19].uid, order[19].0);
            let stored: String = db
                .memory()
                .query_row("SELECT name FROM list_row WHERE uid = '733'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(stored, "memory-only");
            let on_page = order.iter().take(20).any(|(uid, _, _)| uid == "733");
            assert_eq!(db.loaded_count().unwrap(), if on_page { 20 } else { 21 });
            assert_eq!(list_copies(&db, &order[20].0), 0);
        }

        #[test]
        fn asked_page_is_indexed() {
            let (_scratch, mut db) = open_fixture();
            let order = super::last_add(&db);
            let (uid, surname) = super::unique_surname(&order, 100..120);
            let page = db
                .table(&[], ListOrder::LastAdd, Some(20), Some(100))
                .unwrap();
            assert_eq!(page.rows[0].uid, order[100].0);
            assert_eq!(page.total, 5176);
            assert_eq!(list_copies(&db, &uid), 1);
            let found = db
                .search(&surname, &[], ListOrder::LastAdd, None, None)
                .unwrap();
            assert!(found.rows.iter().any(|row| row.uid == uid));
            finish(&mut db);
            assert_eq!(db.loaded_count().unwrap(), 5176);
            assert_eq!(list_copies(&db, &uid), 1);
            assert_eq!(db.index_state().unwrap(), IndexState::Ready);
        }

        #[test]
        fn ready_table_reads_memory() {
            let (_scratch, mut db) = ready();
            let order = super::last_add(&db);
            let uid = &order[0].0;
            db.memory()
                .execute(
                    "UPDATE list_row SET name = 'memory-only' WHERE uid = ?1",
                    [uid],
                )
                .unwrap();
            let page = db.table(&[], ListOrder::LastAdd, Some(1), Some(0)).unwrap();
            assert_eq!(page.rows[0].uid, order[0].0);
            assert_eq!(page.rows[0].name, "memory-only");
            let file_name: String = db
                .file()
                .query_row("SELECT name FROM u_fulltext WHERE uid = ?1", [uid], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(file_name, order[0].1);
        }
    }

    mod full_row {
        use super::super::{EventDb, DB_FILE};
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        const FIXTURE: &str = "/Users/mac/Documents/development/JJO/Irbis/temp-reg/registration_x86/base/GELCPAQSFL/db.sqlite";

        static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(1);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rust-reg-event-row-{}-{}-{n}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn open_fixture() -> (Scratch, EventDb) {
            let scratch = Scratch::new();
            let src = Path::new(FIXTURE);
            assert!(
                src.is_file(),
                "scala event database is missing at {}",
                src.display()
            );
            let event = scratch.path().join("GELCPAQSFL");
            fs::create_dir_all(&event).unwrap();
            fs::copy(src, event.join(DB_FILE)).unwrap();
            let db = EventDb::open(scratch.path(), "GELCPAQSFL").unwrap();
            (scratch, db)
        }

        fn file_count(db: &EventDb) -> i64 {
            db.file()
                .query_row("SELECT COUNT(*) FROM mem_u", [], |row| row.get(0))
                .unwrap()
        }

        #[test]
        fn full_row_is_the_json() {
            let (_scratch, mut db) = open_fixture();
            assert_eq!(db.loaded_count().unwrap(), 0);
            let json = db.full_row("733").unwrap().unwrap();
            let stored: String = db
                .file()
                .query_row("SELECT data FROM mem_u WHERE uid = '733'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(json, stored);
            assert!(json.contains("Илья"));
            assert_eq!(db.loaded_count().unwrap(), 0);
            let order = super::last_add(&db);
            assert!(db.load_batch().unwrap());
            let name: String = db
                .memory()
                .query_row(
                    "SELECT name FROM list_row WHERE uid = ?1",
                    [&order[0].0],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(name, order[0].1);
        }

        #[test]
        fn full_row_missing() {
            let (_scratch, db) = open_fixture();
            let before = file_count(&db);
            assert!(db.full_row("missing").unwrap().is_none());
            assert_eq!(file_count(&db), before);
            assert_eq!(db.loaded_count().unwrap(), 0);
        }

        #[test]
        fn list_row_has_no_json() {
            let scratch = Scratch::new();
            let db = EventDb::open(scratch.path(), "EVENT").unwrap();
            let mut stmt = db
                .memory()
                .prepare("SELECT name FROM pragma_table_info('list_row')")
                .unwrap();
            let columns: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(!columns.iter().any(|name| name == "data"));
        }
    }
}
