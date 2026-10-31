//! One LMDB file: the row index, the stored rows, the event log, and the watch record.

use std::ops::Bound;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use heed::types::Bytes;
use heed::{Database, Env, EnvFlags, EnvOpenOptions};
use prost::Message;

use crate::irbis::google_tables::v1::{
    ChangeKind, NamedCell, RowChange, SnapshotReason, SnapshotRecord, StoredRow, WatchRecord,
};
use crate::irbis::table::v1::{value, Cell, Column, Row, Value};
use crate::model::CellValue;
use crate::uid::{content_hash, HashCell};

pub const RETAIN_EVENTS: u64 = 100_000;
const RETAIN_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const RETAIN_SNAPSHOTS: usize = 10;
/// 16 GiB. Address space for the mapping, not bytes reserved on disk.
const DEFAULT_MAP_SIZE: u64 = 16 << 30;
const MAP_SIZE_ENV: &str = "GOOGLE_TABLES_MAP_SIZE";

#[derive(Debug)]
pub enum StoreError {
    Heed(heed::Error),
    Decode(prost::DecodeError),
    BadRecord(&'static str),
    DuplicateUid(String),
    Io(std::io::Error),
    OutOfRange,
    /// The data file is already larger than `GOOGLE_TABLES_MAP_SIZE`.
    MapLarger {
        file_len: u64,
        map_size: u64,
    },
    BadMapSize(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Heed(err) => write!(f, "{err}"),
            StoreError::Decode(err) => write!(f, "{err}"),
            StoreError::BadRecord(what) => write!(f, "bad {what}"),
            StoreError::DuplicateUid(uid) => write!(f, "duplicate uid {uid}"),
            StoreError::Io(err) => write!(f, "{err}"),
            StoreError::OutOfRange => write!(f, "event log is outside the retained window"),
            StoreError::MapLarger { file_len, map_size } => write!(
                f,
                "LMDB file is {file_len} bytes, larger than {MAP_SIZE_ENV} {map_size}"
            ),
            StoreError::BadMapSize(why) => write!(f, "{MAP_SIZE_ENV} {why}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<heed::Error> for StoreError {
    fn from(err: heed::Error) -> Self {
        StoreError::Heed(err)
    }
}

impl From<prost::DecodeError> for StoreError {
    fn from(err: prost::DecodeError) -> Self {
        StoreError::Decode(err)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        StoreError::Io(err)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScannedCell {
    pub name: String,
    pub excel_id: String,
    pub value: CellValue,
    pub exclude_from_hash: bool,
}

#[derive(Debug, Clone)]
pub struct ScannedRow {
    pub uid: String,
    pub sheet_row: u32,
    pub fingerprint: String,
    pub cells: Vec<ScannedCell>,
    /// When false, the cells were not fetched. A matching fingerprint keeps the stored hash.
    pub cells_known: bool,
}

#[derive(Debug, Clone)]
pub struct CommitOptions {
    /// `Some` stores this Drive `version`. `None` leaves the stored version as it is.
    pub version: Option<String>,
    /// The first commit of a watch appends one `SNAPSHOT` instead of one `ADDED` per row.
    pub snapshot: bool,
    pub columns: Vec<Column>,
    pub exclude: Vec<String>,
}

impl Default for CommitOptions {
    fn default() -> Self {
        Self {
            version: None,
            snapshot: false,
            columns: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReport {
    pub last_seq: u64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredSnapshot {
    pub snapshot_seq: u64,
    pub rows: Vec<StoredRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub uid: String,
    pub fingerprint: String,
    pub content_hash: [u8; 16],
    pub sheet_row: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchState {
    pub status: String,
    pub last_seq: u64,
    pub drive_version: String,
}

pub struct Store {
    env: Env,
    idx: Database<Bytes, Bytes>,
    row: Database<Bytes, Bytes>,
    evt: Database<Bytes, Bytes>,
    watch: Database<Bytes, Bytes>,
    ts: Database<Bytes, Bytes>,
    snap: Database<Bytes, Bytes>,
}

/// How much of the configured map the file has not used yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapHeadroom {
    Plenty,
    /// At most 10 percent of the map is free, and more than 5 percent remains.
    Low {
        free_percent: u64,
    },
    /// At most 5 percent of the map is free.
    Critical {
        free_percent: u64,
    },
}

/// `file_len` is the data file in bytes. `map_size` is the configured ceiling.
/// A file larger than the ceiling is an error. Ten percent free is [`MapHeadroom::Low`].
/// Five percent free, and anything below that, is [`MapHeadroom::Critical`].
pub fn map_headroom(file_len: u64, map_size: u64) -> Result<MapHeadroom, StoreError> {
    if map_size == 0 {
        return Err(StoreError::BadMapSize(
            "must be a positive byte count".into(),
        ));
    }
    if file_len > map_size {
        return Err(StoreError::MapLarger { file_len, map_size });
    }
    let free_percent = (map_size - file_len) * 100 / map_size;
    if free_percent <= 5 {
        Ok(MapHeadroom::Critical { free_percent })
    } else if free_percent <= 10 {
        Ok(MapHeadroom::Low { free_percent })
    } else {
        Ok(MapHeadroom::Plenty)
    }
}

fn map_size_from_env() -> Result<usize, StoreError> {
    let raw = match std::env::var(MAP_SIZE_ENV) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return align_map_size(DEFAULT_MAP_SIZE),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(StoreError::BadMapSize("is not unicode".into()));
        }
    };
    let text = raw.trim();
    if text.is_empty() {
        return align_map_size(DEFAULT_MAP_SIZE);
    }
    let bytes: u64 = text
        .parse()
        .map_err(|_| StoreError::BadMapSize("must be a positive byte count".into()))?;
    if bytes == 0 {
        return Err(StoreError::BadMapSize(
            "must be a positive byte count".into(),
        ));
    }
    align_map_size(bytes)
}

fn align_map_size(bytes: u64) -> Result<usize, StoreError> {
    let page = os_page_size() as u64;
    if page == 0 {
        return Err(StoreError::BadMapSize("page size is zero".into()));
    }
    let aligned = bytes
        .checked_add(page - 1)
        .ok_or_else(|| StoreError::BadMapSize("is too large for this process".into()))?
        / page
        * page;
    usize::try_from(aligned)
        .map_err(|_| StoreError::BadMapSize("is too large for this process".into()))
}

fn os_page_size() -> usize {
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page <= 0 {
        4096
    } else {
        page as usize
    }
}

fn check_map_file(path: &Path, map_size: usize) -> Result<(), StoreError> {
    let file_len = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
        Err(err) => return Err(err.into()),
    };
    match map_headroom(file_len, map_size as u64)? {
        MapHeadroom::Plenty => {}
        MapHeadroom::Low { free_percent } => {
            log::warn!(
                "LMDB file {} has {free_percent}% of its map free ({file_len} of {map_size} bytes)",
                path.display()
            );
        }
        MapHeadroom::Critical { free_percent } => {
            log::warn!(
                "CRITICAL: LMDB file {} has {free_percent}% of its map free ({file_len} of {map_size} bytes)",
                path.display()
            );
        }
    }
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let map_size = map_size_from_env()?;
        Self::open_with_map_size(path, map_size)
    }

    pub fn open_with_map_size(path: &Path, map_size: usize) -> Result<Self, StoreError> {
        let map_size = align_map_size(map_size as u64)?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        check_map_file(path, map_size)?;
        let mut options = EnvOpenOptions::new();
        options.map_size(map_size).max_dbs(8);
        let env = unsafe {
            options.flags(EnvFlags::NO_SUB_DIR);
            options.open(path)?
        };
        let mut wtxn = env.write_txn()?;
        let idx = env.create_database(&mut wtxn, Some("idx"))?;
        let row = env.create_database(&mut wtxn, Some("row"))?;
        let evt = env.create_database(&mut wtxn, Some("evt"))?;
        let watch = env.create_database(&mut wtxn, Some("watch"))?;
        let ts = env.create_database(&mut wtxn, Some("ts"))?;
        let snap = env.create_database(&mut wtxn, Some("snap"))?;
        wtxn.commit()?;
        Ok(Self {
            env,
            idx,
            row,
            evt,
            watch,
            ts,
            snap,
        })
    }

    pub fn commit(
        &self,
        wid: u32,
        scan: &[ScannedRow],
        options: CommitOptions,
    ) -> Result<CommitReport, StoreError> {
        let mut scanned = scan.to_vec();
        scanned.sort_by(|left, right| left.uid.cmp(&right.uid));
        for pair in scanned.windows(2) {
            if pair[0].uid == pair[1].uid {
                return Err(StoreError::DuplicateUid(pair[0].uid.clone()));
            }
        }

        let mut wtxn = self.env.write_txn()?;
        let stored = self.load_index(&wtxn, wid)?;
        let plan = plan_merge(&scanned, &stored);
        let mut record = self.load_watch(&wtxn, wid)?;
        let stored_count = stored.len() as u64;
        let delete_count = plan.deletes.len() as u64;
        if stored_count >= 10 && delete_count * 10 > stored_count * 3 {
            let rows = self.load_rows(&wtxn, wid)?;
            self.put_snapshot(&mut wtxn, wid, SnapshotReason::MassiveDelete, rows)?;
        }

        let collapse = options.snapshot
            && plan.deletes.is_empty()
            && !plan.upserts.is_empty()
            && plan
                .upserts
                .iter()
                .all(|change| change.kind == UpsertKind::Added);
        if collapse {
            let rows = plan
                .upserts
                .iter()
                .map(|change| stored_from_scan(change.row))
                .collect();
            self.put_snapshot(&mut wtxn, wid, SnapshotReason::Initial, rows)?;
        }
        let mut seq = record.last_seq;
        for delete in &plan.deletes {
            let key = entry_key(wid, &delete.uid);
            let stored_row = self
                .row
                .get(&wtxn, &key)?
                .map(StoredRow::decode)
                .transpose()?
                .unwrap_or_else(|| StoredRow {
                    uid: delete.uid.clone(),
                    sheet_row: delete.sheet_row,
                    cells: Vec::new(),
                });
            seq += 1;
            let event = RowChange {
                seq,
                kind: ChangeKind::Deleted as i32,
                uid: delete.uid.clone(),
                row: Some(row_from_stored(&stored_row)),
                content_hash: delete.content_hash.to_vec(),
                changed_columns: Vec::new(),
                columns: Vec::new(),
            };
            self.put_event(&mut wtxn, wid, seq, &event)?;
            self.idx.delete(&mut wtxn, &key)?;
            self.row.delete(&mut wtxn, &key)?;
        }
        for change in &plan.upserts {
            let key = entry_key(wid, &change.row.uid);
            let hash = if change.row.cells_known {
                content_hash(&hash_cells(&change.row.cells))
            } else {
                change.previous_hash
            };
            match change.kind {
                UpsertKind::Added | UpsertKind::Updated => {
                    let previous = if change.kind == UpsertKind::Updated {
                        self.row
                            .get(&wtxn, &key)?
                            .map(StoredRow::decode)
                            .transpose()?
                    } else {
                        None
                    };
                    let changed_columns = if change.kind == UpsertKind::Updated {
                        changed_names(
                            previous
                                .as_ref()
                                .map(|row| row.cells.as_slice())
                                .unwrap_or(&[]),
                            &change.row.cells,
                            &options.exclude,
                        )
                    } else {
                        Vec::new()
                    };
                    let stored_row = stored_from_scan(change.row);
                    self.idx.put(
                        &mut wtxn,
                        &key,
                        &encode_index(&change.row.fingerprint, &hash, change.row.sheet_row),
                    )?;
                    self.row.put(&mut wtxn, &key, &stored_row.encode_to_vec())?;
                    if collapse && change.kind == UpsertKind::Added {
                        continue;
                    }
                    seq += 1;
                    let kind = if change.kind == UpsertKind::Added {
                        ChangeKind::Added
                    } else {
                        ChangeKind::Updated
                    };
                    let event = RowChange {
                        seq,
                        kind: kind as i32,
                        uid: change.row.uid.clone(),
                        row: Some(row_from_scan(change.row)),
                        content_hash: hash.to_vec(),
                        changed_columns,
                        columns: Vec::new(),
                    };
                    self.put_event(&mut wtxn, wid, seq, &event)?;
                }
                UpsertKind::Moved => {
                    self.idx.put(
                        &mut wtxn,
                        &key,
                        &encode_index(&change.row.fingerprint, &hash, change.row.sheet_row),
                    )?;
                    if let Some(mut stored_row) = self
                        .row
                        .get(&wtxn, &key)?
                        .map(StoredRow::decode)
                        .transpose()?
                    {
                        stored_row.sheet_row = change.row.sheet_row;
                        self.row.put(&mut wtxn, &key, &stored_row.encode_to_vec())?;
                    }
                }
            }
        }
        if collapse {
            seq += 1;
            let event = RowChange {
                seq,
                kind: ChangeKind::Snapshot as i32,
                uid: String::new(),
                row: None,
                content_hash: Vec::new(),
                changed_columns: Vec::new(),
                columns: options.columns.clone(),
            };
            self.put_event(&mut wtxn, wid, seq, &event)?;
        }

        record.last_seq = seq;
        record.status = "running".into();
        if let Some(version) = options.version {
            record.drive_version = version;
        }
        if !options.columns.is_empty() {
            record.columns = options.columns;
            record.exclude_from_hash = options.exclude;
        }
        self.watch
            .put(&mut wtxn, &watch_key(wid), &record.encode_to_vec())?;
        wtxn.commit()?;
        Ok(CommitReport {
            last_seq: seq,
            status: record.status,
        })
    }

    pub fn read_stored(&self, wid: u32) -> Result<StoredSnapshot, StoreError> {
        let rtxn = self.env.read_txn()?;
        let record = self.load_watch(&rtxn, wid)?;
        let mut rows = Vec::new();
        for pair in self.row.prefix_iter(&rtxn, &wid.to_be_bytes())? {
            let (_key, value) = pair?;
            rows.push(StoredRow::decode(value)?);
        }
        rows.sort_by(|left, right| left.uid.cmp(&right.uid));
        Ok(StoredSnapshot {
            snapshot_seq: record.last_seq,
            rows,
        })
    }

    pub fn changes(&self, wid: u32) -> Result<Vec<RowChange>, StoreError> {
        let rtxn = self.env.read_txn()?;
        let mut events = Vec::new();
        for pair in self.evt.prefix_iter(&rtxn, &wid.to_be_bytes())? {
            let (_key, value) = pair?;
            events.push(RowChange::decode(value)?);
        }
        Ok(events)
    }

    pub fn index(&self, wid: u32) -> Result<Vec<IndexEntry>, StoreError> {
        let rtxn = self.env.read_txn()?;
        self.load_index(&rtxn, wid)
    }

    pub fn set_status(&self, wid: u32, status: &str) -> Result<(), StoreError> {
        let mut wtxn = self.env.write_txn()?;
        let mut record = self.load_watch(&wtxn, wid)?;
        record.status = status.into();
        self.watch
            .put(&mut wtxn, &watch_key(wid), &record.encode_to_vec())?;
        wtxn.commit()?;
        Ok(())
    }

    /// Pause on a missing mapped column and remember `version` so the loop waits for a newer one.
    pub fn mark_column_missing(&self, wid: u32, version: &str) -> Result<(), StoreError> {
        let mut wtxn = self.env.write_txn()?;
        let mut record = self.load_watch(&wtxn, wid)?;
        record.status = "column_missing".into();
        if !version.is_empty() {
            record.drive_version = version.to_string();
        }
        self.watch
            .put(&mut wtxn, &watch_key(wid), &record.encode_to_vec())?;
        wtxn.commit()?;
        Ok(())
    }

    pub fn watch(&self, wid: u32) -> Result<WatchState, StoreError> {
        let rtxn = self.env.read_txn()?;
        let record = self.load_watch(&rtxn, wid)?;
        Ok(WatchState {
            status: record.status,
            last_seq: record.last_seq,
            drive_version: record.drive_version,
        })
    }

    /// Snapshots for `wid`, oldest first.
    pub fn list_snapshots(&self, wid: u32) -> Result<Vec<SnapshotRecord>, StoreError> {
        let rtxn = self.env.read_txn()?;
        let mut found = Vec::new();
        for pair in self.snap.prefix_iter(&rtxn, &wid.to_be_bytes())? {
            let (key, value) = pair?;
            if key.len() != 12 {
                continue;
            }
            found.push(SnapshotRecord::decode(value)?);
        }
        found.sort_by(|left, right| {
            left.created_at_unix_ms
                .cmp(&right.created_at_unix_ms)
                .then(left.snapshot_id.cmp(&right.snapshot_id))
        });
        Ok(found)
    }

    pub fn get_snapshot(
        &self,
        wid: u32,
        snapshot_id: u64,
    ) -> Result<Option<SnapshotRecord>, StoreError> {
        let rtxn = self.env.read_txn()?;
        match self.snap.get(&rtxn, &snapshot_key(wid, snapshot_id))? {
            Some(bytes) => Ok(Some(SnapshotRecord::decode(bytes)?)),
            None => Ok(None),
        }
    }

    pub fn load_saved(&self, wid: u32) -> Result<Option<WatchRecord>, StoreError> {
        let rtxn = self.env.read_txn()?;
        match self.watch.get(&rtxn, &watch_key(wid))? {
            Some(bytes) => Ok(Some(WatchRecord::decode(bytes)?)),
            None => Ok(None),
        }
    }

    pub fn save_watch(&self, wid: u32, record: &WatchRecord) -> Result<(), StoreError> {
        let mut wtxn = self.env.write_txn()?;
        self.watch
            .put(&mut wtxn, &watch_key(wid), &record.encode_to_vec())?;
        wtxn.commit()?;
        Ok(())
    }

    pub fn watches(&self) -> Result<Vec<(u32, WatchRecord)>, StoreError> {
        let rtxn = self.env.read_txn()?;
        let mut found = Vec::new();
        for pair in self.watch.iter(&rtxn)? {
            let (key, value) = pair?;
            if key.len() != 4 {
                continue;
            }
            let mut wid_bytes = [0u8; 4];
            wid_bytes.copy_from_slice(key);
            found.push((u32::from_be_bytes(wid_bytes), WatchRecord::decode(value)?));
        }
        Ok(found)
    }

    /// Events with a sequence above `after`. A gap in front of the retained log is `OutOfRange`.
    pub fn changes_after(&self, wid: u32, after: u64) -> Result<Vec<RowChange>, StoreError> {
        let rtxn = self.env.read_txn()?;
        let prefix = wid.to_be_bytes();
        let oldest = match self.evt.prefix_iter(&rtxn, &prefix)?.next() {
            Some(pair) => {
                let (key, _) = pair?;
                Some(event_seq(key)?)
            }
            None => None,
        };
        if let Some(oldest) = oldest {
            if oldest > after.saturating_add(1) {
                return Err(StoreError::OutOfRange);
            }
        }
        let Some(start_seq) = after.checked_add(1) else {
            return Ok(Vec::new());
        };
        let start = event_key(wid, start_seq);
        let upper = wid.checked_add(1).map(|next| next.to_be_bytes());
        let end = match upper.as_ref() {
            Some(bytes) => Bound::Excluded(&bytes[..]),
            None => Bound::Unbounded,
        };
        let range = (Bound::Included(&start[..]), end);
        let mut events = Vec::new();
        for pair in self.evt.range(&rtxn, &range)? {
            let (_key, value) = pair?;
            events.push(RowChange::decode(value)?);
        }
        Ok(events)
    }

    /// Drop events outside the retained count or older than seven days.
    ///
    /// The log is a contiguous suffix of `last_seq`. Deletion starts at the oldest key and
    /// stops at the first event that is both inside the retained count and younger than seven days.
    pub fn trim(&self, wid: u32, keep: u64) -> Result<(), StoreError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let age_ms = RETAIN_AGE.as_millis() as u64;
        let doomed = {
            let rtxn = self.env.read_txn()?;
            let prefix = wid.to_be_bytes();
            let oldest = match self.evt.prefix_iter(&rtxn, &prefix)?.next() {
                Some(pair) => {
                    let (key, _) = pair?;
                    event_seq(key)?
                }
                None => return Ok(()),
            };
            let last = self.load_watch(&rtxn, wid)?.last_seq;
            if last < oldest {
                return Ok(());
            }
            let total = last - oldest + 1;
            let mut doomed = Vec::new();
            let mut index = 0u64;
            for pair in self.evt.prefix_iter(&rtxn, &prefix)? {
                let (key, _) = pair?;
                if key.len() != 12 {
                    continue;
                }
                let mut owned = [0u8; 12];
                owned.copy_from_slice(key);
                let inside = total.saturating_sub(index) <= keep;
                if inside {
                    let old = match self.ts.get(&rtxn, &owned)? {
                        Some(bytes) if bytes.len() == 8 => {
                            let stamped = u64::from_be_bytes(bytes.try_into().unwrap());
                            now_ms.saturating_sub(stamped) > age_ms
                        }
                        _ => false,
                    };
                    if !old {
                        break;
                    }
                }
                doomed.push(owned);
                index += 1;
            }
            doomed
        };
        if doomed.is_empty() {
            return Ok(());
        }
        let mut wtxn = self.env.write_txn()?;
        for key in doomed {
            self.evt.delete(&mut wtxn, &key)?;
            self.ts.delete(&mut wtxn, &key)?;
        }
        wtxn.commit()?;
        Ok(())
    }

    fn put_event(
        &self,
        wtxn: &mut heed::RwTxn<'_>,
        wid: u32,
        seq: u64,
        event: &RowChange,
    ) -> Result<(), StoreError> {
        let key = event_key(wid, seq);
        self.evt.put(wtxn, &key, &event.encode_to_vec())?;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.ts.put(wtxn, &key, &millis.to_be_bytes())?;
        Ok(())
    }

    fn load_index(&self, txn: &heed::RoTxn<'_>, wid: u32) -> Result<Vec<IndexEntry>, StoreError> {
        let mut entries = Vec::new();
        for pair in self.idx.prefix_iter(txn, &wid.to_be_bytes())? {
            let (key, value) = pair?;
            let uid = uid_of(wid, key)?;
            let (fingerprint, content_hash, sheet_row) = decode_index(value)?;
            entries.push(IndexEntry {
                uid,
                fingerprint,
                content_hash,
                sheet_row,
            });
        }
        Ok(entries)
    }

    fn load_watch(&self, txn: &heed::RoTxn<'_>, wid: u32) -> Result<WatchRecord, StoreError> {
        match self.watch.get(txn, &watch_key(wid))? {
            Some(bytes) => Ok(WatchRecord::decode(bytes)?),
            None => Ok(WatchRecord {
                status: "running".into(),
                ..WatchRecord::default()
            }),
        }
    }

    fn load_rows(&self, txn: &heed::RoTxn<'_>, wid: u32) -> Result<Vec<StoredRow>, StoreError> {
        let mut rows = Vec::new();
        for pair in self.row.prefix_iter(txn, &wid.to_be_bytes())? {
            let (_key, value) = pair?;
            rows.push(StoredRow::decode(value)?);
        }
        rows.sort_by(|left, right| left.uid.cmp(&right.uid));
        Ok(rows)
    }

    fn put_snapshot(
        &self,
        wtxn: &mut heed::RwTxn<'_>,
        wid: u32,
        reason: SnapshotReason,
        rows: Vec<StoredRow>,
    ) -> Result<(), StoreError> {
        self.prune_snapshots(wtxn, wid)?;
        let snapshot_id = self.next_snapshot_id(wtxn, wid)?;
        let record = SnapshotRecord {
            snapshot_id,
            reason: reason as i32,
            created_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            rows,
        };
        self.snap.put(
            wtxn,
            &snapshot_key(wid, snapshot_id),
            &record.encode_to_vec(),
        )?;
        Ok(())
    }

    /// Leave room for the snapshot about to be written. Oldest by time, then id, go first.
    fn prune_snapshots(&self, wtxn: &mut heed::RwTxn<'_>, wid: u32) -> Result<(), StoreError> {
        let mut found = Vec::new();
        for pair in self.snap.prefix_iter(wtxn, &wid.to_be_bytes())? {
            let (key, value) = pair?;
            if key.len() != 12 {
                continue;
            }
            let record = SnapshotRecord::decode(value)?;
            let mut owned = [0u8; 12];
            owned.copy_from_slice(key);
            found.push((record.created_at_unix_ms, record.snapshot_id, owned));
        }
        if found.len() < RETAIN_SNAPSHOTS {
            return Ok(());
        }
        found.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        let extra = found.len() + 1 - RETAIN_SNAPSHOTS;
        for (_, _, key) in found.into_iter().take(extra) {
            self.snap.delete(wtxn, &key)?;
        }
        Ok(())
    }

    fn next_snapshot_id(&self, txn: &heed::RoTxn<'_>, wid: u32) -> Result<u64, StoreError> {
        let mut next = 1u64;
        for pair in self.snap.prefix_iter(txn, &wid.to_be_bytes())? {
            let (key, _) = pair?;
            if key.len() != 12 {
                continue;
            }
            let id = u64::from_be_bytes(key[4..12].try_into().unwrap());
            next = next.max(id.saturating_add(1));
        }
        Ok(next)
    }
}

struct MergePlan<'a> {
    deletes: Vec<&'a IndexEntry>,
    upserts: Vec<Upsert<'a>>,
}

struct Upsert<'a> {
    kind: UpsertKind,
    row: &'a ScannedRow,
    previous_hash: [u8; 16],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpsertKind {
    Added,
    Updated,
    Moved,
}

fn plan_merge<'a>(scanned: &'a [ScannedRow], stored: &'a [IndexEntry]) -> MergePlan<'a> {
    let mut deletes = Vec::new();
    let mut upserts = Vec::new();
    let mut left = 0;
    let mut right = 0;
    while left < scanned.len() || right < stored.len() {
        if right == stored.len() || (left < scanned.len() && scanned[left].uid < stored[right].uid)
        {
            upserts.push(Upsert {
                kind: UpsertKind::Added,
                row: &scanned[left],
                previous_hash: [0; 16],
            });
            left += 1;
            continue;
        }
        if left == scanned.len() || stored[right].uid < scanned[left].uid {
            deletes.push(&stored[right]);
            right += 1;
            continue;
        }
        let row = &scanned[left];
        let previous = &stored[right];
        if !row.cells_known {
            if row.fingerprint != previous.fingerprint || row.sheet_row != previous.sheet_row {
                upserts.push(Upsert {
                    kind: UpsertKind::Moved,
                    row,
                    previous_hash: previous.content_hash,
                });
            }
            left += 1;
            right += 1;
            continue;
        }
        let hash = content_hash(&hash_cells(&row.cells));
        if hash != previous.content_hash {
            upserts.push(Upsert {
                kind: UpsertKind::Updated,
                row,
                previous_hash: previous.content_hash,
            });
        } else if row.fingerprint != previous.fingerprint || row.sheet_row != previous.sheet_row {
            upserts.push(Upsert {
                kind: UpsertKind::Moved,
                row,
                previous_hash: previous.content_hash,
            });
        }
        left += 1;
        right += 1;
    }
    MergePlan { deletes, upserts }
}

fn hash_cells(cells: &[ScannedCell]) -> Vec<HashCell<'_>> {
    cells
        .iter()
        .map(|cell| HashCell {
            name: &cell.name,
            value: &cell.value,
            exclude_from_hash: cell.exclude_from_hash,
        })
        .collect()
}

fn changed_names(old: &[NamedCell], new: &[ScannedCell], exclude: &[String]) -> Vec<String> {
    let included = |name: &str| !exclude.iter().any(|item| item == name);
    let mut names = Vec::new();
    for cell in new
        .iter()
        .filter(|cell| included(&cell.name) && !cell.exclude_from_hash)
    {
        let stored = old.iter().find(|candidate| candidate.name == cell.name);
        let same = stored
            .and_then(|candidate| candidate.value.as_ref())
            .is_some_and(|value| value == &proto_value(&cell.value));
        if !same {
            names.push(cell.name.clone());
        }
    }
    for cell in old.iter().filter(|cell| included(&cell.name)) {
        let present = new.iter().any(|candidate| candidate.name == cell.name);
        if !present {
            names.push(cell.name.clone());
        }
    }
    names.sort();
    names.dedup();
    names
}

fn stored_from_scan(row: &ScannedRow) -> StoredRow {
    let mut cells: Vec<NamedCell> = row
        .cells
        .iter()
        .map(|cell| NamedCell {
            name: cell.name.clone(),
            value: Some(proto_value(&cell.value)),
        })
        .collect();
    cells.sort_by(|left, right| left.name.cmp(&right.name));
    StoredRow {
        uid: row.uid.clone(),
        sheet_row: row.sheet_row,
        cells,
    }
}

fn row_from_scan(row: &ScannedRow) -> Row {
    Row {
        excel_row: row.sheet_row,
        cells: row
            .cells
            .iter()
            .map(|cell| Cell {
                excel_id: cell.excel_id.clone(),
                value: Some(proto_value(&cell.value)),
            })
            .collect(),
    }
}

fn row_from_stored(row: &StoredRow) -> Row {
    Row {
        excel_row: row.sheet_row,
        cells: row
            .cells
            .iter()
            .map(|cell| Cell {
                excel_id: String::new(),
                value: cell.value.clone(),
            })
            .collect(),
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

fn watch_key(wid: u32) -> [u8; 4] {
    wid.to_be_bytes()
}

fn entry_key(wid: u32, uid: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(4 + uid.len());
    key.extend_from_slice(&wid.to_be_bytes());
    key.extend_from_slice(uid.as_bytes());
    key
}

fn event_key(wid: u32, seq: u64) -> [u8; 12] {
    snapshot_key(wid, seq)
}

fn event_seq(key: &[u8]) -> Result<u64, StoreError> {
    if key.len() != 12 {
        return Err(StoreError::BadRecord("event key"));
    }
    let mut seq = [0u8; 8];
    seq.copy_from_slice(&key[4..12]);
    Ok(u64::from_be_bytes(seq))
}

fn snapshot_key(wid: u32, snapshot_id: u64) -> [u8; 12] {
    let mut key = [0u8; 12];
    key[..4].copy_from_slice(&wid.to_be_bytes());
    key[4..].copy_from_slice(&snapshot_id.to_be_bytes());
    key
}

fn uid_of(wid: u32, key: &[u8]) -> Result<String, StoreError> {
    let prefix = wid.to_be_bytes();
    if key.len() < prefix.len() || key[..prefix.len()] != prefix {
        return Err(StoreError::BadRecord("index key"));
    }
    let uid =
        std::str::from_utf8(&key[prefix.len()..]).map_err(|_| StoreError::BadRecord("uid"))?;
    Ok(uid.to_string())
}

fn encode_index(fingerprint: &str, hash: &[u8; 16], sheet_row: u32) -> Vec<u8> {
    let raw = fingerprint.as_bytes();
    let mut out = Vec::with_capacity(4 + raw.len() + 16 + 4);
    out.extend_from_slice(&(raw.len() as u32).to_be_bytes());
    out.extend_from_slice(raw);
    out.extend_from_slice(hash);
    out.extend_from_slice(&sheet_row.to_be_bytes());
    out
}

fn decode_index(bytes: &[u8]) -> Result<(String, [u8; 16], u32), StoreError> {
    if bytes.len() < 4 {
        return Err(StoreError::BadRecord("index value"));
    }
    let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
    let rest = &bytes[4..];
    if rest.len() < length + 16 + 4 {
        return Err(StoreError::BadRecord("index value"));
    }
    let fingerprint = std::str::from_utf8(&rest[..length])
        .map_err(|_| StoreError::BadRecord("fingerprint"))?
        .to_string();
    let mut hash = [0u8; 16];
    hash.copy_from_slice(&rest[length..length + 16]);
    let sheet_row = u32::from_be_bytes(rest[length + 16..length + 20].try_into().unwrap());
    Ok((fingerprint, hash, sheet_row))
}
