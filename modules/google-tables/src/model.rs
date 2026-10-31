/// Internal table. Protobuf conversion happens at the gRPC boundary.

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub index: u32,
    pub excel_id: String,
    pub name: String,
    pub subcolumns: Vec<Column>,
}

impl Column {
    pub fn new(index: u32, excel_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            index,
            excel_id: excel_id.into(),
            name: name.into(),
            subcolumns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateValue {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub has_time: bool,
}

impl DateValue {
    pub fn day(year: i32, month: u32, day: u32) -> Self {
        Self {
            year,
            month,
            day,
            hour: 0,
            minute: 0,
            second: 0,
            has_time: false,
        }
    }
}

/// One of text, float, int, date, or bool.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Text(String),
    Float(f64),
    Int(i64),
    Date(DateValue),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridCell {
    pub col: u32,
    pub display: String,
    pub value: Option<CellValue>,
}

impl GridCell {
    pub fn text(col: u32, display: impl Into<String>) -> Self {
        let display = display.into();
        Self {
            value: Some(CellValue::Text(display.clone())),
            col,
            display,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub first_row: u32,
    pub last_row: u32,
    pub first_col: u32,
    pub last_col: u32,
    pub cells: std::collections::BTreeMap<u32, Vec<GridCell>>,
}

impl Grid {
    pub fn new(
        last_row: u32,
        last_col: u32,
        cells: std::collections::BTreeMap<u32, Vec<GridCell>>,
    ) -> Self {
        Self {
            first_row: 1,
            last_row,
            first_col: 0,
            last_col,
            cells,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub excel_row: u32,
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataRow {
    pub excel_row: u32,
    pub cells: Vec<GridCell>,
}
