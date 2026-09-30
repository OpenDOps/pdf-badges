"""gRPC handlers."""

import inspect
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
from excel_tooling.password import needs_password
from excel_tooling.scan import DuplicateColumn, HeaderNotCandidate, SubcolumnRowOutOfRange
from excel_tooling.workbook import BadFile, NoData, PasswordRequired, SheetNotListed

import excel_tooling  # noqa: F401  (puts generated stubs on sys.path)
from irbis.excel.v1 import excel_pb2, excel_pb2_grpc
from irbis.table.v1 import table_pb2

MAX_UPLOAD_BYTES = 64 * 1024 * 1024
MAX_MESSAGE_BYTES = MAX_UPLOAD_BYTES
IDLE_SECONDS = 15 * 60
PASSWORD_SECONDS = 60
SWEEP_SECONDS = 60
MAX_SESSIONS = 100
BATCH_ROWS = 100
_ALLOWED = {".xlsx", ".xlsm", ".xls", ".ods"}


@dataclass
class PendingPassword:
    session_id: str
    path: Path
    limit: int
    created: float


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
    calc: threading.Lock
    closed: bool = False


class SessionCap(Exception):
    pass


class ExcelToolingService(excel_pb2_grpc.ExcelToolingServicer):
    def __init__(self, opener=None, clock=None, sweep_seconds=None, max_sessions=None):
        from excel_tooling import workbook

        self._opener = opener or workbook.open
        self._clock = clock or time.monotonic
        self._sweep_seconds = SWEEP_SECONDS if sweep_seconds is None else sweep_seconds
        self._max_sessions = _session_cap(max_sessions)
        self._sessions_lock = threading.Lock()
        self._stopped = threading.Event()
        self.sessions: dict[str, Session] = {}
        self.pending: dict[str, PendingPassword] = {}
        self._sweeper = threading.Thread(target=self._run_sweeper, name="session-sweep", daemon=True)
        self._sweeper.start()

    def shutdown(self) -> None:
        self._stopped.set()
        self._sweeper.join(timeout=1)
        with self._sessions_lock:
            pending = list(self.pending.values())
            self.pending.clear()
        for item in pending:
            item.path.unlink(missing_ok=True)

    def OpenWorkbook(self, request_iterator, context):
        self._sweep()
        meta, path = _store_upload(request_iterator, context)
        limit = _limit(meta.header_candidate_limit)
        password = meta.password
        if not password and needs_password(path):
            return self._park(path, limit)
        calc = threading.Lock()
        password_needed = False
        with calc:
            try:
                book = _invoke_opener(self._opener, path, password)
            except PasswordRequired:
                password_needed = True
                book = None
            except BadFile as exc:
                path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(exc))
            except NoData:
                path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.FAILED_PRECONDITION, "no data")
            except Exception:
                path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.INTERNAL, "calc")
            if not password_needed:
                try:
                    candidates = book.candidates(book.selected_sheet_index, limit)
                except Exception:
                    _discard(book, path)
                    context.abort(grpc.StatusCode.INTERNAL, "calc")
                try:
                    session = self._remember(path, book, limit, candidates, calc)
                except SessionCap:
                    _discard(book, path)
                    context.abort(grpc.StatusCode.RESOURCE_EXHAUSTED, "sessions")
                return _view(session)
        if password:
            path.unlink(missing_ok=True)
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "password")
        return self._park(path, limit)

    def ProvidePassword(self, request, context):
        self._sweep()
        if not request.password:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "password")
        with self._sessions_lock:
            if request.session_id in self.sessions:
                context.abort(grpc.StatusCode.FAILED_PRECONDITION, "password")
            pending = self.pending.pop(request.session_id, None)
        if pending is None:
            context.abort(grpc.StatusCode.NOT_FOUND, "unknown session")
        if self._clock() - pending.created >= PASSWORD_SECONDS:
            pending.path.unlink(missing_ok=True)
            context.abort(grpc.StatusCode.NOT_FOUND, "unknown session")
        calc = threading.Lock()
        with calc:
            try:
                book = _invoke_opener(self._opener, pending.path, request.password)
            except PasswordRequired:
                self._restore_pending(pending)
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, "password")
            except BadFile as exc:
                pending.path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(exc))
            except NoData:
                pending.path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.FAILED_PRECONDITION, "no data")
            except Exception:
                pending.path.unlink(missing_ok=True)
                context.abort(grpc.StatusCode.INTERNAL, "calc")
            try:
                candidates = book.candidates(book.selected_sheet_index, pending.limit)
            except Exception:
                _discard(book, pending.path)
                context.abort(grpc.StatusCode.INTERNAL, "calc")
            try:
                session = self._remember(
                    pending.path, book, pending.limit, candidates, calc, pending.session_id
                )
            except SessionCap:
                _discard(book, pending.path)
                context.abort(grpc.StatusCode.RESOURCE_EXHAUSTED, "sessions")
            return _view(session)

    def SelectSheet(self, request, context):
        limit = _limit(request.header_candidate_limit)
        if limit is None:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "header_candidate_limit")
        session = self._borrow(request.session_id, context)
        with session.calc:
            self._ensure_open(session, context)
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
        session = self._borrow(request.session_id, context)
        with session.calc:
            self._ensure_open(session, context)
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
        session = self._borrow(request.session_id, context)
        if request.background:
            if request.channel != "queue":
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, "channel")
            self._start_background(session, context)
            return
        if request.channel:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "channel")
        with session.calc:
            self._ensure_open(session, context)
            if session.header_row is None:
                context.abort(grpc.StatusCode.FAILED_PRECONDITION, "header")
            blocks = session.book.iter_blocks(
                session.selected_sheet_index,
                session.header_row,
                session.limit,
                session.subcolumn_row,
            )
            try:
                yield from _walk(session, blocks, context.is_active, self._clock)
            except Exception:
                context.abort(grpc.StatusCode.INTERNAL, "calc")

    def _start_background(self, session: Session, context) -> None:
        with session.calc:
            self._ensure_open(session, context)
            if session.header_row is None:
                context.abort(grpc.StatusCode.FAILED_PRECONDITION, "header")
            sheet = session.selected_sheet_index
            header = session.header_row
            limit = session.limit
            subcolumn = session.subcolumn_row
        threading.Thread(
            target=self._background_read,
            args=(session, sheet, header, limit, subcolumn),
            name="read-table",
            daemon=True,
        ).start()

    def _background_read(self, session: Session, sheet, header, limit, subcolumn) -> None:
        with session.calc:
            if session.closed or session.header_row is None:
                return
            try:
                blocks = session.book.iter_blocks(sheet, header, limit, subcolumn)
                for batch in _walk(session, blocks, lambda: True, self._clock):
                    _QUEUE.write(batch)
            except Exception:
                return

    def Close(self, request, context):
        self._sweep()
        with self._sessions_lock:
            session = self.sessions.pop(request.session_id, None)
            pending = self.pending.pop(request.session_id, None)
        if session is None and pending is None:
            context.abort(grpc.StatusCode.NOT_FOUND, "unknown session")
        if session is not None:
            self._close_book(session)
        if pending is not None:
            pending.path.unlink(missing_ok=True)
        return excel_pb2.CloseResponse()

    def _borrow(self, session_id: str, context) -> Session:
        self._sweep()
        with self._sessions_lock:
            session = self.sessions.get(session_id)
        if session is None:
            context.abort(grpc.StatusCode.NOT_FOUND, "unknown session")
        return session

    def _run_sweeper(self) -> None:
        while not self._stopped.wait(self._sweep_seconds):
            self._sweep()

    def _sweep(self) -> None:
        now = self._clock()
        with self._sessions_lock:
            expired = [
                session
                for session in self.sessions.values()
                if now - session.last_used > IDLE_SECONDS
            ]
            for session in expired:
                self.sessions.pop(session.session_id, None)
            expired_pending = [
                item
                for item in self.pending.values()
                if now - item.created >= PASSWORD_SECONDS
            ]
            for item in expired_pending:
                self.pending.pop(item.session_id, None)
        for session in expired:
            self._close_book(session)
        for item in expired_pending:
            item.path.unlink(missing_ok=True)

    def _park(self, path: Path, limit: int) -> excel_pb2.SessionView:
        pending = PendingPassword(
            session_id=str(uuid.uuid4()),
            path=path,
            limit=limit,
            created=self._clock(),
        )
        with self._sessions_lock:
            self.pending[pending.session_id] = pending
        return excel_pb2.SessionView(session_id=pending.session_id, password_required=True)

    def _restore_pending(self, pending: PendingPassword) -> None:
        if self._clock() - pending.created >= PASSWORD_SECONDS:
            pending.path.unlink(missing_ok=True)
            return
        with self._sessions_lock:
            self.pending[pending.session_id] = pending

    def _remember(self, path, book, limit, candidates, calc, session_id=None) -> Session:
        self._make_room()
        session = Session(
            session_id=session_id or str(uuid.uuid4()),
            path=path,
            book=book,
            limit=limit,
            selected_sheet_index=book.selected_sheet_index,
            candidates=candidates,
            header_row=None,
            subcolumn_row=None,
            last_used=self._clock(),
            calc=calc,
        )
        with self._sessions_lock:
            self.sessions[session.session_id] = session
        return session

    def _make_room(self) -> None:
        with self._sessions_lock:
            if len(self.sessions) < self._max_sessions:
                return
            idle = [item for item in self.sessions.values() if not item.calc.locked()]
            if not idle:
                raise SessionCap()
            victim = min(idle, key=lambda item: item.last_used)
            self.sessions.pop(victim.session_id, None)
        self._close_book(victim)

    def _ensure_open(self, session: Session, context) -> None:
        if session.closed:
            context.abort(grpc.StatusCode.NOT_FOUND, "unknown session")

    def _close_book(self, session: Session) -> None:
        with session.calc:
            if session.closed:
                return
            session.closed = True
            _discard(session.book, session.path)


def serve(address: str = "0.0.0.0:50051", opener=None, clock=None, sweep_seconds=None, max_sessions=None):
    service = ExcelToolingService(
        opener=opener, clock=clock, sweep_seconds=sweep_seconds, max_sessions=max_sessions
    )
    server = grpc.server(
        futures.ThreadPoolExecutor(max_workers=8),
        options=[
            ("grpc.max_receive_message_length", MAX_MESSAGE_BYTES),
            ("grpc.max_send_message_length", MAX_MESSAGE_BYTES),
        ],
    )
    excel_pb2_grpc.add_ExcelToolingServicer_to_server(service, server)
    if _loopback(address):
        reflection.enable_server_reflection(
            (
                excel_pb2.DESCRIPTOR.services_by_name["ExcelTooling"].full_name,
                reflection.SERVICE_NAME,
            ),
            server,
        )
    port = server.add_insecure_port(address)
    server.start()
    return _ServiceServer(server, service), port, service


class _ServiceServer:
    def __init__(self, server, service: ExcelToolingService):
        self._server = server
        self._service = service

    def stop(self, grace=None):
        self._service.shutdown()
        return self._server.stop(grace)

    def __getattr__(self, name):
        return getattr(self._server, name)


def _loopback(address: str) -> bool:
    host, _sep, _port = address.rpartition(":")
    return host.strip("[]") in {"127.0.0.1", "localhost", "::1"}


def _session_cap(value) -> int:
    raw = os.environ.get("EXCEL_MAX_SESSIONS", str(MAX_SESSIONS)) if value is None else value
    try:
        parsed = int(raw)
    except (TypeError, ValueError):
        return MAX_SESSIONS
    return parsed if parsed >= 1 else MAX_SESSIONS


def _invoke_opener(opener, path: Path, password: str):
    if password and _takes_password(opener):
        return opener(path, password)
    return opener(path)


def _takes_password(opener) -> bool:
    try:
        params = list(inspect.signature(opener).parameters.values())
    except (TypeError, ValueError):
        return False
    if any(item.kind is inspect.Parameter.VAR_POSITIONAL for item in params):
        return True
    positional = (
        inspect.Parameter.POSITIONAL_ONLY,
        inspect.Parameter.POSITIONAL_OR_KEYWORD,
    )
    return sum(item.kind in positional for item in params) >= 2


def _store_upload(request_iterator, context):
    chunks = iter(request_iterator)
    try:
        first = next(chunks)
    except StopIteration:
        context.abort(grpc.StatusCode.INVALID_ARGUMENT, "empty upload")
    if first.WhichOneof("part") != "meta":
        context.abort(grpc.StatusCode.INVALID_ARGUMENT, "meta")
    meta = first.meta
    suffix = Path(meta.filename).suffix.lower()
    if suffix not in _ALLOWED:
        context.abort(grpc.StatusCode.INVALID_ARGUMENT, suffix or "filename")
    if _limit(meta.header_candidate_limit) is None:
        context.abort(grpc.StatusCode.INVALID_ARGUMENT, "header_candidate_limit")
    handle, name = tempfile.mkstemp(suffix=suffix)
    path = Path(name)
    try:
        with os.fdopen(handle, "wb") as stored:
            total = 0
            for chunk in chunks:
                if chunk.WhichOneof("part") != "data":
                    context.abort(grpc.StatusCode.INVALID_ARGUMENT, "data")
                size = len(chunk.data)
                if total > MAX_UPLOAD_BYTES - size:
                    context.abort(grpc.StatusCode.RESOURCE_EXHAUSTED, "upload")
                total += size
                stored.write(chunk.data)
    except Exception:
        path.unlink(missing_ok=True)
        raise
    return meta, path


def _discard(book, path: Path) -> None:
    try:
        book.close()
    finally:
        path.unlink(missing_ok=True)


def _limit(value: int) -> int | None:
    if value == 0:
        return 10
    if 1 <= value <= 50:
        return value
    return None


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


class _QueueChannel:
    """Background output. `queue` discards each batch until a real queue exists."""

    def write(self, _batch) -> None:
        return


_QUEUE = _QueueChannel()


def _walk(session: Session, blocks, active, clock):
    while active():
        try:
            block = next(blocks)
        except StopIteration:
            return
        session.last_used = clock()
        for batch in _batches(block):
            if not active():
                return
            yield batch


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
