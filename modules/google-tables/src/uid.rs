//! Row ids, the fingerprint formula, and the content hash.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};

use blake2::digest::{Update, VariableOutput};
use blake2::Blake2bVar;

use crate::columns::parse_excel_id;
use crate::model::CellValue;

pub const UID_ALPHABET: &str = "0123456789ABCDEFGHIKLMNOPQRSTVXYZ";
pub const UID_LEN: usize = 16;

const DRAW_ATTEMPTS: usize = 64;

#[derive(Debug)]
pub enum UidError {
    Exhausted,
    Io(io::Error),
}

impl std::fmt::Display for UidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UidError::Exhausted => write!(f, "could not draw a uid that is not already taken"),
            UidError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for UidError {}

pub fn is_uid(text: &str) -> bool {
    text.len() == UID_LEN
        && text
            .bytes()
            .all(|byte| UID_ALPHABET.as_bytes().contains(&byte))
}

/// Draw a 16-character id. A result already in `taken` is discarded and drawn again.
pub fn draw_uid(taken: &HashSet<String>) -> Result<String, UidError> {
    let mut bytes = [0u8; UID_LEN * DRAW_ATTEMPTS];
    let mut file = File::open("/dev/urandom").map_err(UidError::Io)?;
    file.read_exact(&mut bytes).map_err(UidError::Io)?;
    let mut index = 0;
    draw_uid_with(taken, || {
        let byte = bytes[index];
        index += 1;
        byte
    })
}

pub fn draw_uid_with(
    taken: &HashSet<String>,
    mut next_byte: impl FnMut() -> u8,
) -> Result<String, UidError> {
    let alphabet = UID_ALPHABET.as_bytes();
    for _ in 0..DRAW_ATTEMPTS {
        let mut id = String::with_capacity(UID_LEN);
        for _ in 0..UID_LEN {
            let index = usize::from(next_byte()) % alphabet.len();
            id.push(alphabet[index] as char);
        }
        if !taken.contains(&id) {
            return Ok(id);
        }
    }
    Err(UidError::Exhausted)
}

pub struct FormulaColumn<'a> {
    pub excel_id: &'a str,
    pub name: &'a str,
    pub exclude_from_hash: bool,
}

/// `ARRAYFORMULA` that joins the mapped columns the fingerprint keeps, left to right.
///
/// The open range starts at `first_data_row`. An excluded column's letter is absent.
pub fn fingerprint_formula(columns: &[FormulaColumn<'_>], first_data_row: u32) -> String {
    let mut included: Vec<&FormulaColumn<'_>> = columns
        .iter()
        .filter(|column| !column.exclude_from_hash)
        .filter(|column| parse_excel_id(column.excel_id).is_some())
        .collect();
    included.sort_by_key(|column| parse_excel_id(column.excel_id).unwrap_or(u32::MAX));
    if included.is_empty() {
        return "=ARRAYFORMULA(\"\")".into();
    }
    let refs: Vec<String> = included
        .iter()
        .map(|column| {
            let letter = column.excel_id.trim().to_ascii_uppercase();
            format!("{letter}{first_data_row}:{letter}")
        })
        .collect();
    let length = refs.join("&");
    let joined = refs.join("&\"|\"&");
    format!("=ARRAYFORMULA(IF(LEN({length})=0,\"\",{joined}))")
}

/// Stamp stored on `irbis.fingerprint` when that formula is written.
///
/// The cycle compares this, and that the formula cell is not empty. It does not
/// compare the formula text Sheets returns.
pub fn formula_stamp(formula: &str) -> String {
    let mut hasher = Blake2bVar::new(16).expect("16-byte blake2b");
    hasher.update(formula.as_bytes());
    let mut out = [0u8; 16];
    hasher.finalize_variable(&mut out).expect("16-byte blake2b");
    out.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub struct HashCell<'a> {
    pub name: &'a str,
    pub value: &'a CellValue,
    pub exclude_from_hash: bool,
}

/// 16-byte BLAKE2b. The sheet row is not an input. An excluded cell is not an input.
///
/// Layout, after the format byte `1`: each kept cell in name order as a big-endian name
/// length, the name bytes, a kind byte, then the value. Kind `1` text, `2` float, `3` int,
/// `4` date, `5` bool. Numbers are little-endian. A date is year, month, day, hour, minute,
/// second, and `has_time`.
pub fn content_hash(cells: &[HashCell<'_>]) -> [u8; 16] {
    let mut kept: Vec<&HashCell<'_>> = cells
        .iter()
        .filter(|cell| !cell.exclude_from_hash)
        .collect();
    kept.sort_by(|left, right| left.name.cmp(right.name));
    let mut bytes = Vec::new();
    bytes.push(1);
    for cell in kept {
        let name = cell.name.as_bytes();
        bytes.extend_from_slice(&(name.len() as u32).to_be_bytes());
        bytes.extend_from_slice(name);
        match cell.value {
            CellValue::Text(text) => {
                bytes.push(1);
                let raw = text.as_bytes();
                bytes.extend_from_slice(&(raw.len() as u32).to_be_bytes());
                bytes.extend_from_slice(raw);
            }
            CellValue::Float(value) => {
                bytes.push(2);
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            CellValue::Int(value) => {
                bytes.push(3);
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            CellValue::Date(date) => {
                bytes.push(4);
                bytes.extend_from_slice(&date.year.to_be_bytes());
                bytes.push(date.month as u8);
                bytes.push(date.day as u8);
                bytes.push(date.hour as u8);
                bytes.push(date.minute as u8);
                bytes.push(date.second as u8);
                bytes.push(u8::from(date.has_time));
            }
            CellValue::Bool(value) => {
                bytes.push(5);
                bytes.push(u8::from(*value));
            }
        }
    }
    let mut hasher = Blake2bVar::new(16).expect("16 is a valid BLAKE2b length");
    hasher.update(&bytes);
    let mut out = [0u8; 16];
    hasher
        .finalize_variable(&mut out)
        .expect("output length matches");
    out
}
