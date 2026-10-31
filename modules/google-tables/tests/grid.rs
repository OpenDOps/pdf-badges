use std::collections::BTreeMap;

use google_tables::columns::{excel_id, parse_excel_id};
use google_tables::model::{CellValue, DateValue, Grid, GridCell};
use google_tables::scan::{
    columns_for_header, columns_without_header, data_rows, header_candidates, rows_without_header,
    ScanError,
};
use google_tables::values::{classify, Reading};

fn text(col: u32, display: &str) -> GridCell {
    GridCell::text(col, display)
}

fn grid(last_row: u32, last_col: u32, rows: BTreeMap<u32, Vec<GridCell>>) -> Grid {
    Grid::new(last_row, last_col, rows)
}

fn names(columns: &[google_tables::model::Column]) -> Vec<(String, String)> {
    columns
        .iter()
        .map(|column| (column.excel_id.clone(), column.name.clone()))
        .collect()
}

fn rows(pairs: &[(u32, Vec<GridCell>)]) -> BTreeMap<u32, Vec<GridCell>> {
    pairs.iter().cloned().collect()
}

const SERIAL_1900: f64 = 45366.0;
const SERIAL_1900_1345: f64 = 45366.572916666664;
const SERIAL_1904: f64 = 43904.0;

#[test]
fn grid_letters_and_headers() {
    assert_eq!(
        [0, 25, 26, 27].map(excel_id).to_vec(),
        vec!["A", "Z", "AA", "AB"]
    );
    assert_eq!(parse_excel_id("A"), Some(0));
    assert_eq!(parse_excel_id(" a "), Some(0));
    assert_eq!(parse_excel_id("AA"), Some(26));
    assert_eq!(parse_excel_id("XFD"), Some(16383));
    assert_eq!(excel_id(parse_excel_id("AB").unwrap()), "AB");
    assert_eq!(parse_excel_id(""), None);
    assert_eq!(parse_excel_id("A1"), None);
    assert_eq!(parse_excel_id("XFE"), None);

    let sheet = grid(
        4,
        2,
        rows(&[
            (2, vec![text(0, "Quarter report")]),
            (4, vec![text(0, "Name"), text(1, "City"), text(2, "Amount")]),
        ]),
    );
    let candidates = header_candidates(&sheet, 3);
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.excel_row)
            .collect::<Vec<_>>(),
        vec![2, 4]
    );
    assert_eq!(
        names(&candidates[1].columns),
        vec![
            ("A".into(), "Name".into()),
            ("B".into(), "City".into()),
            ("C".into(), "Amount".into())
        ]
    );

    let wide = grid(1_048_576, 0, rows(&[(2, vec![text(0, "Name")])]));
    assert_eq!(
        header_candidates(&wide, 10)
            .iter()
            .map(|candidate| candidate.excel_row)
            .collect::<Vec<_>>(),
        vec![2]
    );

    let limited = grid(
        4,
        0,
        (1..=4)
            .map(|row| (row, vec![text(0, &format!("r{row}"))]))
            .collect(),
    );
    assert_eq!(
        header_candidates(&limited, 2)
            .iter()
            .map(|candidate| candidate.excel_row)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    let blank_header = grid(1, 2, rows(&[(1, vec![text(0, "Name"), text(2, "Amount")])]));
    let columns =
        columns_for_header(&blank_header, 1, &header_candidates(&blank_header, 1), None).unwrap();
    assert_eq!(
        names(&columns),
        vec![("A".into(), "Name".into()), ("C".into(), "Amount".into())]
    );

    let whitespace = grid(1, 0, rows(&[(1, vec![text(0, "   ")])]));
    assert!(header_candidates(&whitespace, 1).is_empty());

    let duplicate = grid(1, 1, rows(&[(1, vec![text(0, "Name"), text(1, "Name")])]));
    let candidates = header_candidates(&duplicate, 1);
    assert!(matches!(
        columns_for_header(&duplicate, 1, &candidates, None),
        Err(ScanError::DuplicateColumn(_))
    ));

    let only_row_two = grid(2, 0, rows(&[(2, vec![text(0, "Name")])]));
    let candidates = header_candidates(&only_row_two, 3);
    assert!(matches!(
        columns_for_header(&only_row_two, 1, &candidates, None),
        Err(ScanError::HeaderNotCandidate(1))
    ));

    let titled = grid(
        7,
        2,
        rows(&[
            (2, vec![text(0, "Quarter report")]),
            (4, vec![text(0, "Name"), text(1, "City"), text(2, "Amount")]),
            (5, vec![text(0, "Ann"), text(1, "Oslo"), text(2, "2")]),
            (7, vec![text(0, "Bo"), text(2, "3")]),
        ]),
    );
    let columns = columns_for_header(&titled, 4, &header_candidates(&titled, 3), None).unwrap();
    let data = data_rows(&titled, 4, &columns, None);
    assert_eq!(
        data.iter().map(|row| row.excel_row).collect::<Vec<_>>(),
        vec![5, 7]
    );
    assert_eq!(
        data[0]
            .cells
            .iter()
            .map(|cell| cell.display.as_str())
            .collect::<Vec<_>>(),
        vec!["Ann", "Oslo", "2"]
    );
    assert_eq!(
        data[1]
            .cells
            .iter()
            .map(|cell| cell.col)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(
        data[1]
            .cells
            .iter()
            .map(|cell| cell.display.as_str())
            .collect::<Vec<_>>(),
        vec!["Bo", "3"]
    );

    let title_not_data = grid(
        5,
        2,
        rows(&[
            (2, vec![text(0, "Quarter report")]),
            (4, vec![text(0, "Name"), text(1, "City"), text(2, "Amount")]),
            (5, vec![text(0, "Ann"), text(1, "Oslo"), text(2, "2")]),
        ]),
    );
    let columns = columns_for_header(
        &title_not_data,
        4,
        &header_candidates(&title_not_data, 3),
        None,
    )
    .unwrap();
    assert_eq!(
        data_rows(&title_not_data, 4, &columns, None)
            .iter()
            .map(|row| row.excel_row)
            .collect::<Vec<_>>(),
        vec![5]
    );

    let gap = grid(
        53,
        0,
        rows(&[
            (1, vec![text(0, "Name")]),
            (2, vec![text(0, "Keep")]),
            (53, vec![text(0, "Drop")]),
        ]),
    );
    let columns = columns_for_header(&gap, 1, &header_candidates(&gap, 1), None).unwrap();
    assert_eq!(
        data_rows(&gap, 1, &columns, None)
            .iter()
            .map(|row| row.excel_row)
            .collect::<Vec<_>>(),
        vec![2]
    );

    let gap_49 = grid(
        52,
        0,
        rows(&[
            (1, vec![text(0, "Name")]),
            (2, vec![text(0, "Keep")]),
            (52, vec![text(0, "Keep")]),
        ]),
    );
    let columns = columns_for_header(&gap_49, 1, &header_candidates(&gap_49, 1), None).unwrap();
    assert_eq!(
        data_rows(&gap_49, 1, &columns, None)
            .iter()
            .map(|row| row.excel_row)
            .collect::<Vec<_>>(),
        vec![2, 52]
    );

    let stray = grid(
        2,
        3,
        rows(&[
            (1, vec![text(0, "Name"), text(1, "City")]),
            (2, vec![text(3, "note")]),
        ]),
    );
    let columns = columns_for_header(&stray, 1, &header_candidates(&stray, 1), None).unwrap();
    assert!(data_rows(&stray, 1, &columns, None).is_empty());

    let grouped = grid(
        4,
        3,
        rows(&[
            (1, vec![text(0, "Group"), text(3, "Next")]),
            (2, vec![text(0, "One"), text(1, "Two"), text(2, "Other")]),
            (4, vec![text(0, "a"), text(1, "b"), text(3, "n")]),
        ]),
    );
    let columns =
        columns_for_header(&grouped, 1, &header_candidates(&grouped, 2), Some(2)).unwrap();
    assert_eq!(
        (columns[0].excel_id.as_str(), columns[0].name.as_str()),
        ("A", "Group")
    );
    assert_eq!(
        columns[0]
            .subcolumns
            .iter()
            .map(|column| (column.excel_id.as_str(), column.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("A", "One"), ("B", "Two"), ("C", "Other")]
    );
    assert!(columns[1].subcolumns.is_empty());
    assert_eq!(
        (columns[1].excel_id.as_str(), columns[1].name.as_str()),
        ("D", "Next")
    );
    let data = data_rows(&grouped, 1, &columns, Some(2));
    assert_eq!(
        data.iter().map(|row| row.excel_row).collect::<Vec<_>>(),
        vec![4]
    );
    assert_eq!(
        data[0]
            .cells
            .iter()
            .map(|cell| cell.display.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "n"]
    );

    let shared = grid(
        2,
        3,
        rows(&[
            (1, vec![text(0, "Left"), text(2, "Right")]),
            (
                2,
                vec![
                    text(0, "Other"),
                    text(1, "Own"),
                    text(2, "Other"),
                    text(3, "Own"),
                ],
            ),
        ]),
    );
    let columns = columns_for_header(&shared, 1, &header_candidates(&shared, 1), Some(2)).unwrap();
    assert_eq!(
        columns[0]
            .subcolumns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Other", "Own"]
    );
    assert_eq!(
        columns[1]
            .subcolumns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Other", "Own"]
    );

    let duplicate_sub = grid(
        2,
        2,
        rows(&[
            (1, vec![text(0, "Group"), text(2, "Next")]),
            (2, vec![text(0, "Other"), text(1, "Other")]),
        ]),
    );
    assert!(matches!(
        columns_for_header(
            &duplicate_sub,
            1,
            &header_candidates(&duplicate_sub, 1),
            Some(2)
        ),
        Err(ScanError::DuplicateColumn(_))
    ));

    let named_sheet = grid(
        2,
        2,
        rows(&[
            (
                1,
                vec![
                    text(0, "Абрамов"),
                    text(1, "Александр"),
                    text(2, "Авангард"),
                ],
            ),
            (
                2,
                vec![text(0, "Авхадыев"), text(1, "Антон"), text(2, "СёрчИнформ")],
            ),
        ]),
    );
    let columns =
        columns_without_header(&[("A", "surname"), ("b", " name "), ("C", "company name")])
            .unwrap();
    assert_eq!(
        names(&columns),
        vec![
            ("A".into(), "surname".into()),
            ("B".into(), "name".into()),
            ("C".into(), "company name".into()),
        ]
    );
    let data = rows_without_header(&named_sheet, &columns);
    assert_eq!(
        data.iter().map(|row| row.excel_row).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(data[0].cells[0].display, "Абрамов");
    assert_eq!(data[1].cells[2].display, "СёрчИнформ");

    let used = Grid {
        first_row: 3,
        last_row: 4,
        first_col: 0,
        last_col: 0,
        cells: rows(&[(3, vec![text(0, "Ann")]), (4, vec![text(0, "Bo")])]),
    };
    let columns = columns_without_header(&[("A", "name")]).unwrap();
    assert_eq!(
        rows_without_header(&used, &columns)
            .iter()
            .map(|row| row.excel_row)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );

    let unselected = grid(1, 1, rows(&[(1, vec![text(0, "Ann"), text(1, "Oslo")])]));
    let columns = columns_without_header(&[("A", "name")]).unwrap();
    assert_eq!(
        rows_without_header(&unselected, &columns)[0]
            .cells
            .iter()
            .map(|cell| cell.col)
            .collect::<Vec<_>>(),
        vec![0]
    );

    assert!(matches!(
        columns_without_header(&[]),
        Err(ScanError::BadColumn(_))
    ));
    assert!(matches!(
        columns_without_header(&[("A1", "name")]),
        Err(ScanError::BadColumn(_))
    ));
    assert!(matches!(
        columns_without_header(&[("A", "  ")]),
        Err(ScanError::BadColumn(_))
    ));
    assert!(matches!(
        columns_without_header(&[("A", "name"), ("B", "name")]),
        Err(ScanError::DuplicateColumn(_))
    ));
    assert!(matches!(
        columns_without_header(&[("A", "surname"), ("a", "name")]),
        Err(ScanError::DuplicateColumn(_))
    ));

    let window = grid(
        6,
        0,
        rows(&[
            (1, vec![text(0, "Name")]),
            (5, vec![text(0, "Under")]),
            (6, vec![text(0, "Value")]),
        ]),
    );
    let candidates = header_candidates(&window, 3);
    let columns = columns_for_header(&window, 1, &candidates, Some(5)).unwrap();
    assert_eq!(columns[0].subcolumns[0].name, "Under");
    assert_eq!(
        data_rows(&window, 1, &columns, Some(5))
            .iter()
            .map(|row| row.excel_row)
            .collect::<Vec<_>>(),
        vec![6]
    );
    assert!(matches!(
        columns_for_header(&window, 1, &candidates, Some(6)),
        Err(ScanError::SubcolumnRowOutOfRange(6))
    ));
}

#[test]
fn grid_cell_values() {
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("text");
            reading.display = "0012".into();
            reading
        }),
        Some(CellValue::Text("0012".into()))
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("number");
            reading.serial = 2.0;
            reading
        }),
        Some(CellValue::Int(2))
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("number");
            reading.serial = 2.5;
            reading
        }),
        Some(CellValue::Float(2.5))
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("number");
            reading.serial = 3.0 + 1e-12;
            reading
        }),
        Some(CellValue::Int(3))
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("number");
            reading.serial = 0.5;
            reading.is_percent = true;
            reading
        }),
        Some(CellValue::Float(0.5))
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("number");
            reading.serial = 1.5;
            reading.is_currency = true;
            reading
        }),
        Some(CellValue::Float(1.5))
    );

    let date = classify(&{
        let mut reading = Reading::new("number");
        reading.serial = SERIAL_1900;
        reading.is_date = true;
        reading
    });
    assert_eq!(date, Some(CellValue::Date(DateValue::day(2024, 3, 15))));

    let date_time = classify(&{
        let mut reading = Reading::new("number");
        reading.serial = SERIAL_1900_1345;
        reading.is_date = true;
        reading.is_time = true;
        reading
    });
    assert_eq!(
        date_time,
        Some(CellValue::Date(DateValue {
            year: 2024,
            month: 3,
            day: 15,
            hour: 13,
            minute: 45,
            second: 0,
            has_time: true,
        }))
    );

    let time_only = classify(&{
        let mut reading = Reading::new("number");
        reading.serial = SERIAL_1900_1345;
        reading.is_time = true;
        reading
    });
    assert_eq!(
        time_only,
        Some(CellValue::Date(DateValue {
            year: 2024,
            month: 3,
            day: 15,
            hour: 13,
            minute: 45,
            second: 0,
            has_time: true,
        }))
    );

    let epoch_1904 = classify(&{
        let mut reading = Reading::new("number");
        reading.serial = SERIAL_1904;
        reading.is_date = true;
        reading.null_year = 1904;
        reading.null_month = 1;
        reading.null_day = 1;
        reading
    });
    assert_eq!(
        epoch_1904,
        Some(CellValue::Date(DateValue::day(2024, 3, 15)))
    );

    assert_eq!(
        classify(&{
            let mut reading = Reading::new("number");
            reading.serial = 1.0;
            reading.is_logical = true;
            reading
        }),
        Some(CellValue::Bool(true))
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("number");
            reading.serial = 0.0;
            reading.is_logical = true;
            reading
        }),
        Some(CellValue::Bool(false))
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("formula");
            reading.display = "4".into();
            reading.serial = 4.0;
            reading
        }),
        Some(CellValue::Int(4))
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("formula");
            reading.display = String::new();
            reading.serial = 0.0;
            reading.error_code = 0;
            reading
        }),
        None
    );
    assert_eq!(
        classify(&{
            let mut reading = Reading::new("error");
            reading.display = "#N/A".into();
            reading
        }),
        Some(CellValue::Text("#N/A".into()))
    );

    let date = classify(&{
        let mut reading = Reading::new("number");
        reading.serial = SERIAL_1900;
        reading.is_date = true;
        reading
    })
    .unwrap();
    assert!(matches!(date, CellValue::Date(value) if value == DateValue::day(2024, 3, 15)));
}
