from pathlib import Path

import grpc
import pytest

from excel_tooling.model import CellValue, Grid, GridCell, SheetInfo
from excel_tooling.scan import columns_for_header, data_rows, header_candidates
from excel_tooling.server import IDLE_SECONDS, serve
from excel_tooling.workbook import SheetNotListed

import excel_tooling  # noqa: F401
from irbis.excel.v1 import excel_pb2, excel_pb2_grpc

FIXTURES = Path(__file__).resolve().parent / "fixtures"
CHANNEL_OPTIONS = [
    ("grpc.max_receive_message_length", 64 * 1024 * 1024),
    ("grpc.max_send_message_length", 64 * 1024 * 1024),
]


class FakeBook:
    def __init__(self, sheets):
        self._sheets = sheets
        self.selected_sheet_index = sheets[0][0]
        self.closed = False

    def sheets(self):
        return tuple(SheetInfo(index, name) for index, name, _grid in self._sheets)

    def candidates(self, sheet_index, k):
        return header_candidates(self._grid(sheet_index), k)

    def select_header(self, sheet_index, excel_row, k, subcolumn_row=None):
        return columns_for_header(
            self._grid(sheet_index), excel_row, self.candidates(sheet_index, k), subcolumn_row
        )

    def read_table(self, sheet_index, excel_row, k, subcolumn_row=None):
        columns = self.select_header(sheet_index, excel_row, k, subcolumn_row)
        return columns, data_rows(self._grid(sheet_index), excel_row, columns, subcolumn_row)

    def iter_blocks(self, sheet_index, excel_row, k, subcolumn_row=None):
        _columns, rows = self.read_table(sheet_index, excel_row, k, subcolumn_row)
        if rows:
            yield list(rows)

    def close(self):
        self.closed = True

    def _grid(self, sheet_index):
        for index, _name, grid in self._sheets:
            if index == sheet_index:
                return grid
        raise SheetNotListed(sheet_index)


def text(col, display, value=None):
    return GridCell(col, display, value if value is not None else CellValue(text=display))


def titles_grid():
    return Grid(
        1,
        7,
        0,
        2,
        {
            2: [text(0, "Quarter report")],
            4: [text(0, "Name"), text(1, "City"), text(2, "Amount")],
            5: [text(0, "Ann"), text(1, "Oslo"), text(2, "2")],
            7: [text(0, "Bo"), text(2, "3")],
        },
    )


def start(opener=None, clock=None):
    server, port, service = serve("127.0.0.1:0", opener=opener, clock=clock)
    channel = grpc.insecure_channel(f"127.0.0.1:{port}", options=CHANNEL_OPTIONS)
    return service, server, excel_pb2_grpc.ExcelToolingStub(channel)


def upload(stub, filename, payload=b"book", limit=0):
    meta = excel_pb2.OpenChunk(
        meta=excel_pb2.OpenMeta(filename=filename, header_candidate_limit=limit)
    )

    def chunks():
        yield meta
        yield excel_pb2.OpenChunk(data=payload)

    return stub.OpenWorkbook(chunks())


def status(call):
    with pytest.raises(grpc.RpcError) as caught:
        result = call()
        if hasattr(result, "code") and hasattr(result, "__iter__"):
            list(result)
    return caught.value.code()


def collect(stub, session_id):
    batches = list(stub.ReadTable(excel_pb2.ReadTableRequest(session_id=session_id)))
    rows = [row for batch in batches for row in batch.rows]
    return batches, rows


def test_open_returns_first_sheet():
    first = Grid(1, 1, 0, 0, {1: [text(0, "Alpha")]})
    second = Grid(1, 1, 0, 0, {1: [text(0, "Beta")]})
    book = FakeBook([(0, "First", first), (3, "Second", second)])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
    finally:
        server.stop(1)
    assert [(sheet.index, sheet.name) for sheet in view.sheets] == [(0, "First"), (3, "Second")]
    assert view.selected_sheet_index == 0
    assert [candidate.excel_row for candidate in view.header_candidates] == [1]
    assert view.header_candidates[0].columns[0].name == "Alpha"


def test_select_sheet_resets_header():
    first = Grid(1, 1, 0, 0, {1: [text(0, "Alpha")]})
    second = Grid(1, 1, 0, 0, {1: [text(0, "Beta")]})
    book = FakeBook([(0, "First", first), (3, "Second", second)])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=1)
        )
        selected = stub.SelectSheet(
            excel_pb2.SelectSheetRequest(session_id=view.session_id, sheet_index=3)
        )
        assert status(
            lambda: stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id))
        ) == grpc.StatusCode.FAILED_PRECONDITION
    finally:
        server.stop(1)
    assert selected.header_candidates[0].columns[0].name == "Beta"


def test_select_header_empty_rows():
    book = FakeBook([(0, "Titles", titles_grid())])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        table = stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4)
        )
    finally:
        server.stop(1)
    assert [column.name for column in table.columns] == ["Name", "City", "Amount"]
    assert list(table.rows) == []


def test_read_matches_scan():
    book = FakeBook([(0, "Titles", titles_grid())])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4)
        )
        _batches, rows = collect(stub, view.session_id)
    finally:
        server.stop(1)
    assert [row.excel_row for row in rows] == [5, 7]
    assert [cell.excel_id for cell in rows[1].cells] == ["A", "C"]


def test_duplicate_status():
    grid = Grid(1, 1, 0, 1, {1: [text(0, "Name"), text(1, "Name")]})
    book = FakeBook([(0, "Dup", grid)])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        code = status(
            lambda: stub.SelectHeader(
                excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=1)
            )
        )
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.INVALID_ARGUMENT


def test_bad_extension():
    service, server, stub = start(opener=lambda _path: (_ for _ in ()).throw(AssertionError("opened")))
    try:
        code = status(lambda: upload(stub, "notes.csv", b"a,b\n"))
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.INVALID_ARGUMENT
    assert service.sessions == {}


def test_k_zero_is_ten():
    grid = Grid(1, 12, 0, 0, {row: [text(0, f"r{row}")] for row in range(1, 13)})
    book = FakeBook([(0, "Rows", grid)])
    created = {"path": None}

    def opener(path):
        created["path"] = path
        return book

    _service, server, stub = start(opener=opener)
    try:
        view = upload(stub, "book.xlsx", limit=0)
        assert len(view.header_candidates) == 10
        code = status(lambda: upload(stub, "book.xlsx", limit=51))
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.INVALID_ARGUMENT


def test_unknown_session():
    _service, server, stub = start(opener=lambda _path: FakeBook([(0, "Only", titles_grid())]))
    missing = "missing"
    try:
        codes = [
            status(
                lambda: stub.SelectSheet(
                    excel_pb2.SelectSheetRequest(session_id=missing, sheet_index=0)
                )
            ),
            status(
                lambda: stub.SelectHeader(
                    excel_pb2.SelectHeaderRequest(session_id=missing, excel_row=1)
                )
            ),
            status(lambda: stub.ReadTable(excel_pb2.ReadTableRequest(session_id=missing))),
            status(lambda: stub.Close(excel_pb2.CloseRequest(session_id=missing))),
        ]
    finally:
        server.stop(1)
    assert codes == [grpc.StatusCode.NOT_FOUND] * 4


def test_close_then_read():
    book = FakeBook([(0, "Titles", titles_grid())])
    opened = {}

    def opener(path):
        opened["path"] = Path(path)
        return book

    service, server, stub = start(opener=opener)
    try:
        view = upload(stub, "book.xlsx")
        stub.Close(excel_pb2.CloseRequest(session_id=view.session_id))
        code = status(lambda: stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id)))
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.NOT_FOUND
    assert view.session_id not in service.sessions
    assert book.closed
    assert not opened["path"].exists()


def test_expired_session():
    clock = {"now": 1_000.0}
    book = FakeBook([(0, "Titles", titles_grid())])
    service, server, stub = start(opener=lambda _path: book, clock=lambda: clock["now"])
    try:
        view = upload(stub, "book.xlsx")
        clock["now"] += IDLE_SECONDS + 1
        code = status(lambda: stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id)))
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.NOT_FOUND
    assert view.session_id not in service.sessions


def test_subcolumns_over_grpc():
    sheet = Grid(
        1,
        4,
        0,
        3,
        {
            1: [text(0, "Group"), text(3, "Next")],
            2: [text(0, "One"), text(1, "Two"), text(2, "Other")],
            4: [text(0, "a"), text(1, "b"), text(3, "n")],
        },
    )
    book = FakeBook([(0, "Sheet", sheet)])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        code = status(
            lambda: stub.SelectHeader(
                excel_pb2.SelectHeaderRequest(
                    session_id=view.session_id, excel_row=1, subcolumn_row=6
                )
            )
        )
        selected = stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=1, subcolumn_row=2)
        )
        _batches, rows = collect(stub, view.session_id)
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.INVALID_ARGUMENT
    assert len(selected.rows) == 0
    group = selected.columns[0]
    assert (group.excel_id, group.name) == ("A", "Group")
    assert [column.name for column in group.subcolumns] == ["One", "Two", "Other"]
    assert selected.columns[1].subcolumns == []
    assert rows[0].excel_row == 4
    assert [cell.excel_id for cell in rows[0].cells] == ["A", "B", "D"]


def _cell(row, excel_id):
    return next(cell for cell in row.cells if cell.excel_id == excel_id)


@pytest.mark.libreoffice
def test_titles_over_grpc():
    _service, server, stub = start()
    try:
        payload = (FIXTURES / "titles.xlsx").read_bytes()
        view = upload(stub, "titles.xlsx", payload, limit=10)
        selected = stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4)
        )
        batches, rows = collect(stub, view.session_id)
    finally:
        server.stop(1)
    assert [column.name for column in selected.columns] == ["Name", "City", "Amount", "When", "Active"]
    assert len(batches) == 1
    assert [row.excel_row for row in rows] == [5, 7]
    assert _cell(rows[0], "C").value.int_value == 2
    when = _cell(rows[0], "D").value
    assert when.WhichOneof("kind") == "date"
    assert (when.date.year, when.date.month, when.date.day, when.date.has_time) == (2024, 3, 15, False)
    assert _cell(rows[0], "E").value.bool_value is True
    assert [cell.excel_id for cell in rows[1].cells if cell.excel_id == "B"] == []
    assert _cell(rows[1], "C").value.float_value == 3.5
    assert _cell(rows[1], "D").value.int_value == 2


@pytest.mark.libreoffice
def test_lingerie_over_grpc():
    _service, server, stub = start()
    try:
        payload = (FIXTURES / "JPGVL0I5VW.xlsm").read_bytes()
        view = upload(stub, "JPGVL0I5VW.xlsm", payload, limit=3)
        assert [(sheet.index, sheet.name) for sheet in view.sheets] == [
            (0, "База посетителей"),
            (1, "prop"),
        ]
        assert [candidate.excel_row for candidate in view.header_candidates] == [1, 2, 3]
        code = status(
            lambda: stub.SelectHeader(
                excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=3)
            )
        )
        assert code == grpc.StatusCode.INVALID_ARGUMENT
        selected = stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=2)
        )
        batches, rows = collect(stub, view.session_id)
        prop = stub.SelectSheet(
            excel_pb2.SelectSheetRequest(session_id=view.session_id, sheet_index=1, header_candidate_limit=3)
        )
    finally:
        server.stop(1)
    assert len(selected.columns) == 65
    assert (selected.columns[0].excel_id, selected.columns[0].name) == ("A", "ID")
    assert next(column.name for column in selected.columns if column.excel_id == "AH") == "Дата"
    assert next(column.name for column in selected.columns if column.excel_id == "DC") == (
        "Область проживания (Не из списка)"
    )
    assert all(column.excel_id != "AS" for column in selected.columns)
    assert len(rows) == 2924
    assert [len(batch.rows) for batch in batches[:-1]] == [100] * (len(batches) - 1)
    assert len(batches[-1].rows) == 24
    assert rows[0].excel_row == 3
    visitor = next(row for row in rows if row.excel_row == 4)
    assert _cell(visitor, "A").value.text == "JPGVL0I5VW_exp6"
    assert _cell(visitor, "E").value.text == "1712460"
    assert _cell(visitor, "AH").value.text == "02.09.2026"
    names = {(column.excel_id, column.name) for column in prop.header_candidates[0].columns}
    assert ("A", "headerLines") in names
    assert ("E", "colFullName") in names
