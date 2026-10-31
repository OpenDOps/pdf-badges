use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use google_tables::google::{FakeBook, Meta, SheetTab, KEY_COLUMN, KEY_FINGERPRINT, KEY_UID};
use google_tables::irbis::google_tables::v1::{
    ChangeKind, EnsureIdentityRequest, GetSnapshotRequest, GetWatchRequest, ListSnapshotsRequest,
    OpenSpreadsheetRequest, ReadStoredRequest, SelectHeaderRequest, SnapshotReason,
    StartWatchRequest, StopWatchRequest, SubscribeRequest, SyncNowRequest,
};
use google_tables::irbis::table::v1::{value, Row};
use google_tables::model::Grid;
use google_tables::server::GoogleTablesService;
use google_tables::uid::{fingerprint_formula, formula_stamp, FormulaColumn};
use tonic::Code;

fn scratch() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = root.join(format!("google-tables-watch-{nanos}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("google-tables.lmdb")
}

fn fixture() -> FakeBook {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/spreadsheet.json"
    ))
    .unwrap();
    FakeBook::from_fixture("sheet-1", &bytes).unwrap()
}

fn service(path: &std::path::Path) -> GoogleTablesService {
    let service = GoogleTablesService::open_manual(path).unwrap();
    service.insert_book(fixture());
    service
}

fn session(service: &GoogleTablesService) -> String {
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "sheet-1".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    service
        .select_header(SelectHeaderRequest {
            session_id: view.session_id.clone(),
            excel_row: 2,
            subcolumn_row: 0,
        })
        .unwrap();
    service
        .ensure_identity(EnsureIdentityRequest {
            session_id: view.session_id.clone(),
        })
        .unwrap();
    view.session_id
}

async fn pump_snapshot(service: &GoogleTablesService, session_id: &str) -> String {
    let watch = service
        .start_watch(StartWatchRequest {
            session_id: session_id.into(),
            drive_push: false,
        })
        .unwrap();
    service.quiet_blocked().await;
    service.advance_quiet().await;
    service.advance_quiet().await;
    assert!(service.quiet_waits().contains(&Duration::from_secs(10)));
    watch.watch_id
}

fn text_at(row: &Row, excel_id: &str) -> String {
    row.cells
        .iter()
        .find(|cell| cell.excel_id == excel_id)
        .and_then(|cell| match cell.value.as_ref()?.kind.clone() {
            Some(value::Kind::Text(text)) => Some(text),
            _ => None,
        })
        .unwrap_or_default()
}

fn stored_rows(service: &GoogleTablesService, watch_id: &str) -> (u64, Vec<Row>) {
    let batches = service
        .read_stored_batches(ReadStoredRequest {
            watch_id: watch_id.into(),
        })
        .unwrap();
    let seq = batches.first().map(|batch| batch.snapshot_seq).unwrap_or(0);
    let rows = batches.into_iter().flat_map(|batch| batch.rows).collect();
    (seq, rows)
}

fn kind_of(
    events: &[google_tables::irbis::google_tables::v1::RowChange],
    kind: ChangeKind,
) -> usize {
    events
        .iter()
        .filter(|event| event.kind == kind as i32)
        .count()
}

#[tokio::test]
async fn watch_first_scan_is_snapshot() {
    let path = scratch();
    let service = service(&path);
    let session_id = session(&service);
    let watch_id = pump_snapshot(&service, &session_id).await;
    let blocked = service
        .select_header(SelectHeaderRequest {
            session_id,
            excel_row: 2,
            subcolumn_row: 0,
        })
        .unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition);
    let events = service.changes(&watch_id).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, ChangeKind::Snapshot as i32);
    let (seq, rows) = stored_rows(&service, &watch_id);
    assert_eq!(seq, events[0].seq);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| text_at(row, "A") == "Ann"));
    assert!(rows.iter().any(|row| text_at(row, "A") == "Bo"));
}

#[tokio::test]
async fn watch_edit_queues_updated() {
    let path = scratch();
    let service = service(&path);
    let watch_id = pump_snapshot(&service, &session(&service)).await;
    service
        .with_book("sheet-1", |book| {
            book.tab_mut(0).unwrap().put_text(4, 1, "Bergen");
            book.version += 1;
        })
        .unwrap();
    service.advance_quiet().await;
    service
        .with_book("sheet-1", |book| book.gets.clear())
        .unwrap();
    service.advance_quiet().await;
    let events = service.changes(&watch_id).unwrap();
    let updated: Vec<_> = events
        .iter()
        .filter(|event| event.kind == ChangeKind::Updated as i32)
        .collect();
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].changed_columns, vec!["City".to_string()]);
    let mapped: Vec<u32> = service
        .book("sheet-1", |book| {
            book.gets
                .iter()
                .flatten()
                .filter(|range| range.start_col == 0)
                .map(|range| range.start_row)
                .collect()
        })
        .unwrap();
    assert_eq!(mapped, vec![4]);
}

#[tokio::test]
async fn watch_delete_queues_deleted() {
    let path = scratch();
    let service = service(&path);
    let watch_id = pump_snapshot(&service, &session(&service)).await;
    let bo_uid = service
        .book("sheet-1", |book| {
            let tab = book.tab(0).unwrap();
            let uid_col = tab
                .metadata
                .iter()
                .find(|item| item.key == KEY_UID)
                .unwrap()
                .column;
            match tab
                .cells
                .get(&(5, uid_col))
                .and_then(|cell| cell.value.clone())
            {
                Some(google_tables::model::CellValue::Text(text)) => text,
                _ => String::new(),
            }
        })
        .unwrap();
    service
        .with_book("sheet-1", |book| {
            let uid_col = book
                .tab(0)
                .unwrap()
                .metadata
                .iter()
                .find(|item| item.key == KEY_UID)
                .unwrap()
                .column;
            let tab = book.tab_mut(0).unwrap();
            for col in [0, 1, 2, uid_col] {
                tab.cells.remove(&(5, col));
            }
            book.version += 1;
        })
        .unwrap();
    service.advance_quiet().await;
    service.advance_quiet().await;
    let events = service.changes(&watch_id).unwrap();
    let deleted: Vec<_> = events
        .iter()
        .filter(|event| event.kind == ChangeKind::Deleted as i32)
        .collect();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].uid, bo_uid);
}

#[tokio::test]
async fn watch_resume() {
    let path = scratch();
    let service = service(&path);
    let watch_id = pump_snapshot(&service, &session(&service)).await;
    let (seq, _) = stored_rows(&service, &watch_id);
    let mut stream = service
        .subscribe(SubscribeRequest {
            watch_id: watch_id.clone(),
            after_seq: seq,
        })
        .unwrap();
    service
        .with_book("sheet-1", |book| {
            book.tab_mut(0).unwrap().put_text(4, 1, "Bergen");
            book.version += 1;
        })
        .unwrap();
    service.advance_quiet().await;
    service.advance_quiet().await;
    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("subscriber timed out")
        .expect("stream open")
        .expect("event");
    assert_eq!(event.kind, ChangeKind::Updated as i32);
    assert_ne!(event.kind, ChangeKind::Snapshot as i32);
    assert!(event.seq > seq);
}

#[tokio::test]
async fn watch_restart() {
    let path = scratch();
    let service = service(&path);
    let watch_id = pump_snapshot(&service, &session(&service)).await;
    let uids = service.book("sheet-1", |book| uid_texts(book)).unwrap();
    let mut book = service.book("sheet-1", |book| book.clone()).unwrap();
    service.shutdown().await;
    drop(service);
    book.raw_writes.clear();
    book.gets.clear();
    let service = GoogleTablesService::open_manual(&path).unwrap();
    let watch = service
        .get_watch(GetWatchRequest {
            watch_id: watch_id.clone(),
        })
        .unwrap();
    assert_eq!(watch.watch_id, watch_id);
    assert_eq!(watch.spreadsheet_id, "sheet-1");
    assert_eq!(watch.status, "running");
    service.insert_book(book);
    service.quiet_blocked().await;
    service.advance_quiet().await;
    let after = service
        .book("sheet-1", |book| {
            assert!(book.raw_writes.is_empty());
            uid_texts(book)
        })
        .unwrap();
    assert_eq!(after, uids);
}

#[tokio::test]
async fn watch_sync_now() {
    let path = scratch();
    let service = service(&path);
    let watch_id = pump_snapshot(&service, &session(&service)).await;
    service
        .with_book("sheet-1", |book| {
            book.tab_mut(0).unwrap().put_text(4, 1, "Bergen");
            book.version += 1;
        })
        .unwrap();
    let report = service
        .sync_now(SyncNowRequest {
            watch_id: watch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(report.updated, 1);
    let events = service.changes(&watch_id).unwrap();
    assert_eq!(kind_of(&events, ChangeKind::Updated), 1);
}

#[tokio::test]
async fn watch_out_of_range() {
    let path = scratch();
    let service = GoogleTablesService::open_manual_retain(&path, 1).unwrap();
    service.insert_book(fixture());
    let watch_id = pump_snapshot(&service, &session(&service)).await;
    service
        .with_book("sheet-1", |book| {
            book.tab_mut(0).unwrap().put_text(4, 1, "Bergen");
            book.version += 1;
        })
        .unwrap();
    service.advance_quiet().await;
    service.advance_quiet().await;
    let missing = match service.subscribe(SubscribeRequest {
        watch_id,
        after_seq: 0,
    }) {
        Ok(_) => panic!("expected OUT_OF_RANGE"),
        Err(status) => status,
    };
    assert_eq!(missing.code(), Code::OutOfRange);
}

#[tokio::test]
async fn watch_mass_delete_snapshots() {
    let path = scratch();
    let service = GoogleTablesService::open_manual(&path).unwrap();
    let mut tab = SheetTab::from_grid(
        "Visitors",
        0,
        &Grid::new(1, 0, std::collections::BTreeMap::new()),
    );
    tab.put_text(1, 0, "Name");
    for index in 0..10 {
        let row = 2 + index;
        tab.put_text(row, 0, format!("n{index}"));
        tab.put_text(row, 1, format!("{:016X}", index + 1));
        tab.put_text(row, 2, format!("fp-{index}"));
    }
    tab.metadata = vec![
        Meta {
            key: KEY_COLUMN.into(),
            column: 0,
            value: "Name".into(),
        },
        Meta {
            key: KEY_UID.into(),
            column: 1,
            value: String::new(),
        },
        Meta {
            key: KEY_FINGERPRINT.into(),
            column: 2,
            value: String::new(),
        },
    ];
    install_formula(
        &mut tab,
        2,
        &[FormulaColumn {
            excel_id: "A",
            name: "Name",
            exclude_from_hash: false,
        }],
    );
    let mut book = FakeBook::new("mass");
    book.tabs.push(tab);
    service.insert_book(book);
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "mass".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    service
        .select_header(SelectHeaderRequest {
            session_id: view.session_id.clone(),
            excel_row: 1,
            subcolumn_row: 0,
        })
        .unwrap();
    let watch = service
        .start_watch(StartWatchRequest {
            session_id: view.session_id,
            drive_push: false,
        })
        .unwrap();
    service.quiet_blocked().await;
    service.advance_quiet().await;
    service.with_book("mass", |book| {
        let tab = book.tab_mut(0).unwrap();
        for row in 8..=11 {
            for col in 0..=2 {
                tab.cells.remove(&(row, col));
            }
        }
        book.version += 1;
    });
    let applied = service
        .sync_now(SyncNowRequest {
            watch_id: watch.watch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(applied.deleted, 4);
    let current = service
        .get_watch(GetWatchRequest {
            watch_id: watch.watch_id.clone(),
        })
        .unwrap();
    assert_eq!(current.status, "running");
    let listed = service
        .list_snapshots(ListSnapshotsRequest {
            watch_id: watch.watch_id.clone(),
        })
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].reason, SnapshotReason::Initial as i32);
    assert_eq!(listed[1].reason, SnapshotReason::MassiveDelete as i32);
    assert!(listed[0].created_at_unix_ms <= listed[1].created_at_unix_ms);
    assert_eq!(listed[1].row_count, 10);
    let batches = service
        .get_snapshot_batches(GetSnapshotRequest {
            watch_id: watch.watch_id.clone(),
            snapshot_id: listed[1].snapshot_id,
        })
        .unwrap();
    let rows: Vec<_> = batches.into_iter().flat_map(|batch| batch.rows).collect();
    assert_eq!(rows.len(), 10);
    let missing = service.get_snapshot_batches(GetSnapshotRequest {
        watch_id: watch.watch_id.clone(),
        snapshot_id: 99,
    });
    assert_eq!(missing.unwrap_err().code(), Code::NotFound);
    let _ = service.stop_watch(StopWatchRequest {
        watch_id: watch.watch_id,
    });
}

#[tokio::test]
async fn watch_mass_delete_is_quiet() {
    let path = scratch();
    let service = GoogleTablesService::open_manual(&path).unwrap();
    service.insert_book(numbered_sheet("sheet-1", 10));
    let watch_id = watch_numbered(&service, "sheet-1").await;
    service
        .with_book("sheet-1", |book| {
            let tab = book.tab_mut(0).unwrap();
            for row in 8..=11 {
                for col in 0..=2 {
                    tab.cells.remove(&(row, col));
                }
            }
            book.version += 1;
            book.gets.clear();
        })
        .unwrap();
    service.advance_quiet().await;
    assert_eq!(service.book("sheet-1", |book| book.gets.len()).unwrap(), 0);
    service.advance_quiet().await;
    let current = service
        .get_watch(GetWatchRequest {
            watch_id: watch_id.clone(),
        })
        .unwrap();
    assert_eq!(current.status, "running");
    assert_eq!(current.drive_version, "2");
    assert_eq!(
        kind_of(&service.changes(&watch_id).unwrap(), ChangeKind::Deleted),
        4
    );
    let reads = service.book("sheet-1", |book| book.gets.len()).unwrap();
    assert!(reads > 0);
    assert_eq!(
        service.quiet_waits().last().copied(),
        Some(Duration::from_secs(60))
    );
    service.advance_quiet().await;
    assert_eq!(
        service.book("sheet-1", |book| book.gets.len()).unwrap(),
        reads
    );
    let _ = service.stop_watch(StopWatchRequest { watch_id });
}

#[tokio::test]
async fn watch_sync_drops_while_version_moves() {
    let path = scratch();
    let service = service(&path);
    let watch_id = pump_snapshot(&service, &session(&service)).await;
    service.shutdown().await;
    service
        .with_book("sheet-1", |book| {
            book.tab_mut(0).unwrap().put_text(4, 1, "Bergen");
            book.version_queue.push_back("9".into());
            book.version_queue.push_back("10".into());
        })
        .unwrap();
    let report = service
        .sync_now(SyncNowRequest {
            watch_id: watch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(report.updated, 0);
    assert_eq!(report.added, 0);
    assert_eq!(report.deleted, 0);
    let (_, rows) = stored_rows(&service, &watch_id);
    let ann = rows.iter().find(|row| row.excel_row == 4).unwrap();
    assert_ne!(text_at(ann, "B"), "Bergen");
}

#[tokio::test]
async fn watch_mass_delete_follows_uid_write() {
    let path = scratch();
    let service = GoogleTablesService::open_manual(&path).unwrap();
    let mut tab = SheetTab::from_grid(
        "Visitors",
        0,
        &Grid::new(1, 0, std::collections::BTreeMap::new()),
    );
    tab.put_text(1, 0, "Name");
    for index in 0..10 {
        let row = 2 + index;
        tab.put_text(row, 0, format!("n{index}"));
        tab.put_text(row, 1, format!("{:016X}", index + 1));
        tab.put_text(row, 2, format!("fp-{index}"));
    }
    tab.metadata = vec![
        Meta {
            key: KEY_COLUMN.into(),
            column: 0,
            value: "Name".into(),
        },
        Meta {
            key: KEY_UID.into(),
            column: 1,
            value: String::new(),
        },
        Meta {
            key: KEY_FINGERPRINT.into(),
            column: 2,
            value: String::new(),
        },
    ];
    install_formula(
        &mut tab,
        2,
        &[FormulaColumn {
            excel_id: "A",
            name: "Name",
            exclude_from_hash: false,
        }],
    );
    let mut book = FakeBook::new("mass");
    book.tabs.push(tab);
    service.insert_book(book);
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "mass".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    service
        .select_header(SelectHeaderRequest {
            session_id: view.session_id.clone(),
            excel_row: 1,
            subcolumn_row: 0,
        })
        .unwrap();
    let watch = service
        .start_watch(StartWatchRequest {
            session_id: view.session_id,
            drive_push: false,
        })
        .unwrap();
    service.quiet_blocked().await;
    service.advance_quiet().await;
    service.with_book("mass", |book| {
        let tab = book.tab_mut(0).unwrap();
        tab.cells.remove(&(2, 1));
        for row in 8..=11 {
            for col in 0..=2 {
                tab.cells.remove(&(row, col));
            }
        }
        book.version += 1;
    });
    let report = service
        .sync_now(SyncNowRequest {
            watch_id: watch.watch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(report.deleted, 0);
    assert_eq!(
        service
            .get_watch(GetWatchRequest {
                watch_id: watch.watch_id.clone(),
            })
            .unwrap()
            .status,
        "running"
    );
    let written = service
        .book("mass", |book| book.raw_writes.last().unwrap().cells.clone())
        .unwrap();
    assert_eq!(written, vec![(2, 1, "0000000000000001".to_string())]);
    service.advance_quiet().await;
    service.advance_quiet().await;
    assert_eq!(
        kind_of(
            &service.changes(&watch.watch_id).unwrap(),
            ChangeKind::Deleted
        ),
        4
    );
    let watch = service
        .get_watch(GetWatchRequest {
            watch_id: watch.watch_id.clone(),
        })
        .unwrap();
    assert_eq!(watch.status, "running");
    let uid = service
        .book("mass", |book| {
            book.tab(0)
                .unwrap()
                .cells
                .get(&(2, 1))
                .and_then(|cell| cell.value.as_ref())
                .and_then(|value| match value {
                    google_tables::model::CellValue::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        })
        .unwrap();
    assert_eq!(uid, "0000000000000001");
    let _ = service.stop_watch(StopWatchRequest {
        watch_id: watch.watch_id,
    });
}

#[tokio::test]
async fn watch_cycle_backoff() {
    let path = scratch();
    let service = GoogleTablesService::open_manual(&path).unwrap();
    service.insert_book(numbered_sheet("sheet-1", 1));
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "sheet-1".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    service
        .select_header(SelectHeaderRequest {
            session_id: view.session_id.clone(),
            excel_row: 1,
            subcolumn_row: 0,
        })
        .unwrap();
    let watch = service
        .start_watch(StartWatchRequest {
            session_id: view.session_id,
            drive_push: false,
        })
        .unwrap();
    let watch_id = watch.watch_id;
    service.quiet_blocked().await;
    service
        .with_book("sheet-1", |book| {
            book.gets.clear();
            book.rate_limit = [false, false, false, false, false, true]
                .into_iter()
                .collect();
        })
        .unwrap();
    service.advance_quiet().await;
    assert!(service.cycle_busy(&watch_id));
    assert_eq!(service.book("sheet-1", |book| book.gets.len()).unwrap(), 0);
    assert_eq!(
        service.quiet_waits().last().copied(),
        Some(Duration::from_secs(1))
    );
    service.advance_quiet().await;
    assert!(!service.cycle_busy(&watch_id));
    let watch = service
        .get_watch(GetWatchRequest {
            watch_id: watch_id.clone(),
        })
        .unwrap();
    assert_eq!(watch.status, "running");
    assert!(watch.last_seq > 0);
    assert!(service.book("sheet-1", |book| book.gets.len()).unwrap() > 0);
    let _ = service.stop_watch(StopWatchRequest { watch_id });
}

#[tokio::test]
async fn watch_backoff_is_quiet() {
    let path = scratch();
    let service = service(&path);
    let session_id = session(&service);
    service
        .with_book("sheet-1", |book| {
            book.gets.clear();
            book.rate_limit = [true, true, false].into_iter().collect();
        })
        .unwrap();
    service
        .start_watch(StartWatchRequest {
            session_id,
            drive_push: false,
        })
        .unwrap();
    service.quiet_blocked().await;
    assert_eq!(
        service.quiet_waits().last().copied(),
        Some(Duration::from_secs(1))
    );
    assert_eq!(service.book("sheet-1", |book| book.gets.len()).unwrap(), 0);
    service.advance_quiet().await;
    assert_eq!(
        service.quiet_waits().last().copied(),
        Some(Duration::from_secs(2))
    );
    assert_eq!(service.book("sheet-1", |book| book.gets.len()).unwrap(), 0);
    service.shutdown().await;
}

#[tokio::test]
async fn watch_missing_column_pauses() {
    let path = scratch();
    let service = service(&path);
    let watch_id = pump_snapshot(&service, &session(&service)).await;
    service
        .with_book("sheet-1", |book| {
            book.tab_mut(0)
                .unwrap()
                .metadata
                .retain(|item| !(item.key == KEY_COLUMN && item.value == "City"));
            book.version += 1;
            book.gets.clear();
        })
        .unwrap();
    service.advance_quiet().await;
    service.advance_quiet().await;
    let watch = service
        .get_watch(GetWatchRequest {
            watch_id: watch_id.clone(),
        })
        .unwrap();
    assert_eq!(watch.status, "column_missing");
    let gets = service.book("sheet-1", |book| book.gets.len()).unwrap();
    assert_eq!(gets, 0);
    service.advance_quiet().await;
    assert_eq!(service.book("sheet-1", |book| book.gets.len()).unwrap(), 0);
    assert_eq!(
        service.quiet_waits().last().copied(),
        Some(Duration::from_secs(60))
    );
    let still = service
        .book("sheet-1", |book| {
            book.tab(0)
                .unwrap()
                .metadata
                .iter()
                .all(|item| item.value != "City")
        })
        .unwrap();
    assert!(still);
    let _ = service.stop_watch(StopWatchRequest { watch_id });
}

fn numbered_sheet(id: &str, rows: u32) -> FakeBook {
    let mut tab = SheetTab::from_grid(
        "Visitors",
        0,
        &Grid::new(1, 0, std::collections::BTreeMap::new()),
    );
    tab.put_text(1, 0, "Name");
    for index in 0..rows {
        let row = 2 + index;
        tab.put_text(row, 0, format!("n{index}"));
        tab.put_text(row, 1, format!("{:016X}", index + 1));
        tab.put_text(row, 2, format!("fp-{index}"));
    }
    tab.metadata = vec![
        Meta {
            key: KEY_COLUMN.into(),
            column: 0,
            value: "Name".into(),
        },
        Meta {
            key: KEY_UID.into(),
            column: 1,
            value: String::new(),
        },
        Meta {
            key: KEY_FINGERPRINT.into(),
            column: 2,
            value: String::new(),
        },
    ];
    install_formula(
        &mut tab,
        2,
        &[FormulaColumn {
            excel_id: "A",
            name: "Name",
            exclude_from_hash: false,
        }],
    );
    let mut book = FakeBook::new(id);
    book.tabs.push(tab);
    book
}

async fn watch_numbered(service: &GoogleTablesService, id: &str) -> String {
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: id.into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    service
        .select_header(SelectHeaderRequest {
            session_id: view.session_id.clone(),
            excel_row: 1,
            subcolumn_row: 0,
        })
        .unwrap();
    let watch = service
        .start_watch(StartWatchRequest {
            session_id: view.session_id,
            drive_push: false,
        })
        .unwrap();
    service.quiet_blocked().await;
    service.advance_quiet().await;
    watch.watch_id
}

fn rewrite_fingerprints(service: &GoogleTablesService, id: &str, rows: u32) {
    service
        .with_book(id, |book| {
            let tab = book.tab_mut(0).unwrap();
            for index in 0..rows {
                tab.put_text(2 + index, 0, format!("n{index}-b"));
                tab.put_text(2 + index, 2, format!("fp-{index}-b"));
            }
        })
        .unwrap();
}

#[tokio::test]
async fn watch_resume_large_backlog() {
    const ROWS: u32 = 2_500;
    let path = scratch();
    let service = GoogleTablesService::open_manual(&path).unwrap();
    service.insert_book(numbered_sheet("wide", ROWS));
    let watch_id = watch_numbered(&service, "wide").await;
    let snapshot = service
        .get_watch(GetWatchRequest {
            watch_id: watch_id.clone(),
        })
        .unwrap()
        .last_seq;
    rewrite_fingerprints(&service, "wide", ROWS);
    let report = service
        .sync_now(SyncNowRequest {
            watch_id: watch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(report.updated, ROWS);
    let mut stream = service
        .subscribe(SubscribeRequest {
            watch_id: watch_id.clone(),
            after_seq: snapshot,
        })
        .unwrap();
    for _ in 0..ROWS {
        let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("replay timed out")
            .expect("stream closed")
            .expect("event");
        assert_eq!(event.kind, ChangeKind::Updated as i32);
    }
    let _ = service.stop_watch(StopWatchRequest { watch_id });
}

#[tokio::test]
async fn watch_live_lag_closes() {
    const ROWS: u32 = 1_001;
    let path = scratch();
    let service = GoogleTablesService::open_manual(&path).unwrap();
    service.insert_book(numbered_sheet("wide", ROWS));
    let watch_id = watch_numbered(&service, "wide").await;
    let snapshot = service
        .get_watch(GetWatchRequest {
            watch_id: watch_id.clone(),
        })
        .unwrap()
        .last_seq;
    let mut stream = service
        .subscribe(SubscribeRequest {
            watch_id: watch_id.clone(),
            after_seq: snapshot,
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while service.subscribers_idle() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("subscriber did not park");
    rewrite_fingerprints(&service, "wide", ROWS);
    let report = service
        .sync_now(SyncNowRequest {
            watch_id: watch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(report.updated, ROWS);
    let status = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("close timed out")
        .expect("stream closed")
        .expect_err("live lag");
    assert_eq!(status.code(), Code::ResourceExhausted);
    let _ = service.stop_watch(StopWatchRequest { watch_id });
}

fn uid_texts(book: &FakeBook) -> Vec<String> {
    let tab = book.tab(0).unwrap();
    let uid_col = tab
        .metadata
        .iter()
        .find(|item| item.key == KEY_UID)
        .unwrap()
        .column;
    [4u32, 5]
        .into_iter()
        .map(|row| {
            match tab
                .cells
                .get(&(row, uid_col))
                .and_then(|cell| cell.value.clone())
            {
                Some(google_tables::model::CellValue::Text(text)) => text,
                _ => String::new(),
            }
        })
        .collect()
}

fn install_formula(tab: &mut SheetTab, col: u32, columns: &[FormulaColumn<'_>]) {
    let formula = fingerprint_formula(columns, 2);
    tab.put_formula(1, col, &formula);
    if let Some(item) = tab
        .metadata
        .iter_mut()
        .find(|item| item.key == KEY_FINGERPRINT)
    {
        item.value = formula_stamp(&formula);
    }
}
