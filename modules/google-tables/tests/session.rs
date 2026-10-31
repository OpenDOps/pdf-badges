use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use google_tables::google::{FakeBook, SheetTab};
use google_tables::irbis::google_tables::v1::{
    CloseRequest, ColumnOption, EnsureIdentityRequest, OpenSpreadsheetRequest, ReadTableRequest,
    SelectHeaderRequest, SetColumnOptionsRequest,
};
use google_tables::irbis::table::v1::value;
use google_tables::model::{CellValue, Grid};
use google_tables::server::GoogleTablesService;
use google_tables::uid::is_uid;
use tonic::Code;

fn fixture() -> FakeBook {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/spreadsheet.json"
    ))
    .unwrap();
    FakeBook::from_fixture("sheet-1", &bytes).unwrap()
}

fn open_fixture() -> (GoogleTablesService, String) {
    let service = GoogleTablesService::new();
    service.insert_book(fixture());
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "sheet-1".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    (service, view.session_id)
}

fn select_row(service: &GoogleTablesService, session_id: &str, excel_row: u32) {
    service
        .select_header(SelectHeaderRequest {
            session_id: session_id.into(),
            excel_row,
            subcolumn_row: 0,
        })
        .unwrap();
}

fn cell_text(cell: &google_tables::irbis::table::v1::Cell) -> String {
    match cell.value.as_ref().and_then(|value| value.kind.clone()) {
        Some(value::Kind::Text(text)) => text,
        _ => String::new(),
    }
}

#[test]
fn session_select_header() {
    let service = GoogleTablesService::new();
    let slash = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "sheet-1".into(),
            credentials_ref: "secrets/local".into(),
            header_candidate_limit: 0,
        })
        .unwrap_err();
    assert_eq!(slash.code(), Code::InvalidArgument);

    service.insert_book(fixture());
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "sheet-1".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    let table = service
        .select_header(SelectHeaderRequest {
            session_id: view.session_id.clone(),
            excel_row: 2,
            subcolumn_row: 0,
        })
        .unwrap();
    assert!(table.rows.is_empty());
    assert_eq!(
        table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Name", "City", "Amount"]
    );
    service
        .close(CloseRequest {
            session_id: view.session_id.clone(),
        })
        .unwrap();
    let missing = service
        .select_header(SelectHeaderRequest {
            session_id: view.session_id,
            excel_row: 2,
            subcolumn_row: 0,
        })
        .unwrap_err();
    assert_eq!(missing.code(), Code::NotFound);
}

#[test]
fn session_identity_appends() {
    let (service, session_id) = open_fixture();
    select_row(&service, &session_id, 2);
    service
        .ensure_identity(EnsureIdentityRequest {
            session_id: session_id.clone(),
        })
        .unwrap();
    service
        .book("sheet-1", |book| {
            let tab = book.tab(0).unwrap();
            assert_eq!(tab.hidden.len(), 2);
            assert!(tab.hidden.iter().all(|col| *col >= 3));
            assert_eq!(
                tab.cells.get(&(2, 0)).and_then(|cell| cell.value.clone()),
                Some(CellValue::Text("Name".into()))
            );
            assert_eq!(
                tab.cells.get(&(2, 1)).and_then(|cell| cell.value.clone()),
                Some(CellValue::Text("City".into()))
            );
            assert_eq!(
                tab.cells.get(&(2, 2)).and_then(|cell| cell.value.clone()),
                Some(CellValue::Text("Amount".into()))
            );
            let uid_col = tab.hidden.iter().copied().min().unwrap();
            assert_eq!(tab.warning_only, [uid_col].into_iter().collect());
            assert!(tab.metadata.iter().any(|item| item.key == "irbis.uid"));
            assert!(tab
                .metadata
                .iter()
                .any(|item| item.key == "irbis.fingerprint"));
        })
        .unwrap();
}

#[test]
fn session_read_fills_uids() {
    let (service, session_id) = open_fixture();
    select_row(&service, &session_id, 2);
    service
        .ensure_identity(EnsureIdentityRequest {
            session_id: session_id.clone(),
        })
        .unwrap();
    let batches = service
        .read_table(ReadTableRequest {
            session_id: session_id.clone(),
        })
        .unwrap();
    assert_eq!(batches.len(), 1);
    let rows = &batches[0].rows;
    assert_eq!(
        rows.iter().map(|row| row.excel_row).collect::<Vec<_>>(),
        vec![4, 5]
    );
    for row in rows {
        assert!(row.cells.iter().any(|cell| is_uid(&cell_text(cell))));
    }
    service
        .book("sheet-1", |book| {
            assert_eq!(book.raw_writes.len(), 1);
            assert_eq!(book.raw_writes[0].cells.len(), 2);
            let column = book.raw_writes[0].cells[0].1;
            assert!(book.raw_writes[0]
                .cells
                .iter()
                .all(|(_, col, text)| *col == column && is_uid(text)));
        })
        .unwrap();
}

#[test]
fn session_exclude_roundtrip() {
    let (service, session_id) = open_fixture();
    select_row(&service, &session_id, 2);
    service
        .ensure_identity(EnsureIdentityRequest {
            session_id: session_id.clone(),
        })
        .unwrap();
    service
        .set_column_options(SetColumnOptionsRequest {
            session_id,
            options: vec![ColumnOption {
                name: "Amount".into(),
                exclude_from_hash: true,
            }],
        })
        .unwrap();
    service
        .book("sheet-1", |book| {
            assert!(book.formulas.len() >= 2);
            assert!(book.formulas[0].contains("C3"));
            let last = book.formulas.last().unwrap();
            assert!(last.contains("A3"));
            assert!(last.contains("B3"));
            assert!(!last.contains("C3"));
        })
        .unwrap();
}

#[test]
fn session_expires() {
    let start = Instant::now();
    let offset = Arc::new(Mutex::new(Duration::ZERO));
    let clock_offset = Arc::clone(&offset);
    let service = GoogleTablesService::with_clock(move || start + *clock_offset.lock().unwrap());
    service.insert_book(fixture());
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "sheet-1".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    *offset.lock().unwrap() = Duration::from_secs(15 * 60);
    let missing = service
        .select_header(SelectHeaderRequest {
            session_id: view.session_id,
            excel_row: 2,
            subcolumn_row: 0,
        })
        .unwrap_err();
    assert_eq!(missing.code(), Code::NotFound);
}

#[test]
fn session_gap_stops() {
    let mut tab = SheetTab::from_grid("Visitors", 0, &Grid::new(1, 0, BTreeMap::new()));
    tab.put_text(1, 0, "Name");
    tab.put_text(2, 0, "Keep");
    tab.put_text(53, 0, "Drop");
    let mut book = FakeBook::new("gap");
    book.tabs.push(tab);
    let service = GoogleTablesService::new();
    service.insert_book(book);
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "gap".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    select_row(&service, &view.session_id, 1);
    let batches = service
        .read_table(ReadTableRequest {
            session_id: view.session_id,
        })
        .unwrap();
    let rows: Vec<u32> = batches
        .iter()
        .flat_map(|batch| batch.rows.iter().map(|row| row.excel_row))
        .collect();
    assert_eq!(rows, vec![2]);
}

#[test]
fn session_batches() {
    let mut tab = SheetTab::from_grid("Visitors", 0, &Grid::new(1, 0, BTreeMap::new()));
    tab.put_text(1, 0, "Name");
    for row in 2..=102 {
        tab.put_text(row, 0, format!("r{row}"));
    }
    let mut book = FakeBook::new("wide");
    book.tabs.push(tab);
    let service = GoogleTablesService::new();
    service.insert_book(book);
    let view = service
        .open_spreadsheet(OpenSpreadsheetRequest {
            spreadsheet_id: "wide".into(),
            credentials_ref: "local".into(),
            header_candidate_limit: 0,
        })
        .unwrap();
    select_row(&service, &view.session_id, 1);
    let batches = service
        .read_table(ReadTableRequest {
            session_id: view.session_id,
        })
        .unwrap();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].rows.len(), 100);
    assert_eq!(batches[1].rows.len(), 1);
}
