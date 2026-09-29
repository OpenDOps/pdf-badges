# excel-tooling runbook

How to start the service and check it by hand. The reader design is [design.md](design.md). The shared table is [table.md](../table.md).

Commands below are run from `modules/excel-tooling` unless a step says otherwise. The service listens on port `50051`. `SelectHeader` returns the column names in one reply. `ReadTable` then streams data rows, at most 100 per message.

A `javaldx` warning from the container is harmless. The server is up when the process stays running and the client below prints the sheets.

## Start the server

Docker Desktop has to be running. From the repository root:

```bash
docker compose up --build excel-tooling
```

The container root is read-only. `/tmp` is writable, and that is where uploads and the LibreOffice profile go. The profile baked into the image disables macro execution, and each workbook is still opened with `MacroExecutionMode` 0. Wait until the service is healthy. It listens on port `50051`.

Leave that terminal open. The next sections use a second terminal.

## Manual check: JPGVL0I5VW.xlsm

The fixture is `tests/fixtures/JPGVL0I5VW.xlsm`. Sheet `База посетителей` is selected on open. With `header_candidate_limit` 3, the candidates are rows 1, 2, and 3.

Row 2 is the column names (`ID`, `Дата`, and the rest). Row 3 is the subcolumn row. `subcolumn_row` must be one of the four rows after the column row. Here that is row 3.

```bash
.venv/bin/python << 'PY'
import excel_tooling
import grpc
from irbis.excel.v1 import excel_pb2, excel_pb2_grpc

path = "tests/fixtures/JPGVL0I5VW.xlsm"
header_row = 2
subcolumn_row = 3
channel = grpc.insecure_channel("localhost:50051", options=[
    ("grpc.max_send_message_length", 64 * 1024 * 1024),
    ("grpc.max_receive_message_length", 64 * 1024 * 1024),
])
stub = excel_pb2_grpc.ExcelToolingStub(channel)
payload = open(path, "rb").read()

def chunks():
    yield excel_pb2.OpenChunk(meta=excel_pb2.OpenMeta(
        filename="JPGVL0I5VW.xlsm", header_candidate_limit=3
    ))
    yield excel_pb2.OpenChunk(data=payload)

view = stub.OpenWorkbook(chunks())
print("sheets:", [(sheet.index, sheet.name) for sheet in view.sheets])
print("header candidates:")
for candidate in view.header_candidates:
    labels = [f"{column.excel_id}={column.name}" for column in candidate.columns]
    print(f"  row {candidate.excel_row}: {', '.join(labels[:8])}{'...' if len(labels) > 8 else ''}")

selected = stub.SelectHeader(excel_pb2.SelectHeaderRequest(
    session_id=view.session_id, excel_row=header_row, subcolumn_row=subcolumn_row
))
print("columns:")
for column in selected.columns:
    print(f"{column.excel_id}\t{column.name}")
    for sub in column.subcolumns:
        print(f"  {sub.excel_id}\t{sub.name}")

batches = list(stub.ReadTable(excel_pb2.ReadTableRequest(session_id=view.session_id)))
rows = [row for batch in batches for row in batch.rows]
print(f"batches: {len(batches)}")
print(f"rows: {len(rows)}")
for row in rows:
    cells = []
    for cell in row.cells:
        kind = cell.value.WhichOneof("kind")
        cells.append(f"{cell.excel_id}={getattr(cell.value, kind)}")
    print(f"{row.excel_row}\t" + "\t".join(cells))
stub.Close(excel_pb2.CloseRequest(session_id=view.session_id))
PY
```

Each parent column is one line. Its subcolumns are the indented lines under it. A parent with no indented lines has no subcolumns.

Expect:

- Sheets `(0, База посетителей)` and `(1, prop)`. Chart tabs are absent.
- Three candidates, rows 1, 2, and 3.
- 65 parent columns. `A` is `ID`. `AH` is `Дата`.
- `AR` `Как Вы узнали о LINGERIE SHOW-FORUM?` holds 16 subcolumns, from `AR` `Реклама на сайтах` through `BG` `Свой вариант`. The next parent is `BH` `Даты участия (эксп)`, with no subcolumns.
- `Другое` and `Свой вариант` each appear under more than one parent. That is kept.
- 2923 data rows, streamed in batches of 100, with a shorter last batch. The first data row is excel row 4. Row 3 is not a data row.
- On row 4, `A` is text `JPGVL0I5VW_exp6`, `E` is text `1712460`, and `AH` is text `02.09.2026`.

The row print is long. Column mappings are printed before `rows:`.

### Other selections on the same file

Change the two numbers at the top of the script.

| `header_row` | `subcolumn_row` | Result |
|---|---|---|
| 2 | 3 | Full names plus the subcolumn mapping above. Data starts at row 4. |
| 2 | 0 | Full names, no subcolumns. Row 3 is the first data row. 2924 data rows. `AS` is not a column. |
| 1 | 0 | Short ids. 109 columns, `A` is `h0`, `DE` is `daysvisit`. The first data row is excel row 2 and cell `A` is text `ID`. |
| 3 | 0 | Fails. Row 3 repeats `Другое` and `Свой вариант`, so it cannot be the column-name row. |

`subcolumn_row` of 0 means there is no subcolumn row. A value outside `header_row + 1` through `header_row + 4` is rejected.

## Automated tests

Host tests skip anything that needs LibreOffice:

```bash
.venv/bin/pytest -q -m "not libreoffice"
```

The service image's default command is `python -m excel_tooling`. Calc tests override that command. The entrypoint starts soffice, then pytest:

```bash
docker compose build excel-tooling
docker run --rm excel-tooling pytest -m libreoffice -q
```

`tests/test_lingerie.py` is the check against `JPGVL0I5VW.xlsm`, including the subcolumn mapping. To run only that file:

```bash
docker run --rm excel-tooling pytest tests/test_lingerie.py -q
```

With the compose service healthy, `grpcurl` on your `PATH`, the smoke client checks both fixtures and that `grpcurl` lists `irbis.excel.v1.ExcelTooling`:

```bash
.venv/bin/python tests/smoke.py tests/fixtures/titles.xlsx
.venv/bin/python tests/smoke.py tests/fixtures/JPGVL0I5VW.xlsm
```
