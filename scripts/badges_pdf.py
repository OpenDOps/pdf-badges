"""Draw one badge page per workbook row.

The workbook is read through excel-tooling. The pages are drawn by pdf-tooling.
The sheet has no column-name row, so SelectColumns names the columns and every
row is data. Column A is the surname, column B is the given name, column C is
the company. The page is scripts/badges/tadviser.yaml.
"""

import argparse
import importlib
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

import excel_tooling
import grpc

# Mustache field -> (excel column letter, column name).
BINDING = {
    "surname": ("A", "surname"),
    "name": ("B", "name"),
    "company": ("C", "company"),
}

MAX_MESSAGE = 64 * 1024 * 1024


def main():
    parser = argparse.ArgumentParser(description="Render badge PDF from a workbook")
    parser.add_argument("--workbook", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--excel", default="excel-tooling:50051")
    parser.add_argument("--pdf", default="pdf-tooling:50052")
    parser.add_argument("--template", required=True, type=Path)
    parser.add_argument("--font-dir", required=True, type=Path)
    parser.add_argument("--proto-root", required=True, type=Path)
    parser.add_argument("--excel-gen", required=True, type=Path)
    parser.add_argument("--sheet", default="")
    args = parser.parse_args()
    _use_excel_stubs(args.excel_gen)

    rows = read_rows(args.excel, args.workbook, args.sheet)
    pdf = render_pdf(args.pdf, args.template, args.font_dir, args.proto_root, rows)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(pdf)
    print(f"pages: {len(rows)}")
    print(f"wrote {args.output} ({len(pdf)} bytes)")


def _use_excel_stubs(gen: Path):
    """Prefer the repository stubs over the ones baked into the service image."""
    sys.path.insert(0, str(gen))
    global excel_pb2, excel_pb2_grpc, table_pb2
    from irbis.excel.v1 import excel_pb2 as excel_messages
    from irbis.excel.v1 import excel_pb2_grpc as excel_grpc
    from irbis.table.v1 import table_pb2 as table_messages
    excel_pb2 = excel_messages
    excel_pb2_grpc = excel_grpc
    table_pb2 = table_messages


def read_rows(address, workbook: Path, sheet_name: str):
    channel = grpc.insecure_channel(address, options=_options())
    stub = excel_pb2_grpc.ExcelToolingStub(channel)
    payload = workbook.read_bytes()
    session_id = None
    try:
        view = stub.OpenWorkbook(_open_chunks(payload))
        session_id = view.session_id
        if view.password_required:
            raise SystemExit("workbook is password protected")
        print("sheets:", [(sheet.index, sheet.name) for sheet in view.sheets])
        if sheet_name:
            match = next((sheet for sheet in view.sheets if sheet.name == sheet_name), None)
            if match is None:
                raise SystemExit(f"sheet not found: {sheet_name}")
            view = stub.SelectSheet(excel_pb2.SelectSheetRequest(
                session_id=session_id,
                sheet_index=match.index,
                header_candidate_limit=1,
            ))
        selected = stub.SelectColumns(excel_pb2.SelectColumnsRequest(
            session_id=session_id,
            columns=[
                table_pb2.Column(excel_id=letter, name=name)
                for letter, name in BINDING.values()
            ],
        ))
        print("columns:", [f"{column.excel_id}={column.name}" for column in selected.columns])
        id_to_field = {
            letter: field
            for field, (letter, _name) in BINDING.items()
        }
        rows = []
        for batch in stub.ReadTable(excel_pb2.ReadTableRequest(session_id=session_id)):
            for row in batch.rows:
                values = {field: "" for field in BINDING}
                for cell in row.cells:
                    field = id_to_field.get(cell.excel_id)
                    if field is not None:
                        values[field] = _cell_text(cell.value)
                rows.append(values)
        if not rows:
            raise SystemExit("workbook has no data rows")
        first = rows[0]
        print(f"rows: {len(rows)}")
        print(f"first: {first['surname']} {first['name']} / {first['company']}")
        return rows
    except grpc.RpcError as err:
        raise SystemExit(f"excel-tooling: {err.code().name} {err.details()}") from err
    finally:
        if session_id:
            stub.Close(excel_pb2.CloseRequest(session_id=session_id))
        channel.close()


def render_pdf(address, template: Path, font_dir: Path, proto_root: Path, rows):
    pdf_pb2, pdf_pb2_grpc = _pdf_stubs(proto_root)
    channel = grpc.insecure_channel(address, options=_options())
    stub = pdf_pb2_grpc.PdfToolingStub(channel)
    session_id = None
    try:
        view = stub.LoadTemplate(_template_chunks(pdf_pb2, template, font_dir))
        session_id = view.session_id
        loaded = [field.name for field in view.fields]
        print("fields:", loaded)
        unknown = [field for field in BINDING if field not in loaded]
        if unknown:
            raise SystemExit(f"binding field is not on the template: {', '.join(unknown)}")
        unbound = [name for name in loaded if name not in BINDING]
        if unbound:
            raise SystemExit(f"template field has no column: {', '.join(unbound)}")
        request = pdf_pb2.RenderRequest(
            session_id=session_id,
            rows=[
                pdf_pb2.ValueRow(values=[
                    pdf_pb2.FieldValue(field=field, value=row[field])
                    for field in BINDING
                ])
                for row in rows
            ],
        )
        return b"".join(chunk.data for chunk in stub.RenderChunks(request))
    except grpc.RpcError as err:
        raise SystemExit(f"pdf-tooling: {err.code().name} {err.details()}") from err
    finally:
        if session_id:
            stub.Close(pdf_pb2.CloseRequest(session_id=session_id))
        channel.close()


def _pdf_stubs(proto_root: Path):
    out = Path(tempfile.mkdtemp())
    subprocess.check_call([
        sys.executable,
        "-m",
        "grpc_tools.protoc",
        f"-I{proto_root}",
        f"--python_out={out}",
        f"--grpc_python_out={out}",
        str(proto_root / "irbis" / "pdf" / "v1" / "pdf.proto"),
    ])
    for directory in out.rglob("*"):
        if directory.is_dir():
            (directory / "__init__.py").touch()
    (out / "__init__.py").touch()
    import irbis
    irbis.__path__.append(str(out / "irbis"))
    pdf_pb2 = importlib.import_module("irbis.pdf.v1.pdf_pb2")
    pdf_pb2_grpc = importlib.import_module("irbis.pdf.v1.pdf_pb2_grpc")
    return pdf_pb2, pdf_pb2_grpc


def _open_chunks(payload: bytes):
    yield excel_pb2.OpenChunk(meta=excel_pb2.OpenMeta(
        filename="badges.xlsx",
        header_candidate_limit=1,
    ))
    yield excel_pb2.OpenChunk(data=payload)


def _template_chunks(pdf_pb2, template: Path, font_dir: Path):
    archive = _template_zip(template, font_dir)
    yield pdf_pb2.TemplateChunk(meta=pdf_pb2.TemplateMeta(filename=template.name))
    yield pdf_pb2.TemplateChunk(data=archive)


def _template_zip(template: Path, font_dir: Path) -> bytes:
    yaml = template.read_text(encoding="utf-8")
    names = []
    for line in yaml.splitlines():
        stripped = line.strip()
        if stripped.startswith("source_path:"):
            names.append(Path(stripped.split(":", 1)[1].strip().strip("\"'")).name)
    if not names:
        raise SystemExit(f"no fonts listed in {template}")
    buffer = tempfile.SpooledTemporaryFile()
    with zipfile.ZipFile(buffer, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(template.name, yaml)
        for name in names:
            font = font_dir / name
            if not font.is_file():
                raise SystemExit(f"font not found: {font}")
            archive.write(font, f"fonts/{name}")
    buffer.seek(0)
    return buffer.read()


def _cell_text(value) -> str:
    kind = value.WhichOneof("kind")
    if kind == "text":
        return value.text
    if kind == "int_value":
        return str(value.int_value)
    if kind == "float_value":
        return _float_text(value.float_value)
    if kind == "bool_value":
        return "true" if value.bool_value else "false"
    if kind == "date":
        date = value.date
        day = f"{date.year:04d}-{date.month:02d}-{date.day:02d}"
        if date.has_time:
            return f"{day} {date.hour:02d}:{date.minute:02d}:{date.second:02d}"
        return day
    return ""


def _float_text(number: float) -> str:
    text = format(number, ".16g")
    if "." in text and "e" not in text and "E" not in text:
        text = text.rstrip("0").rstrip(".")
    return text


def _options():
    return [
        ("grpc.max_send_message_length", MAX_MESSAGE),
        ("grpc.max_receive_message_length", MAX_MESSAGE),
    ]


if __name__ == "__main__":
    main()
