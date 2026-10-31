//! One watch cycle: version, two columns, UID decisions, then the rows that changed.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::columns::excel_id;
use crate::google::{
    value_runs, BookCell, ColRange, GoogleError, GoogleSheets, KEY_COLUMN, KEY_EXCLUDE,
    KEY_FINGERPRINT, KEY_UID,
};
use crate::irbis::table::v1::Column;
use crate::model::CellValue;
use crate::store::{CommitOptions, IndexEntry, ScannedCell, ScannedRow, Store};
use crate::uid::{
    content_hash, draw_uid, fingerprint_formula, formula_stamp, is_uid, FormulaColumn, HashCell,
};

type CycleSleep = Pin<Box<dyn Future<Output = ()> + Send>>;

/// One `batchGet` or `batchUpdate` holds at most this many ranges.
const RANGE_CHUNK: usize = 100;

macro_rules! sheets_retry {
    ($failures:expr, $sleep:expr, $jitter:expr, $call:expr) => {{
        loop {
            match $call.await {
                Err($crate::google::GoogleError::RateLimited) => {
                    let delay = $crate::google::backoff_delay(*$failures, $jitter());
                    $sleep(delay).await;
                    *$failures = $failures.saturating_add(1);
                }
                Ok(value) => {
                    *$failures = 0;
                    break value;
                }
                Err(err) => return Err(err),
            }
        }
    }};
}

pub struct MappedColumn {
    pub name: String,
    pub exclude_from_hash: bool,
}

pub struct CycleWatch {
    pub wid: u32,
    pub spreadsheet_id: String,
    pub sheet_id: i32,
    pub header_row: u32,
    pub columns: Vec<MappedColumn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleStop {
    Committed,
    Wrote,
    Dropped,
    MissingColumn(String),
}

struct Located {
    name: String,
    index: u32,
    exclude_from_hash: bool,
}

struct SheetRow {
    sheet_row: u32,
    uid: String,
    fingerprint: String,
}

pub async fn run_cycle(
    store: &Store,
    client: &mut impl GoogleSheets,
    watch: &CycleWatch,
    mut sleep: impl FnMut(Duration) -> CycleSleep,
    mut jitter: impl FnMut() -> Duration,
) -> Result<CycleStop, GoogleError> {
    let mut failures = 0u32;
    let version = sheets_retry!(
        &mut failures,
        &mut sleep,
        &mut jitter,
        client.version(&watch.spreadsheet_id)
    );
    let meta = sheets_retry!(
        &mut failures,
        &mut sleep,
        &mut jitter,
        client.metadata(&watch.spreadsheet_id, watch.sheet_id)
    );
    let mut located = Vec::new();
    for column in &watch.columns {
        let Some(found) = meta
            .iter()
            .find(|item| item.key == KEY_COLUMN && item.value == column.name)
        else {
            let _ = store.mark_column_missing(watch.wid, &version);
            return Ok(CycleStop::MissingColumn(column.name.clone()));
        };
        let excluded = column.exclude_from_hash
            || meta.iter().any(|item| {
                item.key == KEY_EXCLUDE && item.column == found.column && item.value == "1"
            });
        located.push(Located {
            name: column.name.clone(),
            index: found.column,
            exclude_from_hash: excluded,
        });
    }
    let created_uid = sheets_retry!(
        &mut failures,
        &mut sleep,
        &mut jitter,
        client.ensure_service_column(&watch.spreadsheet_id, watch.sheet_id, KEY_UID, true)
    );
    let created_fp = sheets_retry!(
        &mut failures,
        &mut sleep,
        &mut jitter,
        client.ensure_service_column(
            &watch.spreadsheet_id,
            watch.sheet_id,
            KEY_FINGERPRINT,
            false
        )
    );
    if created_uid || created_fp {
        if created_fp {
            let fresh = sheets_retry!(
                &mut failures,
                &mut sleep,
                &mut jitter,
                client.metadata(&watch.spreadsheet_id, watch.sheet_id)
            );
            let fp_col = fresh
                .iter()
                .find(|item| item.key == KEY_FINGERPRINT)
                .map(|item| item.column)
                .ok_or_else(|| GoogleError::Message("fingerprint column".into()))?;
            let expected = formula_for(&located, watch.header_row + 1);
            sheets_retry!(
                &mut failures,
                &mut sleep,
                &mut jitter,
                client.write_formula(&watch.spreadsheet_id, watch.sheet_id, 1, fp_col, &expected)
            );
            let stamp = formula_stamp(&expected);
            sheets_retry!(
                &mut failures,
                &mut sleep,
                &mut jitter,
                client.set_meta_value(
                    &watch.spreadsheet_id,
                    watch.sheet_id,
                    KEY_FINGERPRINT,
                    &stamp,
                )
            );
        }
        return Ok(CycleStop::Wrote);
    }
    let Some(uid_col) = meta
        .iter()
        .find(|item| item.key == KEY_UID)
        .map(|item| item.column)
    else {
        return Ok(CycleStop::Dropped);
    };
    let Some(fp_col) = meta
        .iter()
        .find(|item| item.key == KEY_FINGERPRINT)
        .map(|item| item.column)
    else {
        return Ok(CycleStop::Dropped);
    };

    let start = watch.header_row + 1;
    let columns = sheets_retry!(
        &mut failures,
        &mut sleep,
        &mut jitter,
        client.batch_get(
            &watch.spreadsheet_id,
            watch.sheet_id,
            &[
                ColRange {
                    start_row: start,
                    end_row: None,
                    start_col: uid_col,
                    end_col: uid_col,
                },
                ColRange {
                    start_row: 1,
                    end_row: None,
                    start_col: fp_col,
                    end_col: fp_col,
                },
            ],
        )
    );
    let after = sheets_retry!(
        &mut failures,
        &mut sleep,
        &mut jitter,
        client.version(&watch.spreadsheet_id)
    );
    if after != version {
        return Ok(CycleStop::Dropped);
    }

    let expected = formula_for(&located, start);
    let stamp = formula_stamp(&expected);
    let recorded = meta
        .iter()
        .find(|item| item.key == KEY_FINGERPRINT)
        .map(|item| item.value.as_str())
        .unwrap_or("");
    let filled = columns
        .get(&(1, fp_col))
        .and_then(|cell| cell.formula.as_ref())
        .is_some_and(|text| !text.is_empty());
    let formula_ok = recorded == stamp && filled;
    let mut rows = read_service_rows(start, uid_col, fp_col, &columns);
    let stored = store
        .index(watch.wid)
        .map_err(|err| GoogleError::Message(err.to_string()))?;
    let by_uid = index_by_uid(&stored);
    let uid_writes = decide_uids(
        client,
        watch,
        &stored,
        &by_uid,
        &located,
        uid_col,
        &mut rows,
        &mut failures,
        &mut sleep,
        &mut jitter,
    )
    .await?;
    if !formula_ok {
        sheets_retry!(
            &mut failures,
            &mut sleep,
            &mut jitter,
            client.write_formula(&watch.spreadsheet_id, watch.sheet_id, 1, fp_col, &expected)
        );
        sheets_retry!(
            &mut failures,
            &mut sleep,
            &mut jitter,
            client.set_meta_value(
                &watch.spreadsheet_id,
                watch.sheet_id,
                KEY_FINGERPRINT,
                &stamp,
            )
        );
    }
    if !formula_ok || !uid_writes.is_empty() {
        if !uid_writes.is_empty() {
            let runs = value_runs(&uid_writes);
            for chunk in runs.chunks(RANGE_CHUNK) {
                sheets_retry!(
                    &mut failures,
                    &mut sleep,
                    &mut jitter,
                    client.write_raw(&watch.spreadsheet_id, watch.sheet_id, chunk)
                );
            }
        }
        return Ok(CycleStop::Wrote);
    }

    let changed = rows_to_fetch(&rows, &by_uid);
    let mut fetched = BTreeMap::new();
    if !changed.is_empty() {
        let ranges = sheet_ranges(&changed, rows.len(), &located);
        fetched = read_ranges(
            client,
            watch,
            &ranges,
            &mut failures,
            &mut sleep,
            &mut jitter,
        )
        .await?;
        let after_fetch = sheets_retry!(
            &mut failures,
            &mut sleep,
            &mut jitter,
            client.version(&watch.spreadsheet_id)
        );
        if after_fetch != version {
            return Ok(CycleStop::Dropped);
        }
    }

    let changed_rows: HashSet<u32> = changed.iter().copied().collect();
    let scan = build_scan(&rows, &changed_rows, &located, &fetched);
    let last_seq = store
        .watch(watch.wid)
        .map_err(|err| GoogleError::Message(err.to_string()))?
        .last_seq;
    let snapshot = last_seq == 0 && stored.is_empty();
    store
        .commit(
            watch.wid,
            &scan,
            CommitOptions {
                version: Some(version),
                snapshot,
                columns: proto_columns(&located),
                exclude: located
                    .iter()
                    .filter(|column| column.exclude_from_hash)
                    .map(|column| column.name.clone())
                    .collect(),
                ..CommitOptions::default()
            },
        )
        .map_err(|err| GoogleError::Message(err.to_string()))?;
    Ok(CycleStop::Committed)
}

fn formula_for(located: &[Located], start: u32) -> String {
    let letters: Vec<String> = located
        .iter()
        .map(|column| excel_id(column.index))
        .collect();
    let formula_inputs: Vec<FormulaColumn<'_>> = located
        .iter()
        .zip(letters.iter())
        .map(|(column, letter)| FormulaColumn {
            excel_id: letter,
            name: &column.name,
            exclude_from_hash: column.exclude_from_hash,
        })
        .collect();
    fingerprint_formula(&formula_inputs, start)
}

fn read_service_rows(
    start: u32,
    uid_col: u32,
    fp_col: u32,
    cells: &BTreeMap<(u32, u32), BookCell>,
) -> Vec<SheetRow> {
    let end = cells
        .keys()
        .map(|(row, _)| *row)
        .filter(|row| *row >= start)
        .max()
        .unwrap_or(start.saturating_sub(1));
    if end < start {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for sheet_row in start..=end {
        let uid = text_at(cells, sheet_row, uid_col);
        let fingerprint = text_at(cells, sheet_row, fp_col);
        if uid.is_empty() && fingerprint.is_empty() {
            continue;
        }
        rows.push(SheetRow {
            sheet_row,
            uid,
            fingerprint,
        });
    }
    rows
}

fn text_at(cells: &BTreeMap<(u32, u32), BookCell>, row: u32, col: u32) -> String {
    match cells.get(&(row, col)).and_then(|cell| cell.value.as_ref()) {
        Some(CellValue::Text(text)) => text.clone(),
        Some(CellValue::Int(number)) => number.to_string(),
        Some(CellValue::Float(number)) => number.to_string(),
        Some(CellValue::Bool(flag)) => flag.to_string(),
        Some(CellValue::Date(_)) => String::new(),
        None => String::new(),
    }
}

async fn decide_uids(
    client: &mut impl GoogleSheets,
    watch: &CycleWatch,
    stored: &[IndexEntry],
    by_uid: &HashMap<&str, &IndexEntry>,
    located: &[Located],
    uid_col: u32,
    rows: &mut [SheetRow],
    failures: &mut u32,
    sleep: &mut impl FnMut(Duration) -> CycleSleep,
    jitter: &mut impl FnMut() -> Duration,
) -> Result<Vec<(u32, u32, String)>, GoogleError> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows.iter().filter(|row| is_uid(&row.uid)) {
        *counts.entry(row.uid.clone()).or_default() += 1;
    }
    let mut taken: HashSet<String> = counts.keys().cloned().collect();
    for entry in stored {
        taken.insert(entry.uid.clone());
    }
    let present = counts.keys().cloned().collect::<HashSet<_>>();
    let missing: Vec<&IndexEntry> = stored
        .iter()
        .filter(|entry| !present.contains(&entry.uid))
        .collect();

    let mut need_cells: Vec<u32> = rows
        .iter()
        .filter(|row| counts.get(&row.uid).copied().unwrap_or(0) > 1)
        .map(|row| row.sheet_row)
        .collect();
    if !missing.is_empty() {
        need_cells.extend(
            rows.iter()
                .filter(|row| row_for_restore(row, &counts, by_uid))
                .map(|row| row.sheet_row),
        );
    }
    need_cells.sort_unstable();
    need_cells.dedup();
    let fetched = if need_cells.is_empty() {
        BTreeMap::new()
    } else {
        let ranges = sheet_ranges(&need_cells, rows.len(), located);
        read_ranges(client, watch, &ranges, failures, sleep, jitter).await?
    };

    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if counts.get(&row.uid).copied().unwrap_or(0) > 1 {
            groups.entry(row.uid.clone()).or_default().push(index);
        }
    }
    let mut writes = Vec::new();
    for (uid, indexes) in groups {
        let stored_hash = by_uid.get(uid.as_str()).map(|entry| entry.content_hash);
        let mut matched: Vec<usize> = indexes
            .iter()
            .copied()
            .filter(|index| {
                let hash = hash_row(rows[*index].sheet_row, located, &fetched);
                stored_hash.is_some_and(|expected| hash == expected)
            })
            .collect();
        matched.sort_by_key(|index| rows[*index].sheet_row);
        let keeper = matched.first().copied();
        let replace: Vec<usize> = if matched.is_empty() {
            indexes
        } else {
            indexes
                .into_iter()
                .filter(|index| Some(*index) != keeper)
                .collect()
        };
        for index in replace {
            let fresh = draw_uid(&taken).map_err(|err| GoogleError::Message(err.to_string()))?;
            taken.insert(fresh.clone());
            rows[index].uid = fresh.clone();
            writes.push((rows[index].sheet_row, uid_col, fresh));
        }
    }

    let mut missing_by_hash: HashMap<[u8; 16], VecDeque<&IndexEntry>> = HashMap::new();
    for entry in missing {
        missing_by_hash
            .entry(entry.content_hash)
            .or_default()
            .push_back(entry);
    }
    let replaced: HashSet<u32> = writes.iter().map(|(sheet_row, _, _)| *sheet_row).collect();
    for row in rows.iter_mut() {
        if replaced.contains(&row.sheet_row) || (row.uid.is_empty() && row.fingerprint.is_empty()) {
            continue;
        }
        let blank = row.uid.is_empty();
        let invalid = !blank && !is_uid(&row.uid);
        let foreign = is_uid(&row.uid)
            && counts.get(&row.uid).copied().unwrap_or(0) == 1
            && !by_uid.contains_key(row.uid.as_str());
        if !(blank || invalid || foreign) {
            continue;
        }
        let hash = hash_row(row.sheet_row, located, &fetched);
        let restored = missing_by_hash
            .get_mut(&hash)
            .and_then(VecDeque::pop_front)
            .map(|entry| entry.uid.clone());
        if let Some(uid) = restored {
            taken.insert(uid.clone());
            row.uid = uid.clone();
            writes.push((row.sheet_row, uid_col, uid));
        } else if blank || invalid {
            let fresh = draw_uid(&taken).map_err(|err| GoogleError::Message(err.to_string()))?;
            taken.insert(fresh.clone());
            row.uid = fresh.clone();
            writes.push((row.sheet_row, uid_col, fresh));
        }
    }
    Ok(writes)
}

/// An empty, invalid, or unique unknown UID can take a missing stored id when the cells match.
fn row_for_restore(
    row: &SheetRow,
    counts: &HashMap<String, usize>,
    by_uid: &HashMap<&str, &IndexEntry>,
) -> bool {
    if row.uid.is_empty() {
        return !row.fingerprint.is_empty();
    }
    if !is_uid(&row.uid) {
        return true;
    }
    counts.get(&row.uid).copied().unwrap_or(0) == 1 && !by_uid.contains_key(row.uid.as_str())
}

fn hash_row(
    sheet_row: u32,
    located: &[Located],
    cells: &BTreeMap<(u32, u32), BookCell>,
) -> [u8; 16] {
    let scanned: Vec<HashCell<'_>> = located
        .iter()
        .filter_map(|column| {
            let value = cells.get(&(sheet_row, column.index))?.value.as_ref()?;
            Some(HashCell {
                name: &column.name,
                value,
                exclude_from_hash: column.exclude_from_hash,
            })
        })
        .collect();
    content_hash(&scanned)
}

fn index_by_uid(stored: &[IndexEntry]) -> HashMap<&str, &IndexEntry> {
    stored
        .iter()
        .map(|entry| (entry.uid.as_str(), entry))
        .collect()
}

fn rows_to_fetch(rows: &[SheetRow], by_uid: &HashMap<&str, &IndexEntry>) -> Vec<u32> {
    rows.iter()
        .filter(|row| {
            by_uid
                .get(row.uid.as_str())
                .map(|entry| entry.fingerprint != row.fingerprint)
                .unwrap_or(true)
        })
        .map(|row| row.sheet_row)
        .collect()
}

/// Consecutive rows share one range. More than half of the service rows become one block
/// from the first wanted row through the last.
fn sheet_ranges(wanted: &[u32], population: usize, located: &[Located]) -> Vec<ColRange> {
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut rows = wanted.to_vec();
    rows.sort_unstable();
    rows.dedup();
    if rows.len() * 2 > population {
        return vec![block_range(rows[0], *rows.last().unwrap(), located)];
    }
    let mut ranges = Vec::new();
    let mut start = rows[0];
    let mut end = rows[0];
    for row in rows.into_iter().skip(1) {
        if row == end + 1 {
            end = row;
        } else {
            ranges.push(block_range(start, end, located));
            start = row;
            end = row;
        }
    }
    ranges.push(block_range(start, end, located));
    ranges
}

fn block_range(start_row: u32, end_row: u32, located: &[Located]) -> ColRange {
    let start_col = located.iter().map(|column| column.index).min().unwrap_or(0);
    let end_col = located.iter().map(|column| column.index).max().unwrap_or(0);
    ColRange {
        start_row,
        end_row: Some(end_row),
        start_col,
        end_col,
    }
}

async fn read_ranges(
    client: &mut impl GoogleSheets,
    watch: &CycleWatch,
    ranges: &[ColRange],
    failures: &mut u32,
    sleep: &mut impl FnMut(Duration) -> CycleSleep,
    jitter: &mut impl FnMut() -> Duration,
) -> Result<BTreeMap<(u32, u32), BookCell>, GoogleError> {
    let mut fetched = BTreeMap::new();
    for chunk in ranges.chunks(RANGE_CHUNK) {
        let part = sheets_retry!(
            failures,
            sleep,
            jitter,
            client.batch_get(&watch.spreadsheet_id, watch.sheet_id, chunk)
        );
        fetched.extend(part);
    }
    Ok(fetched)
}

fn build_scan(
    rows: &[SheetRow],
    changed: &HashSet<u32>,
    located: &[Located],
    fetched: &BTreeMap<(u32, u32), BookCell>,
) -> Vec<ScannedRow> {
    rows.iter()
        .filter(|row| is_uid(&row.uid))
        .map(|row| {
            let known = changed.contains(&row.sheet_row);
            let cells = if known {
                located
                    .iter()
                    .filter_map(|column| {
                        let value = fetched.get(&(row.sheet_row, column.index))?.value.clone()?;
                        Some(ScannedCell {
                            name: column.name.clone(),
                            excel_id: excel_id(column.index),
                            value,
                            exclude_from_hash: column.exclude_from_hash,
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            ScannedRow {
                uid: row.uid.clone(),
                sheet_row: row.sheet_row,
                fingerprint: row.fingerprint.clone(),
                cells,
                cells_known: known,
            }
        })
        .collect()
}

fn proto_columns(located: &[Located]) -> Vec<Column> {
    located
        .iter()
        .map(|column| Column {
            excel_id: excel_id(column.index),
            name: column.name.clone(),
            subcolumns: Vec::new(),
        })
        .collect()
}
