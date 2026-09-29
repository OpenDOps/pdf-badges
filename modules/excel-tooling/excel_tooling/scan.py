"""Header candidates and data rows."""

from excel_tooling.columns import excel_id
from excel_tooling.model import Candidate, Column, DataRow, Grid, GridCell

EMPTY_ROW_GAP = 50
BLOCK_ROWS = 2000


class DuplicateColumn(Exception):
    def __init__(self, name: str):
        self.name = name
        super().__init__(name)


class HeaderNotCandidate(Exception):
    def __init__(self, excel_row: int):
        self.excel_row = excel_row
        super().__init__(str(excel_row))


class SubcolumnRowOutOfRange(Exception):
    def __init__(self, excel_row: int):
        self.excel_row = excel_row
        super().__init__(str(excel_row))


def header_candidates(grid: Grid, k: int) -> tuple[Candidate, ...]:
    found: list[Candidate] = []
    for excel_row in range(1, grid.last_row + 1):
        columns = _columns_of_row(grid, excel_row)
        if not columns:
            continue
        found.append(Candidate(excel_row, columns))
        if len(found) == k:
            break
    return tuple(found)


def columns_for_header(
    grid: Grid,
    excel_row: int,
    candidates: tuple[Candidate, ...],
    subcolumn_row: int | None = None,
) -> tuple[Column, ...]:
    chosen = next(
        (candidate for candidate in candidates if candidate.excel_row == excel_row),
        None,
    )
    if chosen is None:
        raise HeaderNotCandidate(excel_row)
    columns = _columns_of_row(grid, excel_row)
    _reject_duplicates(columns)
    if subcolumn_row is None:
        return columns
    if subcolumn_row <= excel_row or subcolumn_row > excel_row + 4 or subcolumn_row > grid.last_row:
        raise SubcolumnRowOutOfRange(subcolumn_row)
    return _attach_subcolumns(grid, columns, subcolumn_row)


def candidate_grid(source, k: int) -> Grid:
    """Read one sheet row at a time until `k` non-empty header rows are found."""
    first_row, last_row, first_col, last_col = source.bounds()
    cells: dict[int, list[GridCell]] = {}
    found = 0
    stop = first_row
    for excel_row in range(first_row, last_row + 1):
        stop = excel_row
        raw = source.read(excel_row, excel_row)
        row_cells = list(raw.get(excel_row, []))
        source.release()
        if row_cells:
            cells[excel_row] = row_cells
        probe = Grid(first_row, excel_row, first_col, last_col, {excel_row: row_cells})
        if _columns_of_row(probe, excel_row):
            found += 1
        if found == k:
            break
    return Grid(first_row, stop, first_col, last_col, cells)


def walk_blocks(source, start: int, end: int, columns: tuple[Column, ...], block_size: int = BLOCK_ROWS):
    """Yield processed rows for each sheet-row block. The source releases the raw block before the yield."""
    leaf_cols = tuple(column.index for column in _leaves(columns))
    skipped = 0
    for first, last in _block_windows(start, end, block_size):
        raw = source.read(first, last)
        processed: list[DataRow] = []
        stop = False
        for excel_row in range(first, last + 1):
            by_col = {cell.col: cell for cell in raw.get(excel_row, [])}
            cells = tuple(by_col[col] for col in leaf_cols if col in by_col)
            if not cells:
                skipped += 1
                if skipped == EMPTY_ROW_GAP:
                    stop = True
                    break
                continue
            skipped = 0
            processed.append(DataRow(excel_row, cells))
        source.release()
        if processed:
            yield processed
        if stop:
            return


def _block_windows(start: int, end: int, size: int):
    row = start
    while row <= end:
        last = min(row + size - 1, end)
        yield row, last
        row = last + 1


def data_rows(
    grid: Grid,
    header_row: int,
    columns: tuple[Column, ...],
    subcolumn_row: int | None = None,
) -> tuple[DataRow, ...]:
    header_cols = tuple(column.index for column in _leaves(columns))
    start = (subcolumn_row if subcolumn_row is not None else header_row) + 1
    rows: list[DataRow] = []
    skipped = 0
    for excel_row in range(start, grid.last_row + 1):
        by_col = _cells_in_span(grid, excel_row)
        cells = tuple(by_col[col] for col in header_cols if col in by_col)
        if not cells:
            skipped += 1
            if skipped == EMPTY_ROW_GAP:
                break
            continue
        skipped = 0
        rows.append(DataRow(excel_row, cells))
    return tuple(rows)


def _reject_duplicates(columns: tuple[Column, ...]) -> None:
    seen: set[str] = set()
    for column in columns:
        if column.name in seen:
            raise DuplicateColumn(column.name)
        seen.add(column.name)


def _attach_subcolumns(
    grid: Grid, parents: tuple[Column, ...], subcolumn_row: int
) -> tuple[Column, ...]:
    sub_cells = _cells_in_span(grid, subcolumn_row)
    end = grid.last_col + 1
    attached: list[Column] = []
    for index, parent in enumerate(parents):
        next_index = parents[index + 1].index if index + 1 < len(parents) else end
        subs: list[Column] = []
        for col in range(parent.index, next_index):
            cell = sub_cells.get(col)
            if cell is None:
                continue
            name = cell.display.strip()
            if not name:
                continue
            subs.append(Column(index=col, excel_id=excel_id(col), name=name))
        _reject_duplicates(tuple(subs))
        if subs:
            attached.append(
                Column(parent.index, parent.excel_id, parent.name, tuple(subs))
            )
        else:
            attached.append(parent)
    return tuple(attached)


def _leaves(columns: tuple[Column, ...]) -> tuple[Column, ...]:
    leaves: list[Column] = []
    for column in columns:
        if column.subcolumns:
            leaves.extend(column.subcolumns)
        else:
            leaves.append(column)
    return tuple(leaves)


def _columns_of_row(grid: Grid, excel_row: int) -> tuple[Column, ...]:
    columns = []
    for cell in _cells_in_span(grid, excel_row).values():
        name = cell.display.strip()
        if not name:
            continue
        columns.append(Column(index=cell.col, excel_id=excel_id(cell.col), name=name))
    return tuple(columns)


def _cells_in_span(grid: Grid, excel_row: int) -> dict[int, GridCell]:
    return {
        cell.col: cell
        for cell in sorted(grid.cells.get(excel_row, []), key=lambda cell: cell.col)
        if grid.first_col <= cell.col <= grid.last_col
    }
