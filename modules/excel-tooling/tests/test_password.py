import sys
import threading
import zipfile
from pathlib import Path

import grpc

from excel_tooling.password import needs_password
from excel_tooling.server import PASSWORD_SECONDS
from excel_tooling.workbook import PasswordRequired

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_session import FakeBook, start, status, titles_grid  # noqa: E402

from irbis.excel.v1 import excel_pb2

_OLE = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"


def test_xlsx_encryption_is_an_ole_package(tmp_path):
    encrypted = tmp_path / "secret.xlsx"
    encrypted.write_bytes(_OLE + b"\x00" * 16)
    plain = tmp_path / "plain.xlsx"
    plain.write_bytes(b"PK\x03\x04" + b"\x00" * 16)
    assert needs_password(encrypted)
    assert not needs_password(plain)


def test_ods_encryption_is_in_the_manifest(tmp_path):
    encrypted = tmp_path / "secret.ods"
    plain = tmp_path / "plain.ods"
    _ods(encrypted, b"<manifest:encryption-data/>")
    _ods(plain, b"<manifest:file-entry/>")
    assert needs_password(encrypted)
    assert not needs_password(plain)


def test_password_on_open_unlocks_without_a_prompt():
    seen = {}

    def opener(path, password=None):
        seen["password"] = password
        seen["path"] = Path(path)
        return FakeBook([(0, "Titles", titles_grid())])

    _service, server, stub = start(opener=opener)
    try:
        view = _upload(stub, "secret.xlsx", _OLE + b"rest", password="secret")
    finally:
        server.stop(1)
    assert seen["password"] == "secret"
    assert not view.password_required
    assert [sheet.name for sheet in view.sheets] == ["Titles"]


def test_missing_password_waits_one_minute_without_a_lock(monkeypatch):
    clock = {"now": 1_000.0}
    plain = FakeBook([(0, "Titles", titles_grid())])
    plain.hold_stream = True
    unlocked = FakeBook([(0, "Secret", titles_grid())])
    opened = []

    def opener(path, password=None):
        payload = Path(path).read_bytes()
        opened.append(password)
        if payload.startswith(_OLE):
            if password != "secret":
                raise PasswordRequired()
            return unlocked
        return plain

    _track_tempfiles(monkeypatch)
    service, server, stub = start(opener=opener, clock=lambda: clock["now"])
    reader = None
    try:
        waiting = _upload(stub, "secret.xlsx", _OLE + b"rest")
        assert waiting.password_required
        assert waiting.sheets == []
        assert opened == []
        assert waiting.session_id in service.pending
        assert service.pending[waiting.session_id].path.exists()

        first = _upload(stub, "plain.xlsx", b"plain")
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=first.session_id, excel_row=4))
        reader = _start_stream(stub, first.session_id)
        assert plain.stream_started.wait(2)

        refused = status(
            lambda: stub.ProvidePassword(
                excel_pb2.ProvidePasswordRequest(session_id=waiting.session_id, password="nope")
            )
        )
        assert refused == grpc.StatusCode.INVALID_ARGUMENT
        assert waiting.session_id in service.pending
        assert not plain.stream_release.is_set()

        clock["now"] += PASSWORD_SECONDS - 1
        view = stub.ProvidePassword(
            excel_pb2.ProvidePasswordRequest(session_id=waiting.session_id, password="secret")
        )
        assert not view.password_required
        assert [sheet.name for sheet in view.sheets] == ["Secret"]
        assert waiting.session_id not in service.pending
        assert view.session_id in service.sessions
        assert not plain.stream_release.is_set()
    finally:
        plain.stream_release.set()
        if reader is not None:
            reader.join(2)
        server.stop(1)
    assert opened == [None, "nope", "secret"]
    assert reader is not None and not reader.is_alive()


def test_password_wait_expires_after_one_minute(monkeypatch):
    clock = {"now": 5_000.0}
    created = _track_tempfiles(monkeypatch)

    def opener(path, password=None):
        raise AssertionError("calc")

    service, server, stub = start(opener=opener, clock=lambda: clock["now"])
    try:
        waiting = _upload(stub, "secret.xlsx", _OLE + b"rest")
        stored = created[-1]
        clock["now"] += PASSWORD_SECONDS
        code = status(
            lambda: stub.ProvidePassword(
                excel_pb2.ProvidePasswordRequest(session_id=waiting.session_id, password="secret")
            )
        )
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.NOT_FOUND
    assert waiting.session_id not in service.pending
    assert waiting.session_id not in service.sessions
    assert not stored.exists()


def test_wrong_password_on_open_is_rejected(monkeypatch):
    created = _track_tempfiles(monkeypatch)

    def opener(path, password=None):
        raise PasswordRequired()

    service, server, stub = start(opener=opener)
    try:
        code = status(lambda: _upload(stub, "secret.xlsx", _OLE + b"rest", password="nope"))
    finally:
        server.stop(1)
    assert code == grpc.StatusCode.INVALID_ARGUMENT
    assert service.pending == {}
    assert service.sessions == {}
    assert not created[-1].exists()


def test_close_drops_a_password_wait(monkeypatch):
    created = _track_tempfiles(monkeypatch)
    service, server, stub = start(opener=lambda _path: (_ for _ in ()).throw(AssertionError("calc")))
    try:
        waiting = _upload(stub, "secret.xlsx", _OLE + b"rest")
        stub.Close(excel_pb2.CloseRequest(session_id=waiting.session_id))
    finally:
        server.stop(1)
    assert waiting.session_id not in service.pending
    assert not created[-1].exists()


def test_calc_prompt_parks_the_file_without_keeping_a_lock():
    plain = FakeBook([(0, "Titles", titles_grid())])
    plain.hold_stream = True

    def opener(path, password=None):
        if password:
            return FakeBook([(0, "Secret", titles_grid())])
        if Path(path).read_bytes() == b"prompt":
            raise PasswordRequired()
        return plain

    _service, server, stub = start(opener=opener)
    reader = None
    try:
        first = _upload(stub, "plain.xlsx", b"plain")
        stub.SelectHeader(excel_pb2.SelectHeaderRequest(session_id=first.session_id, excel_row=4))
        reader = _start_stream(stub, first.session_id)
        assert plain.stream_started.wait(2)
        waiting = _upload(stub, "book.xls", b"prompt")
        assert waiting.password_required
        assert not plain.stream_release.is_set()
        view = stub.ProvidePassword(
            excel_pb2.ProvidePasswordRequest(session_id=waiting.session_id, password="secret")
        )
        assert [sheet.name for sheet in view.sheets] == ["Secret"]
        assert not plain.stream_release.is_set()
    finally:
        plain.stream_release.set()
        if reader is not None:
            reader.join(2)
        server.stop(1)
    assert reader is not None and not reader.is_alive()


def _ods(path: Path, manifest: bytes) -> None:
    with zipfile.ZipFile(path, "w") as package:
        package.writestr("META-INF/manifest.xml", manifest)


def _upload(stub, filename, payload, password=""):
    def chunks():
        yield excel_pb2.OpenChunk(
            meta=excel_pb2.OpenMeta(filename=filename, password=password)
        )
        yield excel_pb2.OpenChunk(data=payload)

    return stub.OpenWorkbook(chunks())


def _start_stream(stub, session_id):
    def read():
        try:
            list(stub.ReadTable(excel_pb2.ReadTableRequest(session_id=session_id)))
        except grpc.RpcError:
            return

    reader = threading.Thread(target=read)
    reader.start()
    return reader


def _track_tempfiles(monkeypatch):
    from excel_tooling import server as server_mod

    created = []
    real_mkstemp = server_mod.tempfile.mkstemp

    def tracking(*args, **kwargs):
        handle, name = real_mkstemp(*args, **kwargs)
        created.append(Path(name))
        return handle, name

    monkeypatch.setattr(server_mod.tempfile, "mkstemp", tracking)
    return created
