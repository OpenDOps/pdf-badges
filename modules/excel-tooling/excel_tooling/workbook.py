"""LibreOffice workbook."""

from pathlib import Path

from excel_tooling.model import DataRow, Grid, GridCell, SheetInfo
from excel_tooling.scan import candidate_grid, columns_for_header, header_candidates, walk_blocks
from excel_tooling.values import Reading, classify

_ALLOWED = {".xlsx", ".xlsm", ".xls", ".ods"}
_OPEN_PROPERTIES = (
    ("Hidden", True),
    ("MacroExecutionMode", 0),
    ("UpdateDocMode", 0),
)
_desktop = None


class PasswordRequired(Exception):
    pass


class BadFile(Exception):
    def __init__(self, detail: str):
        self.detail = detail
        super().__init__(detail)


class NoData(Exception):
    pass


class SheetNotListed(Exception):
    def __init__(self, sheet_index: int):
        self.sheet_index = sheet_index
        super().__init__(str(sheet_index))


class Workbook:
    def __init__(self, doc):
        self._doc = doc
        self._sources: dict[int, _CalcSource] = {}
        self._listed: tuple[SheetInfo, ...] | None = None
        self._prefixes: dict[tuple[int, int], tuple] = {}
        self.selected_sheet_index: int | None = None

    def sheets(self) -> tuple[SheetInfo, ...]:
        if self._listed is None:
            listed = []
            count = self._doc.Sheets.getCount()
            for index in range(count):
                sheet = self._doc.Sheets.getByIndex(index)
                source = _CalcSource(self._doc, sheet)
                if not source.has_values():
                    continue
                self._sources[index] = source
                listed.append(SheetInfo(index, sheet.Name))
            if not listed:
                raise NoData()
            self._listed = tuple(listed)
            self.selected_sheet_index = listed[0].index
        return self._listed

    def candidates(self, sheet_index: int, k: int):
        return self._prefix(sheet_index, k)[1]

    def select_header(self, sheet_index: int, excel_row: int, k: int, subcolumn_row: int | None = None):
        prefix, found = self._prefix(sheet_index, k)
        source = self._sources[sheet_index]
        first_row, last_row, first_col, last_col = source.bounds()
        cells: dict[int, list[GridCell]] = {}
        self._take_row(source, cells, prefix, excel_row)
        if subcolumn_row is not None:
            self._take_row(source, cells, prefix, subcolumn_row)
        grid = Grid(first_row, last_row, first_col, last_col, cells)
        return columns_for_header(grid, excel_row, found, subcolumn_row)

    def _prefix(self, sheet_index: int, k: int):
        self._require(sheet_index)
        key = (sheet_index, k)
        cached = self._prefixes.get(key)
        if cached is None:
            source = self._sources[sheet_index]
            grid = candidate_grid(source, k)
            cached = (grid, header_candidates(grid, k))
            self._prefixes[key] = cached
        return cached

    def _take_row(self, source, cells: dict, prefix: Grid, excel_row: int) -> None:
        stored = prefix.cells.get(excel_row)
        if stored is not None:
            cells[excel_row] = stored
            return
        cells.update(source.read(excel_row, excel_row))
        source.release()

    def iter_blocks(
        self, sheet_index: int, excel_row: int, k: int, subcolumn_row: int | None = None
    ):
        columns = self.select_header(sheet_index, excel_row, k, subcolumn_row)
        source = self._sources[sheet_index]
        _first, last_row, _first_col, _last_col = source.bounds()
        start = (subcolumn_row if subcolumn_row is not None else excel_row) + 1
        yield from walk_blocks(source, start, last_row, columns)

    def iter_columns(self, sheet_index: int, columns):
        """Read every used row. `columns` were not taken from a header row."""
        self._require(sheet_index)
        source = self._sources[sheet_index]
        first_row, last_row, _first_col, _last_col = source.bounds()
        yield from walk_blocks(source, first_row, last_row, columns)

    def read_table(
        self, sheet_index: int, excel_row: int, k: int, subcolumn_row: int | None = None
    ):
        columns = self.select_header(sheet_index, excel_row, k, subcolumn_row)
        rows: list[DataRow] = []
        for block in self.iter_blocks(sheet_index, excel_row, k, subcolumn_row):
            rows.extend(block)
        return columns, tuple(rows)

    def close(self) -> None:
        doc = self._doc
        self._doc = None
        if doc is None:
            return
        try:
            doc.close(True)
        except Exception:
            doc.dispose()

    def _require(self, sheet_index: int) -> None:
        listed = {sheet.index for sheet in self.sheets()}
        if sheet_index not in listed:
            raise SheetNotListed(sheet_index)


def open(path: str | Path, password: str | None = None) -> Workbook:
    source = Path(path)
    if source.suffix.lower() not in _ALLOWED:
        raise BadFile(source.suffix)
    doc = _load(source, password or None)
    book = Workbook(doc)
    try:
        book.sheets()
    except NoData:
        book.close()
        raise
    return book


def _load(path: Path, password: str | None = None):
    import uno

    handler = _abort_password_prompt()
    url = uno.systemPathToFileUrl(str(path.resolve()))
    doc = _desktop_instance().loadComponentFromURL(
        url, "_blank", 0, _properties(password, handler)
    )
    if handler is not None and handler.password_requested:
        if doc is not None:
            try:
                doc.close(True)
            except Exception:
                doc.dispose()
        raise PasswordRequired()
    if doc is None:
        raise RuntimeError(f"Calc did not open {path.name}")
    try:
        doc.enableAutomaticCalculation(True)
    except AttributeError:
        pass
    doc.calculateAll()
    return doc


def _properties(password: str | None = None, handler=None):
    from com.sun.star.beans import PropertyValue

    values = list(_OPEN_PROPERTIES)
    if password:
        values.append(("Password", password))
    if handler is not None:
        values.append(("InteractionHandler", handler))
    items = []
    for name, value in values:
        item = PropertyValue()
        item.Name = name
        item.Value = value
        items.append(item)
    return tuple(items)


def _abort_password_prompt():
    try:
        import unohelper
        from com.sun.star.task import XInteractionHandler
    except ImportError:
        return None

    class Handler(unohelper.Base, XInteractionHandler):
        def __init__(self):
            self.password_requested = False

        def handle(self, request):
            self.password_requested = True
            for continuation in request.getContinuations():
                name = ""
                try:
                    name = continuation.getImplementationName()
                except Exception:
                    name = ""
                if "Abort" in name and hasattr(continuation, "select"):
                    continuation.select()
                    return

    return Handler()


def _desktop_instance():
    global _desktop
    if _desktop is not None:
        return _desktop
    import uno

    local = uno.getComponentContext()
    resolver = local.ServiceManager.createInstanceWithContext(
        "com.sun.star.bridge.UnoUrlResolver", local
    )
    context = resolver.resolve(
        "uno:socket,host=127.0.0.1,port=2002;urp;StarOffice.ComponentContext"
    )
    _desktop = context.ServiceManager.createInstanceWithContext(
        "com.sun.star.frame.Desktop", context
    )
    return _desktop


class _CalcSource:
    """One listed sheet. Reads a row window and drops it on release."""

    def __init__(self, doc, sheet):
        self._doc = doc
        self._sheet = sheet
        self._bounds = _used_bounds(sheet)
        self.held = None

    def bounds(self):
        return self._bounds

    def has_values(self) -> bool:
        first_row, last_row, first_col, last_col = self._bounds
        if first_row == last_row and first_col == last_col:
            return _cell_has_value(self._sheet, first_col, first_row)
        return _range_has_value(self._sheet, first_col, last_col, first_row, last_row)

    def nonempty_rows(self, k: int):
        if k <= 0:
            return
        first_row, last_row, first_col, last_col = self._bounds
        from com.sun.star.sheet.CellFlags import DATETIME, FORMULA, STRING, VALUE

        cell_range = self._sheet.getCellRangeByPosition(
            first_col, first_row - 1, last_col, last_row - 1
        )
        flags = int(VALUE) | int(DATETIME) | int(STRING) | int(FORMULA)
        found = cell_range.queryContentCells(flags)
        cells = found.getCells()
        if cells is None:
            return
        enum = cells.createEnumeration()
        previous = None
        produced = 0
        while enum.hasMoreElements():
            address = enum.nextElement().getCellAddress()
            excel_row = address.Row + 1
            if excel_row == previous:
                continue
            previous = excel_row
            produced += 1
            yield excel_row
            if produced == k:
                return

    def read(self, first_row: int, last_row: int, columns=None) -> dict[int, list[GridCell]]:
        self.held = _read_window(self._doc, self._sheet, self._bounds, first_row, last_row, columns)
        return self.held

    def release(self) -> None:
        self.held = None


def _used_bounds(sheet):
    cursor = sheet.createCursor()
    cursor.gotoStartOfUsedArea(False)
    cursor.gotoEndOfUsedArea(True)
    address = cursor.getRangeAddress()
    return (
        address.StartRow + 1,
        address.EndRow + 1,
        address.StartColumn,
        address.EndColumn,
    )


def _range_has_value(sheet, first_col: int, last_col: int, first_row: int, last_row: int) -> bool:
    from com.sun.star.sheet.CellFlags import DATETIME, FORMULA, STRING, VALUE

    cell_range = sheet.getCellRangeByPosition(first_col, first_row - 1, last_col, last_row - 1)
    flags = int(VALUE) | int(DATETIME) | int(STRING) | int(FORMULA)
    found = cell_range.queryContentCells(flags)
    cells = found.getCells()
    if cells is None:
        return False
    enum = cells.createEnumeration()
    while enum.hasMoreElements():
        if _uno_cell_has_value(enum.nextElement()):
            return True
    return False


def _cell_has_value(sheet, col: int, excel_row: int) -> bool:
    return _uno_cell_has_value(sheet.getCellByPosition(col, excel_row - 1))


def _uno_cell_has_value(cell) -> bool:
    from com.sun.star.table.CellContentType import EMPTY

    if int(cell.getError()) != 0:
        return True
    return cell.getType() != EMPTY


def _column_spans(columns) -> tuple[tuple[int, int], ...]:
    ordered = sorted(set(columns))
    if not ordered:
        return ()
    spans: list[tuple[int, int]] = []
    start = previous = ordered[0]
    for col in ordered[1:]:
        if col == previous + 1:
            previous = col
            continue
        spans.append((start, previous))
        start = previous = col
    spans.append((start, previous))
    return tuple(spans)


def _read_window(doc, sheet, bounds, first_row: int, last_row: int, columns=None) -> dict[int, list[GridCell]]:
    _origin, _end, first_col, last_col = bounds
    spans = ((first_col, last_col),) if columns is None else _column_spans(columns)
    if not spans:
        return {}
    null = doc.NullDate
    bits = _format_bits()
    cells: dict[int, list[GridCell]] = {}
    for start_col, end_col in spans:
        for row, row_cells in _read_span(doc, sheet, start_col, end_col, first_row, last_row, null, bits).items():
            cells.setdefault(row, []).extend(row_cells)
    return cells


def _read_span(doc, sheet, first_col: int, last_col: int, first_row: int, last_row: int, null, bits):
    cell_range = sheet.getCellRangeByPosition(first_col, first_row - 1, last_col, last_row - 1)
    data = cell_range.getDataArray()
    errors = _formula_errors(cell_range)
    rects = _special_rects(doc, cell_range)
    cells: dict[int, list[GridCell]] = {}
    for row_offset, row_data in enumerate(data):
        absolute_row = (first_row - 1) + row_offset
        row_cells = []
        for col_offset, raw in enumerate(row_data):
            absolute_col = first_col + col_offset
            error = errors.get((absolute_col, absolute_row))
            if error is None and _is_empty(raw):
                continue
            flags = _flags_at(rects, absolute_col, absolute_row)
            reading = _reading(raw, error, flags, null, bits)
            value = classify(reading)
            if value is None:
                continue
            row_cells.append(GridCell(absolute_col, _display(raw, error), value))
        if row_cells:
            cells[absolute_row + 1] = row_cells
    return cells


def _formula_errors(cell_range) -> dict[tuple[int, int], str]:
    from com.sun.star.sheet.CellFlags import FORMULA

    found = {}
    ranges = cell_range.queryContentCells(FORMULA)
    cells = ranges.getCells()
    if cells is None:
        return found
    enum = cells.createEnumeration()
    while enum.hasMoreElements():
        cell = enum.nextElement()
        if cell.getError() == 0:
            continue
        address = cell.getCellAddress()
        found[(address.Column, address.Row)] = cell.getString()
    return found


def _special_rects(doc, cell_range):
    from com.sun.star.util.NumberFormat import CURRENCY, DATE, LOGICAL, PERCENT, TIME

    special = int(DATE) | int(TIME) | int(LOGICAL) | int(PERCENT) | int(CURRENCY)
    cache: dict[int, int] = {}
    rects = []
    unique = cell_range.UniqueCellFormatRanges
    for index in range(unique.getCount()):
        fmt_range = unique.getByIndex(index)
        flags = _format_type(doc, int(fmt_range.NumberFormat), cache)
        if flags & special == 0:
            continue
        for address in fmt_range.getRangeAddresses():
            rects.append(
                (
                    address.StartRow,
                    address.StartColumn,
                    address.EndRow,
                    address.EndColumn,
                    flags,
                )
            )
    return rects


def _format_type(doc, key: int, cache: dict[int, int]) -> int:
    if key not in cache:
        fmt = doc.NumberFormats.getByKey(key)
        cache[key] = int(fmt.Type)
    return cache[key]


def _format_bits():
    from com.sun.star.util.NumberFormat import CURRENCY, DATE, LOGICAL, PERCENT, TIME

    return (int(DATE), int(TIME), int(LOGICAL), int(PERCENT), int(CURRENCY))


def _flags_at(rects, col: int, row: int) -> int:
    for start_row, start_col, end_row, end_col, flags in rects:
        if start_row <= row <= end_row and start_col <= col <= end_col:
            return flags
    return 0


def _reading(raw, error, flags: int, null, bits) -> Reading:
    date_bit, time_bit, logical_bit, percent_bit, currency_bit = bits
    kwargs = {
        "is_date": bool(flags & date_bit),
        "is_time": bool(flags & time_bit),
        "is_logical": bool(flags & logical_bit),
        "is_percent": bool(flags & percent_bit),
        "is_currency": bool(flags & currency_bit),
        "null_year": int(null.Year),
        "null_month": int(null.Month),
        "null_day": int(null.Day),
    }
    if error is not None:
        return Reading("error", display=error, **kwargs)
    if isinstance(raw, bool):
        kwargs["is_logical"] = True
        return Reading("number", display=_display(raw, None), serial=float(raw), **kwargs)
    if isinstance(raw, str):
        return Reading("text", display=raw, **kwargs)
    if isinstance(raw, (int, float)):
        return Reading("number", display=_display(raw, None), serial=float(raw), **kwargs)
    return Reading("text", display=_display(raw, None), **kwargs)


def _display(raw, error) -> str:
    if error is not None:
        return error
    if isinstance(raw, bool):
        return "TRUE" if raw else "FALSE"
    if isinstance(raw, str):
        return raw
    if isinstance(raw, float) and raw.is_integer():
        return str(int(raw))
    if isinstance(raw, (int, float)):
        return str(raw)
    return "" if raw is None else str(raw)


def _is_empty(value) -> bool:
    return value is None or value == ""
