//! A recorded `spreadsheets.get` payload, read through the same source calls as Calc.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::model::{CellValue, Grid, GridCell};
use crate::scan::{Bounds, GridSource};
use crate::values::{classify, Reading};

/// Cells taken from one trimmed Sheets response. `release` drops the last window, not the payload.
pub struct SheetsSource {
    cells: BTreeMap<u32, Vec<GridCell>>,
    bounds: Bounds,
    held: Option<BTreeMap<u32, Vec<GridCell>>>,
}

impl SheetsSource {
    pub fn from_payload(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let spreadsheet: Spreadsheet = serde_json::from_slice(bytes)?;
        let mut cells: BTreeMap<u32, Vec<GridCell>> = BTreeMap::new();
        for sheet in spreadsheet.sheets {
            for grid in sheet.data {
                for (offset, row) in grid.row_data.into_iter().enumerate() {
                    let excel_row = grid.start_row + offset as u32 + 1;
                    let mut row_cells = Vec::new();
                    for (col_offset, cell) in row.values.into_iter().enumerate() {
                        let col = grid.start_column + col_offset as u32;
                        if let Some(grid_cell) = cell_from_payload(col, cell) {
                            row_cells.push(grid_cell);
                        }
                    }
                    if !row_cells.is_empty() {
                        cells.entry(excel_row).or_default().extend(row_cells);
                    }
                }
            }
        }
        for row in cells.values_mut() {
            row.sort_by_key(|cell| cell.col);
        }
        let bounds = bounds_of(&cells);
        Ok(Self {
            cells,
            bounds,
            held: None,
        })
    }

    pub fn open(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let bytes = std::fs::read(path)?;
        Ok(Self::from_payload(&bytes)?)
    }

    /// The used range as a grid the step 1 rules can scan.
    pub fn grid(&self) -> Grid {
        Grid {
            first_row: self.bounds.first_row,
            last_row: self.bounds.last_row,
            first_col: self.bounds.first_col,
            last_col: self.bounds.last_col,
            cells: self.cells.clone(),
        }
    }
}

impl GridSource for SheetsSource {
    fn bounds(&self) -> Bounds {
        self.bounds
    }

    fn has_values(&self) -> bool {
        !self.cells.is_empty()
    }

    fn nonempty_rows(&self, k: usize) -> Vec<u32> {
        if k == 0 {
            return Vec::new();
        }
        self.cells.keys().copied().take(k).collect()
    }

    fn read(
        &mut self,
        first: u32,
        last: u32,
        columns: Option<&[u32]>,
    ) -> BTreeMap<u32, Vec<GridCell>> {
        let mut window = BTreeMap::new();
        for (&excel_row, cells) in &self.cells {
            if excel_row < first || excel_row > last {
                continue;
            }
            let kept: Vec<GridCell> = cells
                .iter()
                .filter(|cell| columns.is_none_or(|wanted| wanted.contains(&cell.col)))
                .cloned()
                .collect();
            if !kept.is_empty() {
                window.insert(excel_row, kept);
            }
        }
        self.held = Some(window.clone());
        window
    }

    fn release(&mut self) {
        self.held = None;
    }
}

fn bounds_of(cells: &BTreeMap<u32, Vec<GridCell>>) -> Bounds {
    let Some((&first_row, _)) = cells.first_key_value() else {
        return Bounds {
            first_row: 1,
            last_row: 1,
            first_col: 0,
            last_col: 0,
        };
    };
    let last_row = cells
        .last_key_value()
        .map(|(&row, _)| row)
        .unwrap_or(first_row);
    let mut first_col = u32::MAX;
    let mut last_col = 0u32;
    for row in cells.values() {
        for cell in row {
            first_col = first_col.min(cell.col);
            last_col = last_col.max(cell.col);
        }
    }
    Bounds {
        first_row,
        last_row,
        first_col,
        last_col,
    }
}

fn cell_from_payload(col: u32, cell: CellData) -> Option<GridCell> {
    let reading = reading_from_cell(&cell)?;
    let value = classify(&reading)?;
    let display = cell
        .formatted_value
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| display_of(&value));
    Some(GridCell {
        col,
        display,
        value: Some(value),
    })
}

fn display_of(value: &CellValue) -> String {
    match value {
        CellValue::Text(text) => text.clone(),
        CellValue::Int(value) => value.to_string(),
        CellValue::Float(value) => value.to_string(),
        CellValue::Bool(value) => value.to_string(),
        CellValue::Date(date) => format!("{:04}-{:02}-{:02}", date.year, date.month, date.day),
    }
}

fn reading_from_cell(cell: &CellData) -> Option<Reading> {
    let effective = cell.effective_value.as_ref();
    if let Some(error) = effective.and_then(|value| value.error_value.as_ref()) {
        let mut reading = Reading::new("error");
        reading.display = cell
            .formatted_value
            .clone()
            .unwrap_or_else(|| error.type_name.clone());
        return Some(reading);
    }
    if let Some(text) = effective.and_then(|value| value.string_value.clone()) {
        let mut reading = Reading::new("text");
        reading.display = cell.formatted_value.clone().unwrap_or(text);
        return Some(reading);
    }
    if let Some(flag) = effective.and_then(|value| value.bool_value) {
        let mut reading = Reading::new("number");
        reading.is_logical = true;
        reading.serial = if flag { 1.0 } else { 0.0 };
        reading.display = cell.formatted_value.clone().unwrap_or_default();
        return Some(reading);
    }
    if let Some(serial) = effective.and_then(|value| value.number_value) {
        let mut reading = Reading::new("number");
        reading.serial = serial;
        reading.display = cell.formatted_value.clone().unwrap_or_default();
        apply_number_format(&mut reading, cell);
        return Some(reading);
    }
    let formula = cell
        .user_entered_value
        .as_ref()
        .and_then(|value| value.formula_value.as_ref());
    if formula.is_some() {
        let mut reading = Reading::new("formula");
        reading.display = cell.formatted_value.clone().unwrap_or_default();
        return Some(reading);
    }
    None
}

fn apply_number_format(reading: &mut Reading, cell: &CellData) {
    let Some(kind) = cell
        .effective_format
        .as_ref()
        .and_then(|format| format.number_format.as_ref())
        .map(|format| format.type_name.as_str())
    else {
        return;
    };
    match kind {
        "DATE" => reading.is_date = true,
        "TIME" => reading.is_time = true,
        "DATE_TIME" => {
            reading.is_date = true;
            reading.is_time = true;
        }
        "PERCENT" => reading.is_percent = true,
        "CURRENCY" | "SCIENTIFIC" => reading.is_currency = true,
        _ => {}
    }
}

#[derive(Debug, Deserialize)]
struct Spreadsheet {
    #[serde(default)]
    sheets: Vec<Sheet>,
}

#[derive(Debug, Deserialize)]
struct Sheet {
    #[serde(default)]
    properties: SheetProperties,
    #[serde(default)]
    data: Vec<GridData>,
}

#[derive(Debug, Default, Deserialize)]
struct SheetProperties {
    #[serde(default, rename = "sheetId")]
    sheet_id: i32,
    #[serde(default)]
    title: String,
}

pub struct LoadedSheet {
    pub title: String,
    pub sheet_id: i32,
    pub grid: Grid,
}

pub fn load_sheets(bytes: &[u8]) -> Result<Vec<LoadedSheet>, serde_json::Error> {
    let spreadsheet: Spreadsheet = serde_json::from_slice(bytes)?;
    let mut loaded = Vec::new();
    for sheet in spreadsheet.sheets {
        let mut cells: BTreeMap<u32, Vec<GridCell>> = BTreeMap::new();
        for grid in sheet.data {
            for (offset, row) in grid.row_data.into_iter().enumerate() {
                let excel_row = grid.start_row + offset as u32 + 1;
                let mut row_cells = Vec::new();
                for (col_offset, cell) in row.values.into_iter().enumerate() {
                    let col = grid.start_column + col_offset as u32;
                    if let Some(grid_cell) = cell_from_payload(col, cell) {
                        row_cells.push(grid_cell);
                    }
                }
                if !row_cells.is_empty() {
                    cells.entry(excel_row).or_default().extend(row_cells);
                }
            }
        }
        for row in cells.values_mut() {
            row.sort_by_key(|cell| cell.col);
        }
        let bounds = bounds_of(&cells);
        loaded.push(LoadedSheet {
            title: sheet.properties.title,
            sheet_id: sheet.properties.sheet_id,
            grid: Grid {
                first_row: bounds.first_row,
                last_row: bounds.last_row,
                first_col: bounds.first_col,
                last_col: bounds.last_col,
                cells,
            },
        });
    }
    Ok(loaded)
}

#[derive(Debug, Deserialize)]
struct GridData {
    #[serde(default, rename = "startRow")]
    start_row: u32,
    #[serde(default, rename = "startColumn")]
    start_column: u32,
    #[serde(default, rename = "rowData")]
    row_data: Vec<RowData>,
}

#[derive(Debug, Deserialize)]
struct RowData {
    #[serde(default)]
    values: Vec<CellData>,
}

#[derive(Debug, Deserialize)]
struct CellData {
    #[serde(default, rename = "formattedValue")]
    formatted_value: Option<String>,
    #[serde(default, rename = "effectiveValue")]
    effective_value: Option<EffectiveValue>,
    #[serde(default, rename = "effectiveFormat")]
    effective_format: Option<EffectiveFormat>,
    #[serde(default, rename = "userEnteredValue")]
    user_entered_value: Option<UserEnteredValue>,
}

#[derive(Debug, Deserialize)]
struct EffectiveValue {
    #[serde(default, rename = "numberValue")]
    number_value: Option<f64>,
    #[serde(default, rename = "stringValue")]
    string_value: Option<String>,
    #[serde(default, rename = "boolValue")]
    bool_value: Option<bool>,
    #[serde(default, rename = "errorValue")]
    error_value: Option<ErrorValue>,
}

#[derive(Debug, Deserialize)]
struct ErrorValue {
    #[serde(default, rename = "type")]
    type_name: String,
}

#[derive(Debug, Deserialize)]
struct EffectiveFormat {
    #[serde(default, rename = "numberFormat")]
    number_format: Option<NumberFormat>,
}

#[derive(Debug, Deserialize)]
struct NumberFormat {
    #[serde(default, rename = "type")]
    type_name: String,
}

#[derive(Debug, Deserialize)]
struct UserEnteredValue {
    #[serde(default, rename = "formulaValue")]
    formula_value: Option<String>,
}
