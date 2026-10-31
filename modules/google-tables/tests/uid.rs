use std::collections::HashSet;

use google_tables::model::CellValue;
use google_tables::uid::{
    content_hash, draw_uid, draw_uid_with, fingerprint_formula, FormulaColumn, HashCell,
    UID_ALPHABET, UID_LEN,
};

fn cell<'a>(name: &'a str, value: &'a CellValue, exclude_from_hash: bool) -> HashCell<'a> {
    HashCell {
        name,
        value,
        exclude_from_hash,
    }
}

#[test]
fn uid_shape() {
    let mut taken = HashSet::new();
    for _ in 0..100 {
        let id = draw_uid(&taken).unwrap();
        assert_eq!(id.len(), UID_LEN);
        assert!(id
            .bytes()
            .all(|byte| UID_ALPHABET.as_bytes().contains(&byte)));
        assert!(taken.insert(id));
    }

    let mut taken = HashSet::new();
    taken.insert("0".repeat(UID_LEN));
    let mut drawn = 0u32;
    let id = draw_uid_with(&taken, || {
        let byte = if drawn < UID_LEN as u32 { 0 } else { 1 };
        drawn += 1;
        byte
    })
    .unwrap();
    assert!(drawn > UID_LEN as u32);
    assert_eq!(id, "1".repeat(UID_LEN));
    assert!(!taken.contains(&id));
}

#[test]
fn uid_formula_skips_now() {
    let formula = fingerprint_formula(
        &[
            FormulaColumn {
                excel_id: "A",
                name: "Name",
                exclude_from_hash: false,
            },
            FormulaColumn {
                excel_id: "C",
                name: "Amount",
                exclude_from_hash: true,
            },
            FormulaColumn {
                excel_id: "B",
                name: "City",
                exclude_from_hash: false,
            },
        ],
        2,
    );
    assert!(formula.starts_with("=ARRAYFORMULA("));
    assert!(formula.contains("A2:A&\"|\"&B2:B"));
    assert!(!formula.contains('C'));
}

#[test]
fn uid_hash_ignores_move() {
    let ann = CellValue::Text("Ann".into());
    let oslo = CellValue::Text("Oslo".into());
    let riga = CellValue::Text("Riga".into());
    let amount = CellValue::Int(2);
    let clock = CellValue::Text("10:00".into());
    let later = CellValue::Text("10:01".into());

    let at_row_5 = [
        cell("Name", &ann, false),
        cell("City", &oslo, false),
        cell("Amount", &amount, true),
    ];
    let at_row_9 = [
        cell("City", &oslo, false),
        cell("Name", &ann, false),
        cell("Amount", &amount, true),
    ];
    assert_eq!(content_hash(&at_row_5), content_hash(&at_row_9));

    let clock_moved = [
        cell("Name", &ann, false),
        cell("City", &oslo, false),
        cell("Amount", &later, true),
        cell("Clock", &clock, true),
    ];
    assert_eq!(content_hash(&at_row_5), content_hash(&clock_moved));

    let city_changed = [
        cell("Name", &ann, false),
        cell("City", &riga, false),
        cell("Amount", &amount, true),
    ];
    assert_ne!(content_hash(&at_row_5), content_hash(&city_changed));
}
