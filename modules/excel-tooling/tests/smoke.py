"""Smoke client for excel-tooling on localhost:50051.

Usage, from modules/excel-tooling, with the service already up:

    python tests/smoke.py tests/fixtures/titles.xlsx
    python tests/smoke.py tests/fixtures/JPGVL0I5VW.xlsm
"""

import subprocess
import sys
from pathlib import Path

import grpc

import excel_tooling  # noqa: F401
from irbis.excel.v1 import excel_pb2, excel_pb2_grpc

ADDRESS = "localhost:50051"
CHANNEL_OPTIONS = [
    ("grpc.max_send_message_length", 64 * 1024 * 1024),
    ("grpc.max_receive_message_length", 64 * 1024 * 1024),
]


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: python tests/smoke.py tests/fixtures/<file>")
    path = Path(sys.argv[1])
    check_grpcurl()
    if path.name == "titles.xlsx":
        check_titles(path)
    elif path.name == "JPGVL0I5VW.xlsm":
        check_lingerie(path)
    else:
        raise SystemExit(f"unsupported fixture: {path.name}")
    print(f"ok {path.name}")


def check_grpcurl() -> None:
    listed = subprocess.check_output(
        ["grpcurl", "-plaintext", ADDRESS, "list"], text=True
    )
    if "irbis.excel.v1.ExcelTooling" not in listed.split():
        raise SystemExit(listed)


def check_titles(path: Path) -> None:
    stub, view = open_workbook(path, limit=10)
    try:
        selected = stub.SelectSheet(
            excel_pb2.SelectSheetRequest(
                session_id=view.session_id, sheet_index=1, header_candidate_limit=10
            )
        )
        if selected.selected_sheet_index != 1:
            raise SystemExit(f"sheet index {selected.selected_sheet_index}")
        header = stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=4)
        )
        rows = read_rows(stub, view.session_id)
    finally:
        stub.Close(excel_pb2.CloseRequest(session_id=view.session_id))
    names = [column.name for column in header.columns]
    if names != ["Name", "City", "Amount", "When", "Active"]:
        raise SystemExit(f"columns {names}")
    if [row.excel_row for row in rows] != [5, 7]:
        raise SystemExit(f"rows {[row.excel_row for row in rows]}")
    amount = _id(header.columns, "Amount")
    when = _id(header.columns, "When")
    city = _id(header.columns, "City")
    if _cell(rows[0], amount).value.int_value != 2:
        raise SystemExit("row 5 amount")
    date = _cell(rows[0], when).value.date
    if (date.year, date.month, date.day, date.has_time) != (2024, 3, 15, False):
        raise SystemExit("row 5 date")
    if any(cell.excel_id == city for cell in rows[1].cells):
        raise SystemExit("row 7 city")
    if _cell(rows[1], amount).value.float_value != 3.5:
        raise SystemExit("row 7 amount")
    if _cell(rows[1], when).value.int_value != 2:
        raise SystemExit("row 7 formula")


def check_lingerie(path: Path) -> None:
    stub, view = open_workbook(path, limit=3)
    try:
        sheets = [(sheet.index, sheet.name) for sheet in view.sheets]
        if sheets != [(0, "База посетителей"), (1, "prop")]:
            raise SystemExit(f"sheets {sheets}")
        header = stub.SelectHeader(
            excel_pb2.SelectHeaderRequest(session_id=view.session_id, excel_row=2)
        )
        rows = read_rows(stub, view.session_id)
    finally:
        stub.Close(excel_pb2.CloseRequest(session_id=view.session_id))
    by_id = {column.excel_id: column.name for column in header.columns}
    if len(header.columns) != 65 or by_id.get("A") != "ID" or by_id.get("AH") != "Дата":
        raise SystemExit("columns")
    if by_id.get("DC") != "Область проживания (Не из списка)" or "AS" in by_id:
        raise SystemExit("columns")
    if len(rows) != 2924 or rows[0].excel_row != 3:
        raise SystemExit(f"rows {len(rows)} first {rows[0].excel_row if rows else None}")
    visitor = next(row for row in rows if row.excel_row == 4)
    expected = {"A": "JPGVL0I5VW_exp6", "E": "1712460", "AH": "02.09.2026"}
    for excel_id, text in expected.items():
        if _cell(visitor, excel_id).value.text != text:
            raise SystemExit(f"row 4 {excel_id}")


def open_workbook(path: Path, limit: int):
    channel = grpc.insecure_channel(ADDRESS, options=CHANNEL_OPTIONS)
    stub = excel_pb2_grpc.ExcelToolingStub(channel)
    payload = path.read_bytes()

    def chunks():
        yield excel_pb2.OpenChunk(
            meta=excel_pb2.OpenMeta(filename=path.name, header_candidate_limit=limit)
        )
        yield excel_pb2.OpenChunk(data=payload)

    return stub, stub.OpenWorkbook(chunks())


def read_rows(stub, session_id):
    rows = []
    for batch in stub.ReadTable(excel_pb2.ReadTableRequest(session_id=session_id)):
        rows.extend(batch.rows)
    return rows


def _id(columns, name):
    return next(column.excel_id for column in columns if column.name == name)


def _cell(row, excel_id):
    return next(cell for cell in row.cells if cell.excel_id == excel_id)


if __name__ == "__main__":
    main()
