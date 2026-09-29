"""gRPC handlers."""

import os
import tempfile
import threading
import time
import uuid
from concurrent import futures
from dataclasses import dataclass
from pathlib import Path

import grpc
from grpc_reflection.v1alpha import reflection

from excel_tooling.columns import excel_id
from excel_tooling.model import CellValue, Column
from excel_tooling.scan import DuplicateColumn, HeaderNotCandidate, SubcolumnRowOutOfRange
from excel_tooling.workbook import BadFile, NoData, SheetNotListed

import excel_tooling  # noqa: F401  (puts generated stubs on sys.path)
from irbis.excel.v1 import excel_pb2, excel_pb2_grpc
from irbis.table.v1 import table_pb2

MAX_MESSAGE_BYTES = 64 * 1024 * 1024
IDLE_SECONDS = 15 * 60
BATCH_ROWS = 100
_ALLOWED = {".xlsx", ".xlsm", ".xls", ".ods"}


@dataclass
class Session:
    session_id: str
    path: Path
    book: object
    limit: int
    selected_sheet_index: int
    candidates: tuple
    header_row: int | None
    subcolumn_row: int | None
    last_used: float


class ExcelToolingService(excel_pb2_grpc.ExcelToolingServicer):
    def __init__(self, opener=None, clock=None):
        from excel_tooling import workbook

        self._opener = opener or workbook.open
        self._clock = clock or time.monotonic
        self._lock = threading.Lock()
        self.sessions: dict[str, Session] = {}

    def OpenWorkbook(self, request_iterator, context):
        with self._lock:
            meta, payload = _upload(request_iterator, context)
            if meta is None:
                return excel_pb2.SessionView()
            limit = _limit(meta.header_candidate_limit)
            if limit is None:
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, "header_candidate_limit")
            suffix = Path(meta.filename).suffix.lower()
            if suffix not in _ALLOWED:
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, suffix or "filename")
            path = _write_temp(suffix, payload)
            try:
                book = self._opener(path)
            except BadFile as exc:
                path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(exc))
            except NoData:
                path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.FAILED_PRECONDITION, "no data")
            except Exception:
                path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.INTERNAL, "calc")
            session = Session(
                session_id=str(uuid.uuid4()),
                path=path,
                book=book,
                limit=limit,
                selected_sheet_index=book.selected_sheet_index,
                candidates=book.candidates(book.selected_sheet_index, limit),
                header_row=None,
                subcolumn_row=None,
                last_used=self._clock(),
            )
            self.sessions[session.session_id] = session
            return _view(session)

    def SelectSheet(self, request, context):
        with self._lock:
            session = self._session(request.session_id, context)
            limit = _limit(request.header_candidate_limit)
            if limit is None:
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, "header_candidate_limit")
            try:
                candidates = session.book.candidates(request.sheet_index, limit)
            except SheetNotListed:
                context.abort(grpc.StatusCode.FAILED_PRECONDITION, "sheet")
            session.limit = limit
            session.selected_sheet_index = request.sheet_index
            session.candidates = candidates
            session.header_row = None
            session.subcolumn_row = None
            session.last_used = self._clock()
            return _view(session)

    def SelectHeader(self, request, context):
        with self._lock:
            session = self._session(request.session_id, context)
            subcolumn_row = request.subcolumn_row or None
            try:
                columns = session.book.select_header(
                    session.selected_sheet_index,
                    request.excel_row,
                    session.limit,
                    subcolumn_row,
                )
            except (DuplicateColumn, HeaderNotCandidate, SubcolumnRowOutOfRange):
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, "header")
            session.header_row = request.excel_row
            session.subcolumn_row = subcolumn_row
            session.last_used = self._clock()
            return _table(columns, ())

    def ReadTable(self, request, context):
        with self._lock:
            session = self._session(request.session_id, context)
            if session.header_row is None:
                context.abort(grpc.StatusCode.FAILED_PRECONDITION, "header")
            blocks = session.book.iter_blocks(
                session.selected_sheet_index,
                session.header_row,
                session.limit,
                session.subcolumn_row,
            )
        while True:
            with self._lock:
                try:
                    block = next(blocks)
                except StopIteration:
                    break
                except Exception:
                    context.abort(grpc.StatusCode.INTERNAL, "calc")
                session.last_used = self._clock()
            for batch in _batches(block):
                yield batch

    def Close(self, request, context):
        with self._lock:
            session = self._session(request.session_id, context)
            self._drop(session)
            return excel_pb2.CloseResponse()

    def _session(self, session_id: str, context) -> Session:
        session = self.sessions.get(session_id)
        if session is not None and self._clock() - session.last_used > IDLE_SECONDS:
            self._drop(session)
            session = None
        if session is None:
            context.abort(grpc.StatusCode.NOT_FOUND, "unknown session")
        return session

    def _drop(self, session: Session) -> None:
        self.sessions.pop(session.session_id, None)
        try:
            session.book.close()
        finally:
            session.path.unlink(missing_ok=True)


def serve(address: str = "0.0.0.0:50051", opener=None, clock=None):
    service = ExcelToolingService(opener=opener, clock=clock)
    server = grpc.server(
        futures.ThreadPoolExecutor(max_workers=8),
        options=[
            ("grpc.max_receive_message_length", MAX_MESSAGE_BYTES),
            ("grpc.max_send_message_length", MAX_MESSAGE_BYTES),
        ],
    )
    excel_pb2_grpc.add_ExcelToolingServicer_to_server(service, server)
    reflection.enable_server_reflection(
        (
            excel_pb2.DESCRIPTOR.services_by_name["ExcelTooling"].full_name,
            reflection.SERVICE_NAME,
        ),
        server,
    )
    port = server.add_insecure_port(address)
    server.start()
    return server, port, service


def _upload(request_iterator, context):
    chunks = iter(request_iterator)
    try:
        first = next(chunks)
    except StopIteration:
        context.abort(grpc.StatusCode.INVALID_ARGUMENT, "empty upload")
    if first.WhichOneof("part") != "meta":
        context.abort(grpc.StatusCode.INVALID_ARGUMENT, "meta")
    payload = bytearray()
    for chunk in chunks:
        if chunk.WhichOneof("part") != "data":
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "data")
        payload.extend(chunk.data)
    return first.meta, bytes(payload)


def _limit(value: int) -> int | None:
    if value == 0:
        return 10
    if 1 <= value <= 50:
        return value
    return None


def _write_temp(suffix: str, payload: bytes) -> Path:
    handle, name = tempfile.mkstemp(suffix=suffix)
    os.close(handle)
    path = Path(name)
    path.write_bytes(payload)
    return path


def _view(session: Session) -> excel_pb2.SessionView:
    view = excel_pb2.SessionView(session_id=session.session_id)
    view.selected_sheet_index = session.selected_sheet_index
    for sheet in session.book.sheets():
        info = view.sheets.add()
        info.name = sheet.name
        info.index = sheet.index
    for candidate in session.candidates:
        item = view.header_candidates.add()
        item.excel_row = candidate.excel_row
        for column in candidate.columns:
            item.columns.append(_column_message(column))
    return view


def _batches(rows):
    batch: list = []
    for row in rows:
        batch.append(row)
        if len(batch) == BATCH_ROWS:
            yield _row_batch(batch)
            batch = []
    if batch:
        yield _row_batch(batch)


def _row_batch(rows) -> excel_pb2.RowBatch:
    message = excel_pb2.RowBatch()
    for row in rows:
        out = message.rows.add()
        out.excel_row = row.excel_row
        for cell in row.cells:
            if cell.value is None:
                continue
            item = out.cells.add()
            item.excel_id = excel_id(cell.col)
            item.value.CopyFrom(_value_message(cell.value))
    return message


def _table(columns, rows) -> table_pb2.Table:
    table = table_pb2.Table()
    for column in columns:
        table.columns.append(_column_message(column))
    for row in rows:
        message = table.rows.add()
        message.excel_row = row.excel_row
        for cell in row.cells:
            if cell.value is None:
                continue
            out = message.cells.add()
            out.excel_id = excel_id(cell.col)
            out.value.CopyFrom(_value_message(cell.value))
    return table


def _column_message(column: Column) -> table_pb2.Column:
    message = table_pb2.Column(excel_id=column.excel_id, name=column.name)
    for sub in column.subcolumns:
        message.subcolumns.append(_column_message(sub))
    return message


def _value_message(value: CellValue) -> table_pb2.Value:
    message = table_pb2.Value()
    if value.date is not None:
        message.date.year = value.date.year
        message.date.month = value.date.month
        message.date.day = value.date.day
        message.date.hour = value.date.hour
        message.date.minute = value.date.minute
        message.date.second = value.date.second
        message.date.has_time = value.date.has_time
    elif value.bool_value is not None:
        message.bool_value = value.bool_value
    elif value.int_value is not None:
        message.int_value = value.int_value
    elif value.float_value is not None:
        message.float_value = value.float_value
    elif value.text is not None:
        message.text = value.text
    return message
