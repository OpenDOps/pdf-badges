/// Header candidates and data rows.
use std::collections::BTreeMap;

use crate::columns::{excel_id, parse_excel_id};
use crate::model::{Candidate, Column, DataRow, Grid, GridCell};

pub const EMPTY_ROW_GAP: u32 = 50;
pub const BLOCK_ROWS: u32 = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    DuplicateColumn(String),
    HeaderNotCandidate(u32),
    SubcolumnRowOutOfRange(u32),
    BadColumn(String),
}

pub fn header_candidates(grid: &Grid, k: usize) -> Vec<Candidate> {
    let mut found = Vec::new();
    for (&excel_row, _) in &grid.cells {
        if excel_row > grid.last_row {
            break;
        }
        let columns = columns_of_row(grid, excel_row);
        if columns.is_empty() {
            continue;
        }
        found.push(Candidate { excel_row, columns });
        if found.len() == k {
            break;
        }
    }
    found
}

pub fn columns_for_header(
    grid: &Grid,
    excel_row: u32,
    candidates: &[Candidate],
    subcolumn_row: Option<u32>,
) -> Result<Vec<Column>, ScanError> {
    let chosen = candidates
        .iter()
        .any(|candidate| candidate.excel_row == excel_row);
    if !chosen {
        return Err(ScanError::HeaderNotCandidate(excel_row));
    }
    let columns = columns_of_row(grid, excel_row);
    reject_duplicates(&columns)?;
    let Some(subcolumn_row) = subcolumn_row else {
        return Ok(columns);
    };
    if subcolumn_row <= excel_row || subcolumn_row > excel_row + 4 || subcolumn_row > grid.last_row
    {
        return Err(ScanError::SubcolumnRowOutOfRange(subcolumn_row));
    }
    attach_subcolumns(grid, &columns, subcolumn_row)
}

/// Visit rows that contain a value until `k` header rows are found. Empty spans are not read.
pub fn candidate_grid<S: GridSource>(source: &mut S, k: usize) -> Grid {
    let bounds = source.bounds();
    let mut cells = BTreeMap::new();
    let mut found = 0;
    let mut stop = bounds.first_row;
    for excel_row in source.nonempty_rows(k) {
        if excel_row < bounds.first_row || excel_row > bounds.last_row {
            continue;
        }
        stop = excel_row;
        let raw = source.read(excel_row, excel_row, None);
        let row_cells = raw.get(&excel_row).cloned().unwrap_or_default();
        source.release();
        if !row_cells.is_empty() {
            cells.insert(excel_row, row_cells.clone());
        }
        let probe = Grid {
            first_row: bounds.first_row,
            last_row: excel_row,
            first_col: bounds.first_col,
            last_col: bounds.last_col,
            cells: BTreeMap::from([(excel_row, row_cells)]),
        };
        if !columns_of_row(&probe, excel_row).is_empty() {
            found += 1;
        }
        if found == k {
            break;
        }
    }
    Grid {
        first_row: bounds.first_row,
        last_row: stop,
        first_col: bounds.first_col,
        last_col: bounds.last_col,
        cells,
    }
}

/// Processed rows for each sheet-row block. The source releases the raw block before the block is returned.
pub fn walk_blocks<S: GridSource>(
    source: &mut S,
    start: u32,
    end: u32,
    columns: &[Column],
    block_size: u32,
) -> Vec<Vec<DataRow>> {
    let leaf_cols: Vec<u32> = leaves(columns)
        .into_iter()
        .map(|column| column.index)
        .collect();
    let mut skipped = 0u32;
    let mut blocks = Vec::new();
    for (first, last) in block_windows(start, end, block_size) {
        let raw = source.read(first, last, Some(&leaf_cols));
        let mut processed = Vec::new();
        let mut stop = false;
        for excel_row in first..=last {
            let by_col = raw.get(&excel_row).map(|cells| {
                let mut map = BTreeMap::new();
                for cell in cells {
                    map.insert(cell.col, cell.clone());
                }
                map
            });
            let cells: Vec<GridCell> = leaf_cols
                .iter()
                .filter_map(|col| by_col.as_ref().and_then(|map| map.get(col).cloned()))
                .collect();
            if cells.is_empty() {
                skipped += 1;
                if skipped == EMPTY_ROW_GAP {
                    stop = true;
                    break;
                }
                continue;
            }
            skipped = 0;
            processed.push(DataRow { excel_row, cells });
        }
        source.release();
        if !processed.is_empty() {
            blocks.push(processed);
        }
        if stop {
            break;
        }
    }
    blocks
}

pub fn columns_without_header(specs: &[(&str, &str)]) -> Result<Vec<Column>, ScanError> {
    if specs.is_empty() {
        return Err(ScanError::BadColumn("columns".into()));
    }
    let mut columns = Vec::new();
    let mut seen_ids = Vec::new();
    let mut seen_names = Vec::new();
    for (raw_id, raw_name) in specs {
        let Some(index) = parse_excel_id(raw_id) else {
            return Err(ScanError::BadColumn((*raw_id).into()));
        };
        let letter = excel_id(index);
        if seen_ids.contains(&letter) {
            return Err(ScanError::DuplicateColumn(letter));
        }
        let name = raw_name.trim();
        if name.is_empty() {
            return Err(ScanError::BadColumn("name".into()));
        }
        if seen_names.iter().any(|seen: &String| seen == name) {
            return Err(ScanError::DuplicateColumn(name.into()));
        }
        seen_ids.push(letter.clone());
        seen_names.push(name.to_string());
        columns.push(Column::new(index, letter, name));
    }
    Ok(columns)
}

/// Data starts on the first used row. No row is consumed as a header.
pub fn rows_without_header(grid: &Grid, columns: &[Column]) -> Vec<DataRow> {
    emit_rows(grid, grid.first_row, columns)
}

pub fn data_rows(
    grid: &Grid,
    header_row: u32,
    columns: &[Column],
    subcolumn_row: Option<u32>,
) -> Vec<DataRow> {
    let start = subcolumn_row.unwrap_or(header_row) + 1;
    emit_rows(grid, start, columns)
}

fn emit_rows(grid: &Grid, start: u32, columns: &[Column]) -> Vec<DataRow> {
    let header_cols: Vec<u32> = leaves(columns)
        .into_iter()
        .map(|column| column.index)
        .collect();
    let mut rows = Vec::new();
    let mut skipped = 0u32;
    if start > grid.last_row {
        return rows;
    }
    for excel_row in start..=grid.last_row {
        let by_col = cells_in_span(grid, excel_row);
        let cells: Vec<GridCell> = header_cols
            .iter()
            .filter_map(|col| by_col.get(col).cloned())
            .collect();
        if cells.is_empty() {
            skipped += 1;
            if skipped == EMPTY_ROW_GAP {
                break;
            }
            continue;
        }
        skipped = 0;
        rows.push(DataRow { excel_row, cells });
    }
    rows
}

fn reject_duplicates(columns: &[Column]) -> Result<(), ScanError> {
    let mut seen = Vec::new();
    for column in columns {
        if seen.contains(&column.name) {
            return Err(ScanError::DuplicateColumn(column.name.clone()));
        }
        seen.push(column.name.clone());
    }
    Ok(())
}

fn attach_subcolumns(
    grid: &Grid,
    parents: &[Column],
    subcolumn_row: u32,
) -> Result<Vec<Column>, ScanError> {
    let sub_cells = cells_in_span(grid, subcolumn_row);
    let end = grid.last_col + 1;
    let mut attached = Vec::new();
    for (index, parent) in parents.iter().enumerate() {
        let next_index = parents
            .get(index + 1)
            .map(|column| column.index)
            .unwrap_or(end);
        let mut subs = Vec::new();
        for col in parent.index..next_index {
            let Some(cell) = sub_cells.get(&col) else {
                continue;
            };
            let name = cell.display.trim();
            if name.is_empty() {
                continue;
            }
            subs.push(Column::new(col, excel_id(col), name));
        }
        reject_duplicates(&subs)?;
        if subs.is_empty() {
            attached.push(parent.clone());
        } else {
            let mut column = parent.clone();
            column.subcolumns = subs;
            attached.push(column);
        }
    }
    Ok(attached)
}

fn leaves(columns: &[Column]) -> Vec<Column> {
    let mut leaves = Vec::new();
    for column in columns {
        if column.subcolumns.is_empty() {
            leaves.push(column.clone());
        } else {
            leaves.extend(column.subcolumns.clone());
        }
    }
    leaves
}

fn columns_of_row(grid: &Grid, excel_row: u32) -> Vec<Column> {
    cells_in_span(grid, excel_row)
        .into_values()
        .filter_map(|cell| {
            let name = cell.display.trim();
            if name.is_empty() {
                None
            } else {
                Some(Column::new(cell.col, excel_id(cell.col), name))
            }
        })
        .collect()
}

fn cells_in_span(grid: &Grid, excel_row: u32) -> BTreeMap<u32, GridCell> {
    let mut cells = BTreeMap::new();
    for cell in grid.cells.get(&excel_row).into_iter().flatten() {
        if grid.first_col <= cell.col && cell.col <= grid.last_col {
            cells.insert(cell.col, cell.clone());
        }
    }
    cells
}

fn block_windows(start: u32, end: u32, size: u32) -> Vec<(u32, u32)> {
    let mut windows = Vec::new();
    if size == 0 || start > end {
        return windows;
    }
    let mut row = start;
    while row <= end {
        let last = (row + size - 1).min(end);
        windows.push((row, last));
        row = last + 1;
    }
    windows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub first_row: u32,
    pub last_row: u32,
    pub first_col: u32,
    pub last_col: u32,
}

pub trait GridSource {
    fn bounds(&self) -> Bounds;
    fn has_values(&self) -> bool;
    fn nonempty_rows(&self, k: usize) -> Vec<u32>;
    fn read(
        &mut self,
        first: u32,
        last: u32,
        columns: Option<&[u32]>,
    ) -> BTreeMap<u32, Vec<GridCell>>;
    fn release(&mut self);
}
