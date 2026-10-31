//! The Sheets calls the cycle and the setup RPCs share. Tests use [`FakeBook`].

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::columns::parse_excel_id;
use crate::model::{CellValue, Grid, GridCell};
use crate::sheets::load_sheets;

pub const KEY_UID: &str = "irbis.uid";
pub const KEY_FINGERPRINT: &str = "irbis.fingerprint";
pub const KEY_COLUMN: &str = "irbis.column";
pub const KEY_EXCLUDE: &str = "irbis.exclude_hash";

#[derive(Debug)]
pub enum GoogleError {
    RateLimited,
    Message(String),
}

impl std::fmt::Display for GoogleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoogleError::RateLimited => write!(f, "429"),
            GoogleError::Message(text) => write!(f, "{text}"),
        }
    }
}

impl std::error::Error for GoogleError {}

/// `min(2^n, 60)` seconds plus jitter. `n` is the number of failures since the last success.
pub fn backoff_delay(failures: u32, jitter: Duration) -> Duration {
    let secs = 2u64.saturating_pow(failures).min(60);
    Duration::from_secs(secs) + jitter
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColRange {
    pub start_row: u32,
    /// `None` is an open range, such as `D3:D`, through the last cell in the column.
    pub end_row: Option<u32>,
    pub start_col: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub key: String,
    pub column: u32,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BookCell {
    pub value: Option<CellValue>,
    pub formula: Option<String>,
}

impl BookCell {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            value: Some(CellValue::Text(text.into())),
            formula: None,
        }
    }

    pub fn value(value: CellValue) -> Self {
        Self {
            value: Some(value),
            formula: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.formula.is_none() && self.value.is_none()
    }
}

/// One column, from `start_row`, with one value per row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueRun {
    pub start_row: u32,
    pub col: u32,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawWrite {
    pub cells: Vec<(u32, u32, String)>,
    pub runs: Vec<ValueRun>,
}

/// Group cells of one column into contiguous row runs.
pub fn value_runs(cells: &[(u32, u32, String)]) -> Vec<ValueRun> {
    let mut by_col: BTreeMap<u32, Vec<(u32, String)>> = BTreeMap::new();
    for (row, col, text) in cells {
        by_col.entry(*col).or_default().push((*row, text.clone()));
    }
    let mut runs = Vec::new();
    for (col, mut entries) in by_col {
        entries.sort_by_key(|(row, _)| *row);
        let mut rows = entries.into_iter();
        let Some((mut start, first)) = rows.next() else {
            continue;
        };
        let mut end = start;
        let mut values = vec![first];
        for (row, text) in rows {
            if row == end + 1 {
                end = row;
                values.push(text);
            } else {
                runs.push(ValueRun {
                    start_row: start,
                    col,
                    values,
                });
                start = row;
                end = row;
                values = vec![text];
            }
        }
        runs.push(ValueRun {
            start_row: start,
            col,
            values,
        });
    }
    runs
}

#[derive(Debug, Clone)]
pub struct SheetTab {
    pub title: String,
    pub sheet_id: i32,
    pub cells: BTreeMap<(u32, u32), BookCell>,
    pub metadata: Vec<Meta>,
    pub hidden: HashSet<u32>,
    pub warning_only: HashSet<u32>,
}

#[derive(Debug, Clone)]
pub struct FakeBook {
    pub spreadsheet_id: String,
    pub version: u64,
    pub version_queue: VecDeque<String>,
    pub tabs: Vec<SheetTab>,
    pub gets: Vec<Vec<ColRange>>,
    pub raw_writes: Vec<RawWrite>,
    pub formulas: Vec<String>,
    /// `true` makes the next call return 429. A `false` entry is a success that consumes the slot.
    pub rate_limit: VecDeque<bool>,
    /// Calls admitted by the shared limiter. A refused call does not increment this.
    pub tokens: u32,
    /// `batch_get` returns formula text the way Sheets normalizes it, which is not the text written.
    pub normalize_formulas: bool,
}

impl FakeBook {
    pub fn new(spreadsheet_id: impl Into<String>) -> Self {
        Self {
            spreadsheet_id: spreadsheet_id.into(),
            version: 1,
            version_queue: VecDeque::new(),
            tabs: Vec::new(),
            gets: Vec::new(),
            raw_writes: Vec::new(),
            formulas: Vec::new(),
            rate_limit: VecDeque::new(),
            tokens: 0,
            normalize_formulas: false,
        }
    }

    pub fn from_fixture(
        spreadsheet_id: impl Into<String>,
        bytes: &[u8],
    ) -> Result<Self, serde_json::Error> {
        let loaded = load_sheets(bytes)?;
        let mut book = Self::new(spreadsheet_id);
        for sheet in loaded {
            book.tabs.push(SheetTab::from_grid(
                sheet.title,
                sheet.sheet_id,
                &sheet.grid,
            ));
        }
        Ok(book)
    }

    pub fn tab(&self, sheet_id: i32) -> Option<&SheetTab> {
        self.tabs.iter().find(|tab| tab.sheet_id == sheet_id)
    }

    pub fn tab_mut(&mut self, sheet_id: i32) -> Option<&mut SheetTab> {
        self.tabs.iter_mut().find(|tab| tab.sheet_id == sheet_id)
    }

    fn bump(&mut self) {
        self.version += 1;
    }

    fn limited(&mut self) -> Result<(), GoogleError> {
        if self.rate_limit.pop_front().unwrap_or(false) {
            Err(GoogleError::RateLimited)
        } else {
            Ok(())
        }
    }
}

impl SheetTab {
    pub fn from_grid(title: impl Into<String>, sheet_id: i32, grid: &Grid) -> Self {
        let mut cells = BTreeMap::new();
        for (row, row_cells) in &grid.cells {
            for cell in row_cells {
                cells.insert((*row, cell.col), book_cell(cell));
            }
        }
        Self {
            title: title.into(),
            sheet_id,
            cells,
            metadata: Vec::new(),
            hidden: HashSet::new(),
            warning_only: HashSet::new(),
        }
    }

    pub fn grid(&self) -> Grid {
        let mut cells: BTreeMap<u32, Vec<GridCell>> = BTreeMap::new();
        let mut first_col = u32::MAX;
        let mut last_col = 0u32;
        let mut first_row = u32::MAX;
        let mut last_row = 1u32;
        for ((row, col), cell) in &self.cells {
            let Some(value) = cell.value.clone() else {
                continue;
            };
            let display = match &value {
                CellValue::Text(text) => text.clone(),
                CellValue::Int(number) => number.to_string(),
                CellValue::Float(number) => number.to_string(),
                CellValue::Bool(flag) => flag.to_string(),
                CellValue::Date(date) => {
                    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
                }
            };
            cells.entry(*row).or_default().push(GridCell {
                col: *col,
                display,
                value: Some(value),
            });
            first_col = first_col.min(*col);
            last_col = last_col.max(*col);
            first_row = first_row.min(*row);
            last_row = last_row.max(*row);
        }
        if cells.is_empty() {
            return Grid::new(1, 0, BTreeMap::new());
        }
        for row in cells.values_mut() {
            row.sort_by_key(|cell| cell.col);
        }
        Grid {
            first_row,
            last_row,
            first_col,
            last_col,
            cells,
        }
    }

    pub fn put_value(&mut self, row: u32, col: u32, value: CellValue) {
        self.cells.insert((row, col), BookCell::value(value));
    }

    pub fn put_text(&mut self, row: u32, col: u32, text: impl Into<String>) {
        self.cells.insert((row, col), BookCell::text(text));
    }

    pub fn put_formula(&mut self, row: u32, col: u32, formula: impl Into<String>) {
        self.cells.insert(
            (row, col),
            BookCell {
                value: None,
                formula: Some(formula.into()),
            },
        );
    }

    pub fn last_used_column(&self) -> Option<u32> {
        self.cells.keys().map(|(_, col)| *col).max()
    }

    pub fn last_nonempty(&self, col: u32) -> Option<u32> {
        self.cells
            .iter()
            .filter(|((row, cell_col), cell)| *cell_col == col && *row > 0 && !cell_is_blank(cell))
            .map(|((row, _), _)| *row)
            .max()
    }

    pub fn append_hidden(&mut self) -> u32 {
        let col = self
            .last_used_column()
            .into_iter()
            .chain(self.hidden.iter().copied())
            .chain(self.metadata.iter().map(|item| item.column))
            .max()
            .map(|col| col + 1)
            .unwrap_or(0);
        self.hidden.insert(col);
        col
    }
}

fn cell_is_blank(cell: &BookCell) -> bool {
    match &cell.value {
        Some(CellValue::Text(text)) if text.is_empty() => cell.formula.is_none(),
        None => cell.formula.is_none(),
        _ => false,
    }
}

fn book_cell(cell: &GridCell) -> BookCell {
    BookCell {
        value: cell.value.clone(),
        formula: None,
    }
}

pub type SheetsFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait GoogleSheets {
    fn version<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
    ) -> SheetsFuture<'a, Result<String, GoogleError>>;
    fn metadata<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
    ) -> SheetsFuture<'a, Result<Vec<Meta>, GoogleError>>;
    fn batch_get<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        ranges: &'a [ColRange],
    ) -> SheetsFuture<'a, Result<BTreeMap<(u32, u32), BookCell>, GoogleError>>;
    fn write_raw<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        runs: &'a [ValueRun],
    ) -> SheetsFuture<'a, Result<(), GoogleError>>;
    fn write_formula<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        row: u32,
        col: u32,
        formula: &'a str,
    ) -> SheetsFuture<'a, Result<(), GoogleError>>;
    /// Append a hidden column and store `key` when that key is absent.
    /// An existing key stays where it is. Creating a column moves `version`.
    /// `warning_only` covers the new column with a warning-only protected range.
    fn ensure_service_column<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        key: &'a str,
        warning_only: bool,
    ) -> SheetsFuture<'a, Result<bool, GoogleError>>;
    /// Set the value of an existing metadata key. Does not move `version`.
    fn set_meta_value<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        key: &'a str,
        value: &'a str,
    ) -> SheetsFuture<'a, Result<(), GoogleError>>;
}

impl GoogleSheets for FakeBook {
    fn version<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
    ) -> SheetsFuture<'a, Result<String, GoogleError>> {
        Box::pin(async move { FakeBook::version(self, spreadsheet_id) })
    }

    fn metadata<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
    ) -> SheetsFuture<'a, Result<Vec<Meta>, GoogleError>> {
        Box::pin(async move { FakeBook::metadata(self, spreadsheet_id, sheet_id) })
    }

    fn batch_get<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        ranges: &'a [ColRange],
    ) -> SheetsFuture<'a, Result<BTreeMap<(u32, u32), BookCell>, GoogleError>> {
        Box::pin(async move { FakeBook::batch_get(self, spreadsheet_id, sheet_id, ranges) })
    }

    fn write_raw<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        runs: &'a [ValueRun],
    ) -> SheetsFuture<'a, Result<(), GoogleError>> {
        Box::pin(async move { FakeBook::write_raw(self, spreadsheet_id, sheet_id, runs) })
    }

    fn write_formula<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        row: u32,
        col: u32,
        formula: &'a str,
    ) -> SheetsFuture<'a, Result<(), GoogleError>> {
        Box::pin(async move {
            FakeBook::write_formula(self, spreadsheet_id, sheet_id, row, col, formula)
        })
    }

    fn ensure_service_column<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        key: &'a str,
        warning_only: bool,
    ) -> SheetsFuture<'a, Result<bool, GoogleError>> {
        Box::pin(async move {
            FakeBook::ensure_service_column(self, spreadsheet_id, sheet_id, key, warning_only)
        })
    }

    fn set_meta_value<'a>(
        &'a mut self,
        spreadsheet_id: &'a str,
        sheet_id: i32,
        key: &'a str,
        value: &'a str,
    ) -> SheetsFuture<'a, Result<(), GoogleError>> {
        Box::pin(
            async move { FakeBook::set_meta_value(self, spreadsheet_id, sheet_id, key, value) },
        )
    }
}

impl FakeBook {
    pub fn version(&mut self, spreadsheet_id: &str) -> Result<String, GoogleError> {
        self.limited()?;
        self.require(spreadsheet_id)?;
        if let Some(scripted) = self.version_queue.pop_front() {
            return Ok(scripted);
        }
        Ok(self.version.to_string())
    }

    pub fn metadata(
        &mut self,
        spreadsheet_id: &str,
        sheet_id: i32,
    ) -> Result<Vec<Meta>, GoogleError> {
        self.limited()?;
        let tab = self.tab(sheet_id).ok_or_else(|| missing(spreadsheet_id))?;
        Ok(tab.metadata.clone())
    }

    pub fn batch_get(
        &mut self,
        spreadsheet_id: &str,
        sheet_id: i32,
        ranges: &[ColRange],
    ) -> Result<BTreeMap<(u32, u32), BookCell>, GoogleError> {
        self.limited()?;
        self.require(spreadsheet_id)?;
        self.gets.push(ranges.to_vec());
        let normalize = self.normalize_formulas;
        let tab = self.tab(sheet_id).ok_or_else(|| missing(spreadsheet_id))?;
        let mut found = BTreeMap::new();
        for range in ranges {
            for ((row, col), cell) in &tab.cells {
                if range.covers_row(*row)
                    && *col >= range.start_col
                    && *col <= range.end_col
                    && !cell.is_empty()
                {
                    found.insert((*row, *col), shown_cell(cell, normalize));
                }
            }
            for (at, cell) in synthesized(tab, range) {
                found.entry(at).or_insert(cell);
            }
        }
        Ok(found)
    }

    pub fn write_raw(
        &mut self,
        spreadsheet_id: &str,
        sheet_id: i32,
        runs: &[ValueRun],
    ) -> Result<(), GoogleError> {
        self.limited()?;
        self.require(spreadsheet_id)?;
        let tab = self
            .tab_mut(sheet_id)
            .ok_or_else(|| missing(spreadsheet_id))?;
        let mut cells = Vec::new();
        for run in runs {
            for (offset, text) in run.values.iter().enumerate() {
                let row = run.start_row + offset as u32;
                tab.put_text(row, run.col, text);
                cells.push((row, run.col, text.clone()));
            }
        }
        self.raw_writes.push(RawWrite {
            cells,
            runs: runs.to_vec(),
        });
        self.bump();
        Ok(())
    }

    pub fn write_formula(
        &mut self,
        spreadsheet_id: &str,
        sheet_id: i32,
        row: u32,
        col: u32,
        formula: &str,
    ) -> Result<(), GoogleError> {
        self.limited()?;
        self.require(spreadsheet_id)?;
        let tab = self
            .tab_mut(sheet_id)
            .ok_or_else(|| missing(spreadsheet_id))?;
        tab.put_formula(row, col, formula);
        self.formulas.push(formula.to_string());
        self.bump();
        Ok(())
    }

    pub fn ensure_service_column(
        &mut self,
        spreadsheet_id: &str,
        sheet_id: i32,
        key: &str,
        warning_only: bool,
    ) -> Result<bool, GoogleError> {
        self.limited()?;
        self.require(spreadsheet_id)?;
        let tab = self
            .tab_mut(sheet_id)
            .ok_or_else(|| missing(spreadsheet_id))?;
        if tab.metadata.iter().any(|item| item.key == key) {
            return Ok(false);
        }
        let col = tab.append_hidden();
        tab.metadata.push(Meta {
            key: key.to_string(),
            column: col,
            value: String::new(),
        });
        if warning_only {
            tab.warning_only.insert(col);
        }
        self.bump();
        Ok(true)
    }

    pub fn set_meta_value(
        &mut self,
        spreadsheet_id: &str,
        sheet_id: i32,
        key: &str,
        value: &str,
    ) -> Result<(), GoogleError> {
        self.require(spreadsheet_id)?;
        let tab = self
            .tab_mut(sheet_id)
            .ok_or_else(|| missing(spreadsheet_id))?;
        let item = tab
            .metadata
            .iter_mut()
            .find(|item| item.key == key)
            .ok_or_else(|| GoogleError::Message(format!("metadata {key}")))?;
        item.value = value.to_string();
        Ok(())
    }
}

/// Formula text a read returns when `normalize_formulas` is set.
///
/// Spaces are removed and letters outside quotes are uppercased. A space after
/// `=` keeps the result different from the text `write_formula` stored.
fn shown_cell(cell: &BookCell, normalize: bool) -> BookCell {
    if !normalize {
        return cell.clone();
    }
    let mut shown = cell.clone();
    if let Some(formula) = shown.formula.as_mut() {
        *formula = sheets_formula(formula);
    }
    shown
}

fn sheets_formula(formula: &str) -> String {
    let compact = normalize_formula(formula);
    match compact.strip_prefix('=') {
        Some(rest) => format!("= {rest}"),
        None => compact,
    }
}

fn normalize_formula(formula: &str) -> String {
    let mut out = String::new();
    let mut chars = formula.chars().peekable();
    let mut in_string = false;
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    out.push(chars.next().expect("escaped quote"));
                } else {
                    in_string = false;
                }
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }
        if ch.is_whitespace() {
            continue;
        }
        out.push(ch.to_ascii_uppercase());
    }
    out
}

impl ColRange {
    fn covers_row(&self, row: u32) -> bool {
        row >= self.start_row && self.end_row.is_none_or(|end| row <= end)
    }
}

impl FakeBook {
    fn require(&self, spreadsheet_id: &str) -> Result<(), GoogleError> {
        if self.spreadsheet_id == spreadsheet_id {
            Ok(())
        } else {
            Err(missing(spreadsheet_id))
        }
    }
}

fn missing(spreadsheet_id: &str) -> GoogleError {
    GoogleError::Message(format!("unknown spreadsheet {spreadsheet_id}"))
}

/// The fingerprint formula is an `ARRAYFORMULA` over whole columns. A cell with no stored
/// value takes the joined display of those columns, which is what Sheets would show.
fn synthesized(tab: &SheetTab, range: &ColRange) -> Vec<((u32, u32), BookCell)> {
    let Some((formula, fp_col)) = tab.cells.iter().find_map(|((_, col), cell)| {
        cell.formula
            .as_ref()
            .filter(|formula| formula.starts_with('='))
            .map(|formula| (formula.clone(), *col))
    }) else {
        return Vec::new();
    };
    if fp_col < range.start_col || fp_col > range.end_col {
        return Vec::new();
    }
    let Some((start, cols)) = formula_inputs(&formula) else {
        return Vec::new();
    };
    let Some(last) = cols.iter().filter_map(|col| tab.last_nonempty(*col)).max() else {
        return Vec::new();
    };
    let from = range.start_row.max(start);
    let to = range.end_row.map(|end| end.min(last)).unwrap_or(last);
    if from > to {
        return Vec::new();
    }
    let mut cells = Vec::new();
    for row in from..=to {
        if tab.cells.contains_key(&(row, fp_col)) {
            continue;
        }
        let parts: Vec<String> = cols.iter().map(|col| display_at(tab, row, *col)).collect();
        if parts.iter().all(|part| part.is_empty()) {
            continue;
        }
        cells.push(((row, fp_col), BookCell::text(parts.join("|"))));
    }
    cells
}

fn formula_inputs(formula: &str) -> Option<(u32, Vec<u32>)> {
    let (_, rest) = formula.split_once(",\"\",")?;
    let rest = rest.trim_end_matches(')');
    let mut cols = Vec::new();
    let mut start = None;
    for piece in rest.split("&\"|\"&") {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let letter: String = piece
            .chars()
            .take_while(|ch| ch.is_ascii_alphabetic())
            .collect();
        let col = parse_excel_id(&letter)?;
        let digits: String = piece[letter.len()..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        let row = digits.parse().ok()?;
        start = Some(row);
        cols.push(col);
    }
    Some((start?, cols))
}

fn display_at(tab: &SheetTab, row: u32, col: u32) -> String {
    let Some(value) = tab
        .cells
        .get(&(row, col))
        .and_then(|cell| cell.value.as_ref())
    else {
        return String::new();
    };
    match value {
        CellValue::Text(text) => text.clone(),
        CellValue::Int(number) => number.to_string(),
        CellValue::Float(number) => number.to_string(),
        CellValue::Bool(flag) => flag.to_string(),
        CellValue::Date(date) => {
            if date.has_time {
                format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    date.year, date.month, date.day, date.hour, date.minute, date.second
                )
            } else {
                format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
            }
        }
    }
}
