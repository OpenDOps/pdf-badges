import threading
import time
from pathlib import Path

import grpc
import pytest

from excel_tooling.model import CellValue, Column, DataRow, Grid, GridCell, SheetInfo
from excel_tooling.scan import candidate_grid, columns_for_header, header_candidates, walk_blocks
from excel_tooling.server import _walk, serve
from grpc_reflection.v1alpha import reflection_pb2, reflection_pb2_grpc

import excel_tooling  # noqa: F401
from irbis.excel.v1 import excel_pb2, excel_pb2_grpc

FIXTURES = Path(__file__).resolve().parent / "fixtures"
CHANNEL_OPTIONS = [
    ("grpc.max_receive_message_length", 64 * 1024 * 1024),
    ("grpc.max_send_message_length", 64 * 1024 * 1024),
]


class RecordingSource:
    def __init__(self, last_row, rows=None, fill=False):
        self.last_row = last_row
        self.rows = rows or {}
        self.fill = fill
        self.asked = []
        self.held = None

    def bounds(self):
        return (1, self.last_row, 0, 0)

    def read(self, first, last, columns=None):
        if columns is None:
            self.asked.append((first, last))
        else:
            self.asked.append((first, last, tuple(columns)))
        if self.fill:
            self.held = {
                row: [GridCell(0, "x", CellValue(text="x"))]
                for row in range(first, last + 1)
            }
        else:
            self.held = {
                row: self.rows[row] for row in range(first, last + 1) if row in self.rows
            }
        return self.held

    def release(self):
        self.held = None

    def nonempty_rows(self, k):
        if self.fill:
            rows = range(1, self.last_row + 1)
        else:
            rows = sorted(row for row in self.rows if 1 <= row <= self.last_row)
        produced = 0
        for row in rows:
            if produced == k:
                return
            produced += 1
            yield row


def named(row, label):
    return {row: [GridCell(0, label, CellValue(text=label))]}


def test_candidates_do_not_scan_the_sheet():
    rows = {}
    rows.update(named(1, "One"))
    rows.update(named(2, "Two"))
    rows.update(named(3, "Three"))
    rows.update(named(5000, "Later"))
    source = RecordingSource(5000, rows)
    grid = candidate_grid(source, 3)
    assert [row for row, _last in source.asked] == [1, 2, 3]
    assert all(last < 5000 for _row, last in source.asked)
    assert [candidate.excel_row for candidate in header_candidates(grid, 3)] == [1, 2, 3]


def test_candidates_skip_a_large_empty_span():
    source = RecordingSource(
        1_048_576,
        {**named(2, "Name"), **named(100, "Later")},
    )
    grid = candidate_grid(source, 10)
    assert source.asked == [(2, 2), (100, 100)]
    assert [candidate.excel_row for candidate in header_candidates(grid, 10)] == [2, 100]
    stopped = RecordingSource(1_048_576, {**named(2, "Name"), **named(100, "Later")})
    candidate_grid(stopped, 1)
    assert stopped.asked == [(2, 2)]


def test_candidate_prefix_is_reused():
    from excel_tooling.model import SheetInfo
    from excel_tooling.workbook import Workbook

    source = RecordingSource(20, {**named(1, "Name"), **named(2, "City"), **named(4, "Later")})
    book = Workbook(None)
    book._listed = (SheetInfo(0, "Titles"),)
    book._sources[0] = source
    book.selected_sheet_index = 0
    assert [item.excel_row for item in book.candidates(0, 2)] == [1, 2]
    scanned = list(source.asked)
    book.select_header(0, 1, 2)
    columns = book.select_header(0, 1, 2, subcolumn_row=2)
    list(book.iter_blocks(0, 1, 2, subcolumn_row=2))
    assert source.asked[: len(scanned)] == scanned
    assert all(len(item) == 3 for item in source.asked[len(scanned) :])
    assert [column.name for column in columns] == ["Name"]
    again = len(source.asked)
    book.candidates(0, 3)
    assert len(source.asked) > again


def test_blocks_read_only_leaf_columns():
    source = RecordingSource(10, fill=True)
    parent = Column(0, "A", "Group", (Column(0, "A", "One"), Column(2, "C", "Two")))
    far = Column(50, "AY", "Far")
    list(walk_blocks(source, 1, 10, (parent, far)))
    assert source.asked == [(1, 10, (0, 2, 50))]


def test_column_spans_skip_gaps():
    from excel_tooling.workbook import _column_spans

    assert _column_spans((50, 0, 2, 3)) == ((0, 0), (2, 3), (50, 50))


def test_calc_blocks_of_2000():
    source = RecordingSource(2500, fill=True)
    columns = (Column(0, "A", "Name"),)
    blocks = walk_blocks(source, 1, 2500, columns)
    first = next(blocks)
    retained = first
    assert source.held is None
    assert source.asked == [(1, 2000, (0,))]
    assert [row.excel_row for row in retained] == list(range(1, 2001))
    second = next(blocks)
    assert source.held is None
    assert source.asked == [(1, 2000, (0,)), (2001, 2500, (0,))]
    assert [row.excel_row for row in second] == list(range(2001, 2501))
    with pytest.raises(StopIteration):
        next(blocks)


def test_gap_inside_a_block():
    source = RecordingSource(
        2500,
        {
            2: [GridCell(0, "Keep", CellValue(text="Keep"))],
            2100: [GridCell(0, "Drop", CellValue(text="Drop"))],
        },
    )
    columns = (Column(0, "A", "Name"),)
    blocks = list(walk_blocks(source, 2, 2500, columns))
    assert source.asked == [(2, 2001, (0,))]
    assert [row.excel_row for block in blocks for row in block] == [2]


class BatchBook:
    def __init__(self):
        self.selected_sheet_index = 0

    def sheets(self):
        return (SheetInfo(0, "Long"),)

    def candidates(self, sheet_index, k):
        grid = Grid(1, 1, 0, 0, {1: [GridCell(0, "Name", CellValue(text="Name"))]})
        return header_candidates(grid, k)

    def select_header(self, sheet_index, excel_row, k, subcolumn_row=None):
        grid = Grid(1, 1, 0, 0, {1: [GridCell(0, "Name", CellValue(text="Name"))]})
        return columns_for_header(grid, excel_row, self.candidates(sheet_index, k), subcolumn_row)

    def iter_blocks(self, sheet_index, excel_row, k, subcolumn_row=None):
        yield [
            DataRow(index, (GridCell(0, str(index), CellValue(text=str(index))),))
            for index in range(1, 251)
        ]

    def close(self):
        return None


def test_reflection_lists_service_on_a_local_port():
    server, port, _service = serve("127.0.0.1:0", opener=lambda _path: None)
    channel = grpc.insecure_channel(f"127.0.0.1:{port}", options=CHANNEL_OPTIONS)
    try:
        stub = reflection_pb2_grpc.ServerReflectionStub(channel)
        request = reflection_pb2.ServerReflectionRequest(list_services="*")
        responses = list(stub.ServerReflectionInfo(iter([request])))
    finally:
        channel.close()
        server.stop(1)
    names = [item.name for item in responses[0].list_services_response.service]
    assert "irbis.excel.v1.ExcelTooling" in names


def test_reflection_is_off_on_the_service_bind():
    server, port, _service = serve("0.0.0.0:0", opener=lambda _path: None)
    channel = grpc.insecure_channel(f"127.0.0.1:{port}", options=CHANNEL_OPTIONS)
    try:
        stub = reflection_pb2_grpc.ServerReflectionStub(channel)
        request = reflection_pb2.ServerReflectionRequest(list_services="*")
        with pytest.raises(grpc.RpcError) as caught:
            list(stub.ServerReflectionInfo(iter([request])))
    finally:
        channel.close()
        server.stop(1)
    assert caught.value.code() == grpc.StatusCode.UNIMPLEMENTED


class _Active:
    def __init__(self):
        self.flag = True

    def __call__(self):
        return self.flag


def test_walk_stops_when_the_call_is_cancelled():
    pulls = {"n": 0}

    def blocks():
        pulls["n"] += 1
        yield [DataRow(1, (GridCell(0, "a", CellValue(text="a")),))]
        pulls["n"] += 1
        yield [DataRow(2, (GridCell(0, "b", CellValue(text="b")),))]

    active = _Active()
    session = type("S", (), {"last_used": 0})()

    def clock():
        return 1

    sent = []
    for batch in _walk(session, blocks(), active, clock):
        sent.append(batch)
        active.flag = False
    assert len(sent) == 1
    assert pulls["n"] == 1


class PauseBook:
    def __init__(self):
        self.selected_sheet_index = 0
        self.pulls = 0
        self.entered_second = threading.Event()
        self.hold = threading.Event()
        self.done = threading.Event()

    def sheets(self):
        return (SheetInfo(0, "Sheet"),)

    def candidates(self, sheet_index, k):
        grid = Grid(1, 1, 0, 0, {1: [GridCell(0, "Name", CellValue(text="Name"))]})
        return header_candidates(grid, k)

    def select_header(self, sheet_index, excel_row, k, subcolumn_row=None):
        return (Column(0, "A", "Name"),)

    def iter_blocks(self, sheet_index, excel_row, k, subcolumn_row=None):
        self.pulls += 1
        yield [DataRow(1, (GridCell(0, "a", CellValue(text="a")),))]
        self.entered_second.set()
        assert self.hold.wait(2)
        self.pulls += 1
        yield [DataRow(2, (GridCell(0, "b", CellValue(text="b")),))]
        self.pulls += 1
        yield [DataRow(3, (GridCell(0, "c", CellValue(text="c")),))]
        self.done.set()

    def close(self):
        return None


def test_cancel_does_not_read_further_blocks():
    book = PauseBook()
    _service, server, stub = _start(opener=lambda _path: book)
    try:
        view = _upload(stub, "book.xlsx")
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=1))
        call = stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id))
        received = []

        def consume():
            try:
                for batch in call:
                    received.append(batch)
            except grpc.RpcError:
                return

        reader = threading.Thread(target=consume)
        reader.start()
        assert book.entered_second.wait(2)
        call.cancel()
        time.sleep(0.2)
        book.hold.set()
        reader.join(2)
    finally:
        book.hold.set()
        server.stop(1)
    assert not reader.is_alive()
    assert book.pulls == 2
    assert len(received) <= 1
    assert not book.done.is_set()


def test_background_queue_runs_after_the_rpc_returns():
    book = PauseBook()
    _service, server, stub = _start(opener=lambda _path: book)
    try:
        view = _upload(stub, "book.xlsx")
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=1))
        call = stub.ReadTable(
            excel_pb2.ReadTableRequest(
                session_id=view.session_id, background=True, channel="queue"
            )
        )
        assert list(call) == []
        assert book.entered_second.wait(2)
        book.hold.set()
        assert book.done.wait(2)
    finally:
        book.hold.set()
        server.stop(1)
    assert book.pulls == 3


def test_background_channel_must_be_queue():
    book = PauseBook()
    _service, server, stub = _start(opener=lambda _path: book)
    try:
        view = _upload(stub, "book.xlsx")
        missing = excel_pb2.ReadTableRequest(session_id=view.session_id, background=True)
        other = excel_pb2.ReadTableRequest(
            session_id=view.session_id, background=True, channel="file"
        )
        plain = excel_pb2.ReadTableRequest(session_id=view.session_id, channel="queue")
        codes = []
        for request in (missing, other, plain):
            with pytest.raises(grpc.RpcError) as caught:
                list(stub.ReadTable(request))
            codes.append(caught.value.code())
    finally:
        server.stop(1)
    assert codes == [grpc.StatusCode.INVALID_ARGUMENT] * 3


def test_batch_size_100():
    _service, server, stub = _start(opener=lambda _path: BatchBook())
    try:
        view = _upload(stub, "book.xlsx")
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=1))
        batches = list(stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id)))
    finally:
        server.stop(1)
    assert [len(batch.rows) for batch in batches] == [100, 100, 50]
    assert [row.excel_row for batch in batches for row in batch.rows] == list(range(1, 251))


def _start(opener=None):
    server, port, service = serve("127.0.0.1:0", opener=opener)
    channel = grpc.insecure_channel(f"127.0.0.1:{port}", options=CHANNEL_OPTIONS)
    return service, server, excel_pb2_grpc.ExcelToolingStub(channel)


def _upload(stub, filename, payload=b"book", limit=0):
    def chunks():
        yield excel_pb2.OpenChunk(
            meta=excel_pb2.OpenMeta(filename=filename, header_candidate_limit=limit)
        )
        yield excel_pb2.OpenChunk(data=payload)

    return stub.OpenWorkbook(chunks())


def _cell(row, excel_id):
    return next(cell for cell in row.cells if cell.excel_id == excel_id)


@pytest.mark.libreoffice
def test_titles_stream():
    _service, server, stub = _start()
    try:
        view = _upload(stub, "titles.xlsx", (FIXTURES / "titles.xlsx").read_bytes(), limit=10)
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4))
        batches = list(stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id)))
    finally:
        server.stop(1)
    assert len(batches) == 1
    assert [row.excel_row for row in batches[0].rows] == [5, 7]


@pytest.mark.libreoffice
def test_lingerie_stream():
    _service, server, stub = _start()
    try:
        view = _upload(stub, "JPGVL0I5VW.xlsm", (FIXTURES / "JPGVL0I5VW.xlsm").read_bytes(), limit=3)
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=2))
        batches = list(stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id)))
    finally:
        server.stop(1)
    rows = [row for batch in batches for row in batch.rows]
    assert len(rows) == 2924
    assert [len(batch.rows) for batch in batches[:-1]] == [100] * (len(batches) - 1)
    assert len(batches[-1].rows) == 24
    visitor = next(row for row in rows if row.excel_row == 4)
    assert _cell(visitor, "A").value.text == "JPGVL0I5VW_exp6"
    assert _cell(visitor, "E").value.text == "1712460"
    assert _cell(visitor, "AH").value.text == "02.09.2026"
