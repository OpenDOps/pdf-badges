use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use google_tables::cycle::{run_cycle, CycleStop, CycleWatch, MappedColumn};
use google_tables::google::{
    FakeBook, Meta, SheetTab, KEY_COLUMN, KEY_EXCLUDE, KEY_FINGERPRINT, KEY_UID,
};
use google_tables::irbis::google_tables::v1::{ChangeKind, SnapshotReason, StoredRow};
use google_tables::irbis::table::v1::value;
use google_tables::model::{CellValue, Grid};
use google_tables::store::Store;
use google_tables::uid::{
    content_hash, fingerprint_formula, formula_stamp, is_uid, FormulaColumn, HashCell,
};

const ANN: &str = "0123456789ABCDEF";
const BO: &str = "0123456789ABCDEG";
const CARL: &str = "0123456789ABCDEH";

fn scratch() -> Store {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = root.join(format!("google-tables-cycle-{nanos}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    Store::open(&dir.join("google-tables.lmdb")).unwrap()
}

struct Seed<'a> {
    sheet_row: u32,
    name: &'a str,
    city: &'a str,
    uid: &'a str,
    fingerprint: &'a str,
}

fn prepared(seeds: &[Seed<'_>]) -> (FakeBook, CycleWatch) {
    let mut tab = SheetTab::from_grid("Visitors", 0, &Grid::new(1, 0, BTreeMap::new()));
    tab.put_text(1, 0, "Name");
    tab.put_text(1, 1, "City");
    for seed in seeds {
        if !seed.name.is_empty() {
            tab.put_text(seed.sheet_row, 0, seed.name);
        }
        if !seed.city.is_empty() {
            tab.put_text(seed.sheet_row, 1, seed.city);
        }
        if !seed.uid.is_empty() {
            tab.put_text(seed.sheet_row, 2, seed.uid);
        }
        if !seed.fingerprint.is_empty() {
            tab.put_text(seed.sheet_row, 3, seed.fingerprint);
        }
    }
    tab.metadata = vec![
        Meta {
            key: KEY_COLUMN.into(),
            column: 0,
            value: "Name".into(),
        },
        Meta {
            key: KEY_COLUMN.into(),
            column: 1,
            value: "City".into(),
        },
        Meta {
            key: KEY_UID.into(),
            column: 2,
            value: String::new(),
        },
        Meta {
            key: KEY_FINGERPRINT.into(),
            column: 3,
            value: String::new(),
        },
    ];
    let formula = fingerprint_formula(
        &[
            FormulaColumn {
                excel_id: "A",
                name: "Name",
                exclude_from_hash: false,
            },
            FormulaColumn {
                excel_id: "B",
                name: "City",
                exclude_from_hash: false,
            },
        ],
        2,
    );
    tab.put_formula(1, 3, &formula);
    stamp_formula(&mut tab, &formula);
    let mut book = FakeBook::new("sheet-1");
    book.tabs.push(tab);
    let watch = CycleWatch {
        wid: 1,
        spreadsheet_id: "sheet-1".into(),
        sheet_id: 0,
        header_row: 1,
        columns: vec![
            MappedColumn {
                name: "Name".into(),
                exclude_from_hash: false,
            },
            MappedColumn {
                name: "City".into(),
                exclude_from_hash: false,
            },
        ],
    };
    (book, watch)
}

fn run(store: &Store, book: &mut FakeBook, watch: &CycleWatch) -> CycleStop {
    drive(run_cycle(
        store,
        book,
        watch,
        |_| Box::pin(std::future::ready(())),
        || Duration::ZERO,
    ))
    .unwrap()
}

fn drive<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn cell_text(row: &StoredRow, name: &str) -> String {
    row.cells
        .iter()
        .find(|cell| cell.name == name)
        .and_then(|cell| match cell.value.as_ref()?.kind.as_ref()? {
            value::Kind::Text(text) => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn cycle_narrow_fetch() {
    let store = scratch();
    let (mut book, watch) = prepared(&[
        Seed {
            sheet_row: 2,
            name: "Ann",
            city: "Oslo",
            uid: ANN,
            fingerprint: "fp-ann",
        },
        Seed {
            sheet_row: 3,
            name: "Bo",
            city: "Bergen",
            uid: BO,
            fingerprint: "fp-bo",
        },
    ]);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    book.gets.clear();
    book.tab_mut(0).unwrap().put_text(3, 3, "fp-bo-2");
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let mapped: Vec<u32> = book
        .gets
        .iter()
        .flatten()
        .filter(|range| range.start_col == 0)
        .map(|range| range.start_row)
        .collect();
    assert_eq!(mapped, vec![3]);
}

#[test]
fn cycle_write_does_not_commit() {
    let store = scratch();
    let (mut book, watch) = prepared(&[
        Seed {
            sheet_row: 2,
            name: "Ann",
            city: "Oslo",
            uid: "",
            fingerprint: "fp-ann",
        },
        Seed {
            sheet_row: 3,
            name: "Bo",
            city: "Bergen",
            uid: "",
            fingerprint: "fp-bo",
        },
    ]);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    assert_eq!(book.raw_writes.len(), 1);
    assert_eq!(book.raw_writes[0].runs.len(), 1);
    assert_eq!(book.raw_writes[0].runs[0].values.len(), 2);
    assert_eq!(book.raw_writes[0].cells.len(), 2);
    assert!(book.raw_writes[0]
        .cells
        .iter()
        .all(|(_, col, text)| *col == 2 && is_uid(text)));
    assert!(store.read_stored(watch.wid).unwrap().rows.is_empty());
    assert!(store.watch(watch.wid).unwrap().drive_version.is_empty());

    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let events = store.changes(watch.wid).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, ChangeKind::Snapshot as i32);
    assert_eq!(store.watch(watch.wid).unwrap().drive_version, "2");
    assert_eq!(store.read_stored(watch.wid).unwrap().rows.len(), 2);
}

#[test]
fn cycle_version_moves() {
    let store = scratch();
    let (mut book, watch) = prepared(&[Seed {
        sheet_row: 2,
        name: "Ann",
        city: "Oslo",
        uid: ANN,
        fingerprint: "fp-ann",
    }]);
    book.version_queue.push_back("5".into());
    book.version_queue.push_back("6".into());
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Dropped);
    assert!(store.read_stored(watch.wid).unwrap().rows.is_empty());
    assert!(store.changes(watch.wid).unwrap().is_empty());
    assert!(store.watch(watch.wid).unwrap().drive_version.is_empty());
}

#[test]
fn cycle_gap_keeps_the_tail() {
    let store = scratch();
    let (mut book, watch) = prepared(&[
        Seed {
            sheet_row: 2,
            name: "Ann",
            city: "Oslo",
            uid: ANN,
            fingerprint: "fp-ann",
        },
        Seed {
            sheet_row: 53,
            name: "Tail",
            city: "Oslo",
            uid: BO,
            fingerprint: "fp-tail",
        },
    ]);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let rows = store.read_stored(watch.wid).unwrap().rows;
    let rows_on_sheet: Vec<u32> = rows.iter().map(|row| row.sheet_row).collect();
    assert_eq!(rows_on_sheet, vec![2, 53]);
    assert!(rows.iter().any(|row| cell_text(row, "Name") == "Tail"));
}

#[test]
fn cycle_backoff() {
    let store = scratch();
    let (mut book, watch) = prepared(&[Seed {
        sheet_row: 2,
        name: "Ann",
        city: "Oslo",
        uid: ANN,
        fingerprint: "fp-ann",
    }]);
    book.rate_limit = [true, true, false, true].into_iter().collect();
    let waits = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&waits);
    let stop = drive(run_cycle(
        &store,
        &mut book,
        &watch,
        move |delay| {
            let recorded = Arc::clone(&recorded);
            Box::pin(async move {
                recorded.lock().unwrap().push(delay);
            })
        },
        || Duration::ZERO,
    ))
    .unwrap();
    assert_eq!(stop, CycleStop::Committed);
    let waits = waits.lock().unwrap().clone();
    assert_eq!(
        waits,
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(1),
        ]
    );
}

fn committed() -> (Store, FakeBook, CycleWatch) {
    let store = scratch();
    let (mut book, watch) = prepared(&[
        Seed {
            sheet_row: 2,
            name: "Ann",
            city: "Oslo",
            uid: ANN,
            fingerprint: "fp-ann",
        },
        Seed {
            sheet_row: 3,
            name: "Bo",
            city: "Bergen",
            uid: BO,
            fingerprint: "fp-bo",
        },
    ]);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    book.gets.clear();
    book.raw_writes.clear();
    book.formulas.clear();
    (store, book, watch)
}

fn mapped_rows(book: &FakeBook) -> Vec<u32> {
    let mut rows = Vec::new();
    for range in book
        .gets
        .iter()
        .flatten()
        .filter(|range| range.start_col == 0)
    {
        let end = range.end_row.unwrap_or(range.start_row);
        rows.extend(range.start_row..=end);
    }
    rows
}

fn uid_at(book: &FakeBook, row: u32) -> String {
    let col = book
        .tab(0)
        .unwrap()
        .metadata
        .iter()
        .find(|item| item.key == KEY_UID)
        .unwrap()
        .column;
    match book
        .tab(0)
        .unwrap()
        .cells
        .get(&(row, col))
        .and_then(|cell| cell.value.as_ref())
    {
        Some(CellValue::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

#[test]
fn cycle_refill_emits_added() {
    let (store, mut book, watch) = committed();
    {
        let tab = book.tab_mut(0).unwrap();
        for row in 2..=3 {
            for col in 0..=3 {
                tab.cells.remove(&(row, col));
            }
        }
    }
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let tab = book.tab_mut(0).unwrap();
    tab.put_text(2, 0, "Ida");
    tab.put_text(2, 1, "Oslo");
    tab.put_text(2, 2, CARL);
    tab.put_text(2, 3, "fp-ida");
    tab.put_text(3, 0, "Jo");
    tab.put_text(3, 1, "Bergen");
    tab.put_text(3, 2, "0123456789ABCDEI");
    tab.put_text(3, 3, "fp-jo");
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert_eq!(
        kinds(&store, watch.wid),
        vec![
            ChangeKind::Snapshot as i32,
            ChangeKind::Deleted as i32,
            ChangeKind::Deleted as i32,
            ChangeKind::Added as i32,
            ChangeKind::Added as i32,
        ]
    );
}

fn kinds(store: &Store, wid: u32) -> Vec<i32> {
    store
        .changes(wid)
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

#[test]
fn cycle_restore_uid_by_hash() {
    let (store, mut book, watch) = committed();
    book.tab_mut(0).unwrap().cells.remove(&(2, 2));
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    assert_eq!(mapped_rows(&book), vec![2]);
    assert_eq!(book.raw_writes.len(), 1);
    assert_eq!(book.raw_writes[0].cells, vec![(2, 2, ANN.to_string())]);
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
    assert_eq!(uid_at(&book, 2), ANN);
}

#[test]
fn cycle_replace_invalid_uid() {
    let (store, mut book, watch) = committed();
    book.tab_mut(0).unwrap().put_text(2, 2, "JJJJJJJJJJJJJJJJ");
    book.tab_mut(0).unwrap().put_text(1, 2, "JJJJJJJJJJJJJJJJ");
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    assert_eq!(mapped_rows(&book), vec![2]);
    assert_eq!(book.raw_writes.len(), 1);
    assert_eq!(book.raw_writes[0].cells, vec![(2, 2, ANN.to_string())]);
    assert_eq!(
        book.tab(0)
            .unwrap()
            .cells
            .get(&(1, 2))
            .and_then(|cell| cell.value.as_ref()),
        Some(&CellValue::Text("JJJJJJJJJJJJJJJJ".into()))
    );
    assert!(store
        .index(watch.wid)
        .unwrap()
        .iter()
        .any(|entry| entry.uid == ANN));
}

#[test]
fn cycle_edited_uid_restored() {
    let (store, mut book, watch) = committed();
    let tab = book.tab_mut(0).unwrap();
    tab.put_text(2, 2, "not-a-uid");
    tab.put_text(3, 2, CARL);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    let mut written = book.raw_writes[0].cells.clone();
    written.sort();
    assert_eq!(
        written,
        vec![(2, 2, ANN.to_string()), (3, 2, BO.to_string())]
    );
    assert!(kinds(&store, watch.wid)
        .iter()
        .all(|kind| *kind != ChangeKind::Deleted as i32 && *kind != ChangeKind::Added as i32));
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
    assert_eq!(uid_at(&book, 2), ANN);
    assert_eq!(uid_at(&book, 3), BO);
}

#[test]
fn cycle_duplicate_keeps_the_matching_row() {
    let store = scratch();
    let (mut book, watch) = prepared(&[
        Seed {
            sheet_row: 2,
            name: "Ann",
            city: "Oslo",
            uid: ANN,
            fingerprint: "fp-ann",
        },
        Seed {
            sheet_row: 3,
            name: "Bo",
            city: "Bergen",
            uid: BO,
            fingerprint: "fp-bo",
        },
        Seed {
            sheet_row: 4,
            name: "Carl",
            city: "Tromso",
            uid: CARL,
            fingerprint: "fp-carl",
        },
    ]);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    book.gets.clear();
    book.raw_writes.clear();
    let tab = book.tab_mut(0).unwrap();
    tab.put_text(3, 0, "Ann");
    tab.put_text(3, 1, "Oslo");
    tab.put_text(3, 2, ANN);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    assert_eq!(mapped_rows(&book), vec![2, 3]);
    assert_eq!(book.raw_writes.len(), 1);
    let (row, _, text) = &book.raw_writes[0].cells[0];
    assert_eq!(*row, 3);
    assert!(is_uid(text));
    assert_ne!(text, ANN);
    assert_eq!(uid_at(&book, 2), ANN);
}

#[test]
fn cycle_unmatched_copies_are_deleted() {
    let store = scratch();
    let mut tab = SheetTab::from_grid("Visitors", 0, &Grid::new(1, 0, BTreeMap::new()));
    tab.put_text(1, 0, "Name");
    tab.put_text(1, 1, "City");
    let mut old = Vec::new();
    for index in 0..10 {
        let row = 2 + index;
        let uid = format!("{:016X}", index + 1);
        tab.put_text(row, 0, format!("n{index}"));
        tab.put_text(row, 1, format!("c{index}"));
        tab.put_text(row, 2, &uid);
        tab.put_text(row, 3, format!("fp-{index}"));
        old.push(uid);
    }
    tab.metadata = vec![
        Meta {
            key: KEY_COLUMN.into(),
            column: 0,
            value: "Name".into(),
        },
        Meta {
            key: KEY_COLUMN.into(),
            column: 1,
            value: "City".into(),
        },
        Meta {
            key: KEY_UID.into(),
            column: 2,
            value: String::new(),
        },
        Meta {
            key: KEY_FINGERPRINT.into(),
            column: 3,
            value: String::new(),
        },
    ];
    install_formula(
        &mut tab,
        3,
        &[
            FormulaColumn {
                excel_id: "A",
                name: "Name",
                exclude_from_hash: false,
            },
            FormulaColumn {
                excel_id: "B",
                name: "City",
                exclude_from_hash: false,
            },
        ],
    );
    let mut book = FakeBook::new("sheet-1");
    book.tabs.push(tab);
    let watch = CycleWatch {
        wid: 1,
        spreadsheet_id: "sheet-1".into(),
        sheet_id: 0,
        header_row: 1,
        columns: vec![
            MappedColumn {
                name: "Name".into(),
                exclude_from_hash: false,
            },
            MappedColumn {
                name: "City".into(),
                exclude_from_hash: false,
            },
        ],
    };
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    book.raw_writes.clear();
    {
        let tab = book.tab_mut(0).unwrap();
        for index in 0..10 {
            let row = 2 + index;
            tab.put_text(row, 0, format!("x{index}"));
            tab.put_text(row, 2, &old[0]);
        }
    }
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    let written: Vec<String> = book.raw_writes[0]
        .cells
        .iter()
        .map(|(_, _, text)| text.clone())
        .collect();
    assert_eq!(written.len(), 10);
    assert!(written.iter().all(|text| is_uid(text)));
    assert!(written.iter().all(|text| !old.contains(text)));
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert_eq!(
        kinds(&store, watch.wid)
            .into_iter()
            .filter(|kind| *kind == ChangeKind::Deleted as i32)
            .count(),
        10
    );
    assert_eq!(store.watch(watch.wid).unwrap().status, "running");
    assert_eq!(store.watch(watch.wid).unwrap().drive_version, "2");
    let snapshots = store.list_snapshots(watch.wid).unwrap();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].reason, SnapshotReason::Initial as i32);
    assert_eq!(snapshots[1].reason, SnapshotReason::MassiveDelete as i32);
    assert!(snapshots[0].created_at_unix_ms <= snapshots[1].created_at_unix_ms);
    assert_eq!(snapshots[1].rows.len(), 10);
}

#[test]
fn cycle_same_hash_skips_updated() {
    let (store, mut book, watch) = committed();
    book.tab_mut(0).unwrap().put_text(2, 3, "fp-ann-2");
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert_eq!(mapped_rows(&book), vec![2]);
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
    let entry = store
        .index(watch.wid)
        .unwrap()
        .into_iter()
        .find(|entry| entry.uid == ANN)
        .unwrap();
    assert_eq!(entry.fingerprint, "fp-ann-2");
    assert_eq!(entry.sheet_row, 2);
}

#[test]
fn cycle_move_updates_sheet_row() {
    let (store, mut book, watch) = committed();
    let tab = book.tab_mut(0).unwrap();
    for col in 0..=3 {
        let cell = tab.cells.remove(&(2, col)).unwrap();
        tab.cells.insert((5, col), cell);
    }
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert!(mapped_rows(&book).is_empty());
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
    let entry = store
        .index(watch.wid)
        .unwrap()
        .into_iter()
        .find(|entry| entry.uid == ANN)
        .unwrap();
    assert_eq!(entry.sheet_row, 5);
}

#[test]
fn cycle_exclude_hash_skips_the_column() {
    let store = scratch();
    let (mut book, watch) = prepared(&[
        Seed {
            sheet_row: 2,
            name: "Ann",
            city: "Oslo",
            uid: ANN,
            fingerprint: "fp-ann",
        },
        Seed {
            sheet_row: 3,
            name: "Bo",
            city: "Bergen",
            uid: BO,
            fingerprint: "fp-bo",
        },
    ]);
    book.tab_mut(0).unwrap().metadata.push(Meta {
        key: KEY_EXCLUDE.into(),
        column: 1,
        value: "1".into(),
    });
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    let formula = book.formulas.last().unwrap();
    assert!(formula.contains("A2:A"));
    assert!(!formula.contains('B'));
    book.tab_mut(0).unwrap().cells.remove(&(2, 3));
    book.tab_mut(0).unwrap().cells.remove(&(3, 3));
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let ann = CellValue::Text("Ann".into());
    let oslo = CellValue::Text("Oslo".into());
    let hash = content_hash(&[
        HashCell {
            name: "Name",
            value: &ann,
            exclude_from_hash: false,
        },
        HashCell {
            name: "City",
            value: &oslo,
            exclude_from_hash: true,
        },
    ]);
    let stored = store
        .index(watch.wid)
        .unwrap()
        .into_iter()
        .find(|entry| entry.uid == ANN)
        .unwrap();
    assert_eq!(stored.content_hash, hash);
    let saved = store.load_saved(watch.wid).unwrap().unwrap();
    assert!(saved.exclude_from_hash.iter().any(|name| name == "City"));
    book.gets.clear();
    book.tab_mut(0).unwrap().put_text(2, 1, "Bergen");
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert!(mapped_rows(&book).is_empty());
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
    book.tab_mut(0).unwrap().put_text(2, 0, "Ann2");
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let updated = store
        .changes(watch.wid)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == ChangeKind::Updated as i32)
        .unwrap();
    assert_eq!(updated.changed_columns, vec!["Name".to_string()]);
}

#[test]
fn cycle_column_metadata_follows_the_sheet() {
    let (store, mut book, watch) = committed();
    {
        let tab = book.tab_mut(0).unwrap();
        for row in [2u32, 3] {
            let cell = tab.cells.remove(&(row, 1)).unwrap();
            tab.cells.insert((row, 4), cell);
        }
        let city = tab
            .metadata
            .iter_mut()
            .find(|item| item.key == KEY_COLUMN && item.value == "City")
            .unwrap();
        city.column = 4;
    }
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    let formula = book.formulas.last().unwrap();
    assert!(formula.contains("E2:E"));
    assert!(!formula.contains("B2:B"));
    assert!(book
        .tab(0)
        .unwrap()
        .metadata
        .iter()
        .any(|item| { item.key == KEY_COLUMN && item.value == "Name" && item.column == 0 }));
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let saved = store.load_saved(watch.wid).unwrap().unwrap();
    assert_eq!(
        saved
            .columns
            .iter()
            .find(|column| column.name == "Name")
            .unwrap()
            .excel_id,
        "A"
    );
    assert_eq!(
        saved
            .columns
            .iter()
            .find(|column| column.name == "City")
            .unwrap()
            .excel_id,
        "E"
    );
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
}

#[test]
fn cycle_missing_column_pauses() {
    let (store, mut book, watch) = committed();
    book.version = 4;
    book.tab_mut(0)
        .unwrap()
        .metadata
        .retain(|item| !(item.key == KEY_COLUMN && item.value == "City"));
    assert_eq!(
        run(&store, &mut book, &watch),
        CycleStop::MissingColumn("City".into())
    );
    let state = store.watch(watch.wid).unwrap();
    assert_eq!(state.status, "column_missing");
    assert_eq!(state.drive_version, "4");
    let tab = book.tab(0).unwrap();
    assert!(tab.metadata.iter().all(|item| item.value != "City"));
    assert!(tab
        .metadata
        .iter()
        .any(|item| { item.key == KEY_COLUMN && item.value == "Name" && item.column == 0 }));
    assert!(tab.hidden.is_empty());
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
}

#[test]
fn cycle_recreates_uid_column() {
    let (store, mut book, watch) = committed();
    let version = store.watch(watch.wid).unwrap().drive_version;
    book.tab_mut(0)
        .unwrap()
        .metadata
        .retain(|item| item.key != KEY_UID);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    let tab = book.tab(0).unwrap();
    let uid_col = tab
        .metadata
        .iter()
        .find(|item| item.key == KEY_UID)
        .unwrap()
        .column;
    assert_eq!(uid_col, 4);
    assert!(tab.hidden.contains(&uid_col));
    assert!(tab.warning_only.contains(&uid_col));
    assert!(tab
        .metadata
        .iter()
        .any(|item| { item.key == KEY_COLUMN && item.value == "Name" && item.column == 0 }));
    assert!(book.raw_writes.is_empty());
    assert_eq!(store.watch(watch.wid).unwrap().drive_version, version);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    let restored: Vec<String> = book
        .raw_writes
        .last()
        .unwrap()
        .cells
        .iter()
        .map(|(_, _, text)| text.clone())
        .collect();
    assert_eq!(restored, vec![ANN.to_string(), BO.to_string()]);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
}

#[test]
fn cycle_recreates_fingerprint_column() {
    let (store, mut book, watch) = committed();
    book.tab_mut(0)
        .unwrap()
        .metadata
        .retain(|item| item.key != KEY_FINGERPRINT);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    let tab = book.tab(0).unwrap();
    let fp_col = tab
        .metadata
        .iter()
        .find(|item| item.key == KEY_FINGERPRINT)
        .unwrap()
        .column;
    assert_eq!(fp_col, 4);
    assert!(tab.hidden.contains(&fp_col));
    assert!(!tab.warning_only.contains(&fp_col));
    assert!(tab
        .metadata
        .iter()
        .any(|item| { item.key == KEY_COLUMN && item.value == "Name" && item.column == 0 }));
    let formula = book.formulas.last().unwrap();
    assert!(formula.contains("A2:A"));
    assert!(formula.contains("B2:B"));
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
}

#[test]
fn cycle_normalized_formula_commits() {
    let store = scratch();
    let (mut book, watch) = prepared(&[Seed {
        sheet_row: 2,
        name: "Ann",
        city: "Oslo",
        uid: ANN,
        fingerprint: "fp-ann",
    }]);
    book.normalize_formulas = true;
    book.tab_mut(0)
        .unwrap()
        .metadata
        .iter_mut()
        .find(|item| item.key == KEY_FINGERPRINT)
        .unwrap()
        .value
        .clear();
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Wrote);
    assert_eq!(book.formulas.len(), 1);
    let stamp = book
        .tab(0)
        .unwrap()
        .metadata
        .iter()
        .find(|item| item.key == KEY_FINGERPRINT)
        .unwrap()
        .value
        .clone();
    assert_eq!(stamp, formula_stamp(book.formulas.last().unwrap()));
    book.formulas.clear();
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert!(book.formulas.is_empty());
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
}

#[test]
fn cycle_fetch_version_drops() {
    let store = scratch();
    let (mut book, watch) = prepared(&[Seed {
        sheet_row: 2,
        name: "Ann",
        city: "Oslo",
        uid: ANN,
        fingerprint: "fp-ann",
    }]);
    book.version_queue = ["4".into(), "4".into(), "9".into()].into_iter().collect();
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Dropped);
    assert!(store.read_stored(watch.wid).unwrap().rows.is_empty());
    assert!(store.watch(watch.wid).unwrap().drive_version.is_empty());
}

#[test]
fn cycle_first_scan_is_one_range() {
    let store = scratch();
    let mut tab = SheetTab::from_grid("Visitors", 0, &Grid::new(1, 0, BTreeMap::new()));
    tab.put_text(1, 0, "Name");
    tab.put_text(1, 1, "City");
    for index in 0..99 {
        let row = 2 + index;
        tab.put_text(row, 0, format!("n{index}"));
        tab.put_text(row, 1, format!("c{index}"));
        tab.put_text(row, 2, format!("{:016X}", index + 1));
        tab.put_text(row, 3, format!("fp-{index}"));
    }
    tab.metadata = vec![
        Meta {
            key: KEY_COLUMN.into(),
            column: 0,
            value: "Name".into(),
        },
        Meta {
            key: KEY_COLUMN.into(),
            column: 1,
            value: "City".into(),
        },
        Meta {
            key: KEY_UID.into(),
            column: 2,
            value: String::new(),
        },
        Meta {
            key: KEY_FINGERPRINT.into(),
            column: 3,
            value: String::new(),
        },
    ];
    install_formula(
        &mut tab,
        3,
        &[
            FormulaColumn {
                excel_id: "A",
                name: "Name",
                exclude_from_hash: false,
            },
            FormulaColumn {
                excel_id: "B",
                name: "City",
                exclude_from_hash: false,
            },
        ],
    );
    let mut book = FakeBook::new("sheet-1");
    book.tabs.push(tab);
    let watch = CycleWatch {
        wid: 1,
        spreadsheet_id: "sheet-1".into(),
        sheet_id: 0,
        header_row: 1,
        columns: vec![
            MappedColumn {
                name: "Name".into(),
                exclude_from_hash: false,
            },
            MappedColumn {
                name: "City".into(),
                exclude_from_hash: false,
            },
        ],
    };
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let mapped: Vec<_> = book
        .gets
        .iter()
        .flatten()
        .filter(|range| range.start_col == 0)
        .collect();
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].start_row, 2);
    assert_eq!(mapped[0].end_row, Some(100));
    assert_eq!(mapped[0].end_col, 1);
    assert_eq!(store.read_stored(watch.wid).unwrap().rows.len(), 99);
}

#[test]
fn cycle_reads_past_10000() {
    let store = scratch();
    let (mut book, watch) = prepared(&[
        Seed {
            sheet_row: 2,
            name: "Ann",
            city: "Oslo",
            uid: ANN,
            fingerprint: "fp-ann",
        },
        Seed {
            sheet_row: 10_001,
            name: "Tail",
            city: "Riga",
            uid: BO,
            fingerprint: "fp-tail",
        },
    ]);
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    let open = book
        .gets
        .iter()
        .flatten()
        .any(|range| range.start_col == 2 && range.start_row == 2 && range.end_row.is_none());
    assert!(open);
    let rows = store.read_stored(watch.wid).unwrap().rows;
    let rows_on_sheet: Vec<u32> = rows.iter().map(|row| row.sheet_row).collect();
    assert_eq!(rows_on_sheet, vec![2, 10_001]);
    assert!(rows.iter().any(|row| cell_text(row, "Name") == "Tail"));
    book.gets.clear();
    assert_eq!(run(&store, &mut book, &watch), CycleStop::Committed);
    assert_eq!(kinds(&store, watch.wid), vec![ChangeKind::Snapshot as i32]);
    let again = store.read_stored(watch.wid).unwrap().rows;
    assert!(again.iter().any(|row| row.sheet_row == 10_001));
}

fn stamp_formula(tab: &mut SheetTab, formula: &str) {
    if let Some(item) = tab
        .metadata
        .iter_mut()
        .find(|item| item.key == KEY_FINGERPRINT)
    {
        item.value = formula_stamp(formula);
    }
}

fn install_formula(tab: &mut SheetTab, col: u32, columns: &[FormulaColumn<'_>]) {
    let formula = fingerprint_formula(columns, 2);
    tab.put_formula(1, col, &formula);
    stamp_formula(tab, &formula);
}
