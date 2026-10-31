use std::path::PathBuf;

use google_tables::model::CellValue;
use google_tables::scan::{columns_for_header, data_rows, header_candidates, GridSource};
use google_tables::sheets::SheetsSource;

fn fixture() -> SheetsSource {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spreadsheet.json");
    SheetsSource::open(&path).expect("recorded spreadsheet")
}

fn selected() -> (
    google_tables::model::Grid,
    Vec<google_tables::model::Column>,
    Vec<google_tables::model::DataRow>,
) {
    let source = fixture();
    assert!(source.has_values());
    let grid = source.grid();
    let candidates = header_candidates(&grid, 10);
    let columns = columns_for_header(&grid, 2, &candidates, None).expect("header row 2");
    let rows = data_rows(&grid, 2, &columns, None);
    (grid, columns, rows)
}

#[test]
fn grid_payload_header_is_row_2() {
    let source = fixture();
    let grid = source.grid();
    let candidates = header_candidates(&grid, 10);
    assert!(candidates.iter().any(|candidate| candidate.excel_row == 2));
    let columns = columns_for_header(&grid, 2, &candidates, None).unwrap();
    assert_eq!(
        columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Name", "City", "Amount"]
    );
    let rows = data_rows(&grid, 2, &columns, None);
    assert!(rows.iter().all(|row| row.excel_row != 1));
}

#[test]
fn grid_payload_date_is_civil() {
    let (_, _, rows) = selected();
    let date = rows
        .iter()
        .flat_map(|row| row.cells.iter())
        .find_map(|cell| match &cell.value {
            Some(CellValue::Date(date)) => Some(*date),
            _ => None,
        })
        .expect("date cell");
    assert_eq!(
        (date.year, date.month, date.day, date.has_time),
        (2024, 3, 15, false)
    );
    assert!(rows
        .iter()
        .flat_map(|row| row.cells.iter())
        .any(|cell| { matches!(cell.value, Some(CellValue::Int(2))) }));
    assert!(rows
        .iter()
        .flat_map(|row| row.cells.iter())
        .any(|cell| { matches!(cell.value, Some(CellValue::Text(ref text)) if text == "Ann") }));
}

#[test]
fn grid_payload_empty_row_skipped() {
    let (_, _, rows) = selected();
    assert_eq!(
        rows.iter().map(|row| row.excel_row).collect::<Vec<_>>(),
        vec![4, 5]
    );
    let ann = &rows[0];
    assert!(ann.cells.iter().all(|cell| cell.col != 1));
    assert_eq!(ann.cells.len(), 2);
}
