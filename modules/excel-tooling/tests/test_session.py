import threading
import time
from pathlib import Path

import grpc
import pytest

from excel_tooling.model import CellValue, Grid, GridCell, SheetInfo
from excel_tooling.scan import columns_for_header, data_rows, header_candidates, rows_without_header
from excel_tooling.server import IDLE_SECONDS, MAX_UPLOAD_BYTES, serve
from excel_tooling.workbook import SheetNotListed

import excel_tooling  # noqa: F401
from irbis.excel.v1 import excel_pb2, excel_pb2_grpc
from irbis.table.v1 import table_pb2

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
        self.watch_header = False
        self.header_entered = threading.Event()
        self.stream_started = threading.Event()
        self.stream_release = threading.Event()
        self.hold_stream = False

    def sheets(self):
        return tuple(SheetInfo(index, name) for index, name, _grid in self._sheets)

    def candidates(self, sheet_index, k):
        return header_candidates(self._grid(sheet_index), k)

    def select_header(self, sheet_index, excel_row, k, subcolumn_row=None):
        if self.watch_header:
            self.header_entered.set()
        return columns_for_header(
            self._grid(sheet_index), excel_row, self.candidates(sheet_index, k), subcolumn_row
        )

    def read_table(self, sheet_index, excel_row, k, subcolumn_row=None):
        columns = self.select_header(sheet_index, excel_row, k, subcolumn_row)
        return columns, data_rows(self._grid(sheet_index), excel_row, columns, subcolumn_row)

    def iter_columns(self, sheet_index, columns):
        rows = rows_without_header(self._grid(sheet_index), columns)
        if rows:
            yield list(rows)

    def iter_blocks(self, sheet_index, excel_row, k, subcolumn_row=None):
        if self.hold_stream:
            self.stream_started.set()
            if not self.stream_release.wait(5):
                raise TimeoutError("stream gate")
        columns = columns_for_header(
            self._grid(sheet_index), excel_row, self.candidates(sheet_index, k), subcolumn_row
        )
        rows = data_rows(self._grid(sheet_index), excel_row, columns, subcolumn_row)
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


def start(opener=None, clock=None, sweep_seconds=None, max_sessions=None):
    server, port, service = serve(
        "127.0.0.1:0",
        opener=opener,
        clock=clock,
        sweep_seconds=sweep_seconds,
        max_sessions=max_sessions,
    )
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


def _named(excel_id, name, **subs):
    column = table_pb2.Column(excel_id=excel_id, name=name)
    for sub_id, sub_name in subs.items():
        column.subcolumns.add(excel_id=sub_id, name=sub_name)
    return column


def test_select_columns_reads_the_first_row():
    sheet = Grid(
        1,
        2,
        0,
        2,
        {
            1: [text(0, "Абрамов"), text(1, "Александр"), text(2, "Авангард")],
            2: [text(0, "Авхадыев"), text(1, "Антон"), text(2, "СёрчИнформ")],
        },
    )
    book = FakeBook([(0, "Лист1", sheet)])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        table = stub.SelectColumns(
            excel_pb2.SelectColumnsRequest(
                session_id=view.session_id,
                columns=[
                    _named("A", "surname"),
                    _named("b", " name "),
                    _named("C", "company name"),
                ],
            )
        )
        _batches, rows = collect(stub, view.session_id)
    finally:
        server.stop(1)
    assert [(column.excel_id, column.name) for column in table.columns] == [
        ("A", "surname"),
        ("B", "name"),
        ("C", "company name"),
    ]
    assert list(table.rows) == []
    assert [row.excel_row for row in rows] == [1, 2]
    assert rows[0].cells[0].value.text == "Абрамов"
    assert [cell.excel_id for cell in rows[1].cells] == ["A", "B", "C"]


def test_select_columns_replaces_a_header():
    book = FakeBook([(0, "Titles", titles_grid())])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4)
        )
        stub.SelectColumns(
            excel_pb2.SelectColumnsRequest(
                session_id=view.session_id,
                columns=[_named("A", "name")],
            )
        )
        _batches, rows = collect(stub, view.session_id)
    finally:
        server.stop(1)
    assert [row.excel_row for row in rows] == [2, 4, 5, 7]


def test_select_sheet_clears_named_columns():
    first = Grid(1, 1, 0, 0, {1: [text(0, "Alpha")]})
    second = Grid(1, 1, 0, 0, {1: [text(0, "Beta")]})
    book = FakeBook([(0, "First", first), (3, "Second", second)])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        stub.SelectColumns(
            excel_pb2.SelectColumnsRequest(
                session_id=view.session_id,
                columns=[_named("A", "name")],
            )
        )
        stub.SelectSheet(excel_pb2.SelectSheetRequest(session_id=view.session_id, sheet_index=3))
        code = status(
            lambda: stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id))
        )
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.FAILED_PRECONDITION


def test_select_columns_rejects_a_bad_column():
    book = FakeBook([(0, "Лист1", Grid(1, 1, 0, 0, {1: [text(0, "Ann")]}))])
    _service, server, stub = start(opener=lambda _path: book)
    try:
        view = upload(stub, "book.xlsx")
        empty = status(
            lambda: stub.SelectColumns(
                excel_pb2.SelectColumnsRequest(session_id=view.session_id)
            )
        )
        nested = status(
            lambda: stub.SelectColumns(
                excel_pb2.SelectColumnsRequest(
                    session_id=view.session_id,
                    columns=[_named("A", "group", B="child")],
                )
            )
        )
        duplicate = status(
            lambda: stub.SelectColumns(
                excel_pb2.SelectColumnsRequest(
                    session_id=view.session_id,
                    columns=[_named("A", "name"), _named("B", "name")],
                )
            )
        )
    finally:
        server.stop(1)
    assert empty == grpc.StatusCode.INVALID_ARGUMENT
    assert nested == grpc.StatusCode.INVALID_ARGUMENT
    assert duplicate == grpc.StatusCode.INVALID_ARGUMENT


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


def test_upload_is_on_disk_before_calc():
    seen = {}

    def opener(path):
        seen["bytes"] = Path(path).read_bytes()
        return FakeBook([(0, "Titles", titles_grid())])

    def chunks():
        yield excel_pb2.OpenChunk(meta=excel_pb2.OpenMeta(filename="book.xlsx"))
        yield excel_pb2.OpenChunk(data=b"hello-")
        yield excel_pb2.OpenChunk(data=b"bytes")

    _service, server, stub = start(opener=opener)
    try:
        stub.OpenWorkbook(chunks())
    finally:
        server.stop(1)
    assert seen["bytes"] == b"hello-bytes"


def _upload_mib_chunks(extra=b""):
    piece = b"x" * (1024 * 1024)

    def chunks():
        yield excel_pb2.OpenChunk(meta=excel_pb2.OpenMeta(filename="book.xlsx"))
        remaining = MAX_UPLOAD_BYTES
        while remaining:
            take = min(len(piece), remaining)
            yield excel_pb2.OpenChunk(data=piece[:take])
            remaining -= take
        if extra:
            yield excel_pb2.OpenChunk(data=extra)

    return chunks()


def test_upload_of_64_mib_reaches_calc():
    seen = {}

    def opener(path):
        seen["size"] = Path(path).stat().st_size
        return FakeBook([(0, "Titles", titles_grid())])

    _service, server, stub = start(opener=opener)
    try:
        stub.OpenWorkbook(_upload_mib_chunks())
    finally:
        server.stop(1)
    assert seen["size"] == MAX_UPLOAD_BYTES


def test_upload_over_64_mib_is_rejected(monkeypatch):
    from excel_tooling import server as server_mod

    created = {}
    real_mkstemp = server_mod.tempfile.mkstemp

    def tracking(*args, **kwargs):
        handle, name = real_mkstemp(*args, **kwargs)
        created["path"] = Path(name)
        return handle, name

    monkeypatch.setattr(server_mod.tempfile, "mkstemp", tracking)
    service, server, stub = start(opener=lambda _path: (_ for _ in ()).throw(AssertionError("opened")))
    try:
        code = status(lambda: stub.OpenWorkbook(_upload_mib_chunks(b"x")))
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.RESOURCE_EXHAUSTED
    assert service.sessions == {}
    assert not created["path"].exists()


def test_same_workbook_waits_for_the_stream():
    book = FakeBook([(0, "Titles", titles_grid())])
    book.hold_stream = True
    _service, server, stub = start(opener=lambda _path: book)
    errors = []
    try:
        view = upload(stub, "book.xlsx")
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4))
        book.watch_header = True

        def read():
            try:
                collect(stub, view.session_id)
            except Exception as exc:
                errors.append(exc)

        def again():
            try:
                stub.SelectHeader(
                    excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4)
                )
            except Exception as exc:
                errors.append(exc)

        reader = threading.Thread(target=read)
        reader.start()
        assert book.stream_started.wait(2)
        waiter = threading.Thread(target=again)
        waiter.start()
        assert not book.header_entered.wait(0.4)
        book.stream_release.set()
        assert book.header_entered.wait(2)
        reader.join(2)
        waiter.join(2)
    finally:
        book.stream_release.set()
        server.stop(1)
    assert errors == []
    assert not reader.is_alive()
    assert not waiter.is_alive()


def test_other_workbook_runs_during_a_stream():
    blocked = FakeBook([(0, "Titles", titles_grid())])
    blocked.hold_stream = True
    other = FakeBook([(0, "Other", titles_grid())])
    books = [blocked, other]

    def opener(_path):
        return books.pop(0)

    _service, server, stub = start(opener=opener)
    opened = threading.Event()
    errors = []
    try:
        view = upload(stub, "book.xlsx")
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4))

        def read():
            try:
                collect(stub, view.session_id)
            except Exception as exc:
                errors.append(exc)

        def open_other():
            try:
                upload(stub, "other.xlsx", payload=b"other")
                opened.set()
            except Exception as exc:
                errors.append(exc)

        reader = threading.Thread(target=read)
        reader.start()
        assert blocked.stream_started.wait(2)
        second = threading.Thread(target=open_other)
        second.start()
        assert opened.wait(2)
        assert not blocked.stream_release.is_set()
        blocked.stream_release.set()
        reader.join(2)
        second.join(2)
    finally:
        blocked.stream_release.set()
        server.stop(1)
    assert errors == []
    assert not reader.is_alive()
    assert not second.is_alive()


def test_call_sweeps_other_idle_sessions():
    clock = {"now": 1_000.0}
    books = []

    def opener(_path):
        book = FakeBook([(0, "Titles", titles_grid())])
        books.append(book)
        return book

    service, server, stub = start(opener=opener, clock=lambda: clock["now"])
    try:
        first = upload(stub, "a.xlsx", payload=b"a")
        second = upload(stub, "b.xlsx", payload=b"b")
        clock["now"] += IDLE_SECONDS + 1
        third = upload(stub, "c.xlsx", payload=b"c")
    finally:
        server.stop(1)
    assert first.session_id not in service.sessions
    assert second.session_id not in service.sessions
    assert third.session_id in service.sessions
    assert books[0].closed
    assert books[1].closed
    assert not books[2].closed


def test_timer_sweeps_idle_sessions():
    clock = {"now": 1_000.0}
    book = FakeBook([(0, "Titles", titles_grid())])
    service, server, stub = start(
        opener=lambda _path: book,
        clock=lambda: clock["now"],
        sweep_seconds=0.05,
    )
    try:
        view = upload(stub, "book.xlsx")
        clock["now"] += IDLE_SECONDS + 1
        deadline = time.monotonic() + 2
        while view.session_id in service.sessions and time.monotonic() < deadline:
            time.sleep(0.05)
    finally:
        server.stop(1)
    assert view.session_id not in service.sessions
    assert book.closed


def test_session_cap_closes_the_oldest_idle_workbook():
    clock = {"now": 1.0}
    books = []

    def opener(_path):
        book = FakeBook([(0, "Titles", titles_grid())])
        books.append(book)
        return book

    service, server, stub = start(opener=opener, clock=lambda: clock["now"], max_sessions=2)
    try:
        first = upload(stub, "a.xlsx", b"a")
        clock["now"] = 2
        second = upload(stub, "b.xlsx", b"b")
        clock["now"] = 3
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=first.session_id, excel_row=4))
        clock["now"] = 4
        third = upload(stub, "c.xlsx", b"c")
    finally:
        server.stop(1)
    assert first.session_id in service.sessions
    assert second.session_id not in service.sessions
    assert third.session_id in service.sessions
    assert books[1].closed
    assert not books[0].closed
    assert not books[2].closed


def test_session_cap_leaves_a_busy_workbook_open():
    busy = FakeBook([(0, "Titles", titles_grid())])
    busy.hold_stream = True
    opened = []

    def opener(_path):
        book = busy if not opened else FakeBook([(0, "Titles", titles_grid())])
        opened.append(book)
        return book

    service, server, stub = start(opener=opener, max_sessions=1)
    errors = []
    try:
        view = upload(stub, "busy.xlsx", b"busy")
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4))

        def read():
            try:
                collect(stub, view.session_id)
            except Exception as exc:
                errors.append(exc)

        reader = threading.Thread(target=read)
        reader.start()
        assert busy.stream_started.wait(2)
        code = status(lambda: upload(stub, "next.xlsx", b"next"))
        assert view.session_id in service.sessions
        assert not busy.closed
    finally:
        busy.stream_release.set()
        reader.join(2)
        server.stop(1)
    assert code == grpc.StatusCode.RESOURCE_EXHAUSTED
    assert errors == []
    assert opened[1].closed


def test_session_cap_comes_from_the_environment(monkeypatch):
    monkeypatch.setenv("EXCEL_MAX_SESSIONS", "1")
    books = []

    def opener(_path):
        book = FakeBook([(0, "Titles", titles_grid())])
        books.append(book)
        return book

    service, server, stub = start(opener=opener)
    try:
        first = upload(stub, "a.xlsx", b"a")
        second = upload(stub, "b.xlsx", b"b")
    finally:
        server.stop(1)
    assert first.session_id not in service.sessions
    assert second.session_id in service.sessions
    assert books[0].closed


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
