# excel-tooling

Reads a spreadsheet, lets the caller choose which sheet and which row are the column names, and returns a typed `Table` ([table.md](../table.md)). The service runs in Docker and speaks gRPC. The build sequence for a working first version is [implementation-plan.md](implementation-plan.md). How to start it and check a workbook by hand is [runbook.md](runbook.md).

LibreOffice is the engine, not Apache OpenOffice. LibreOffice is the maintained line (current stable is the 25.x series). Apache OpenOffice has had no feature release in years. The package inside the image is LibreOffice Calc.

Google Sheets and Yandex Tables are later openers on this same service. After a source is open, sheet selection, header selection, and `ReadTable` are the same calls.

## What the caller does

1. Upload a workbook. The reply already contains the non-empty sheets and the header candidates of the first sheet, which is selected.
2. If another sheet is wanted, select it. The reply is that sheet's header candidates.
3. Select one candidate row as the column names, or name the columns yourself when the sheet has no header row. The reply is the ordered column list.
4. Read the table as a stream. After `SelectHeader`, data rows start under the chosen header. After `SelectColumns`, every used row is data, including the first. Fully empty rows are skipped. Reading stops after 50 empty rows in a row, or at the end of the used area, whichever comes first. Column names arrived with `SelectHeader` or `SelectColumns`. Each `ReadTable` message carries the next processed rows, at most 100.

```text
OpenWorkbook
    │
    ├─ sheets: only non-empty sheets, tab order
    │    first sheet is selected
    │
    └─ header candidates: first K non-empty rows
           on the selected sheet
                 │
                 ▼
         SelectHeader(row)          SelectColumns(excel_id, name)
                 │                                  │
                 ▼                                  ▼
              ReadTable  (after the header)      ReadTable  (every used row)
```

"List" in the product language is a worksheet. The API calls it a sheet.

## Finding rows

A cell is empty when Calc reports it empty: no text, no number, no boolean, no error. Formatting alone does not count. A formula that calculates to an empty string counts as empty.

A row is empty when every cell in the used column span is empty. Merged cells keep their value on the top-left cell only; the covered cells are empty. v1 does not repeat a merged value across the merge.

Scan from row 1 downward.

**Sheets.** A sheet is listed when a content query finds at least one real value (text, number, boolean, error, or formula). A used area that is only formatting is omitted. Hidden sheets are listed and may be selected. The default selection is the first listed sheet in tab order. If the workbook has no such sheet, `OpenWorkbook` fails.

**Header candidates.** From the rows that contain a value, take the next K non-empty rows. Empty spans are not read one row at a time. The walk stops once K non-empty rows are found. The default K is 10. The caller may set K from 1 to 50 on open or on sheet selection. Each candidate is the sheet row number (1-based) plus the non-empty cells of that row, left to right: column letter and trimmed cell text. Numbers and dates in a candidate row are sent as their displayed text, because a header is a name, not a value.

**After the header is chosen.** Columns are the non-empty cells of that row. Duplicate trimmed names fail the selection. Data begins on the next sheet row after the header. Candidate rows above the chosen header are titles; they are not data. A data row is emitted only when at least one column that has a header is non-empty. Cells in columns without a header are ignored. Fully empty data rows are not emitted. Fifty consecutive skipped rows end the table. The unused area past Calc's used range also ends the table, so a formatted-but-empty tail cannot run to row 1048576.

Example, K = 3, the caller picks row 4:

```text
row 1   (empty)
row 2   "Quarter report"          candidate, not selected
row 3   (empty)
row 4   Name | City | Amount      selected header
row 5   Ann  | Oslo | 2           data
row 6   (empty)                   skipped
row 7   Bo   |     | 3            data, City omitted
... 50 empty rows ...             stop, later rows are not read
```

## Columns without a header

A sheet can have no column-name row. `SelectColumns` names the columns itself. Each entry is an `excel_id` (`A`, `B`, `AA`) and the name the caller wants on the table, such as a template field. `a` is the same column as `A`. Names are trimmed. An empty list, an empty name, a repeated name, a repeated column, a value that is not a column letter, or a column that lists subcolumns is rejected. There is no subcolumn row on this call.

`ReadTable` then starts at the first row of the used area. That row is data. No row is skipped as a title or a header. Empty rows and the 50-row stop are unchanged. A selected column that has no value on a row is omitted from that row, the same way an empty header cell is omitted.

`SelectColumns` and `SelectHeader` replace each other. `SelectSheet` clears both. `ReadTable` before either call fails.

```text
row 1   Абрамов | Александр | Медиа группа Авангард    data
row 2   Авхадыев | Антон    | СёрчИнформ                data
```

The call that reads this sheet is `SelectColumns` with `A`/`surname`, `B`/`name`, and `C`/`company name`.

## Subcolumns

The column-name row can have a second row under it. That row is optional. It is not another candidate for the column names. The caller sets it only after the column row is chosen, and only on one of the next four sheet rows: `column_row + 1` through `column_row + 4`. A row outside that window is rejected. `0` means there is no subcolumn row.

Data then starts on the first sheet row after the subcolumn row. Rows between the column row and the subcolumn row are not data. With no subcolumn row, data still starts on the row after the column row.

A parent column is a non-empty trimmed cell on the column row. The next parent is the next non-empty trimmed cell on that same row. Every column-row cell between them is empty, and that empty run is how far the first parent extends. On the subcolumn row, the non-empty trimmed cells from the parent’s own column up to but not including the next parent are that parent’s subcolumns, left to right.

The parent keeps its own name and excel id, and it holds those subcolumns. The next non-empty cell on the column row is the next parent. A parent with no labels in that span stays a single column. Subcolumn names must be unique under one parent. The same label under two parents is kept. Two parent names that trim to the same text are still an error.

On `JPGVL0I5VW.xlsm`, column row 2 and subcolumn row 3, `AR` is `Как Вы узнали о LINGERIE SHOW-FORUM?`. Row 2 is empty from `AS` until the next name. Row 3 labels `AR` through `BG` (`Реклама на сайтах` through `Свой вариант`) belong to that parent. The next parent on row 2 is the next non-empty name after that run. Visitor values start at row 4.

## Streaming

Calc loads the whole file on open, and `calculateAll()` runs in that process. UNO has no call that leaves the rest of an `.xlsx` or `.xlsm` on disk. The window is only the copy this service keeps.

`ReadTable` asks Calc for at most **2,000 sheet rows** at a time, and only the leaf columns of the selected header. A parent with subcolumns is read through those subcolumns. Gaps between leaf columns are separate ranges, not one array across the used width. A sheet shorter than 2,000 rows is one read of those columns. Those rows are classified, then sent on the stream in batches of at most **100 processed rows**. When a batch is written, it is dropped. When the 2,000-row block has been sent, that block is dropped and the next 2,000 sheet rows are read. The server does not keep the previous block, and it does not load the next block before the current one has been sent. The lock around UNO is held for one read, then released while the batches go out.

Empty sheet rows inside a block count toward the 50-row stop and are not messages. A stop or the end of the used area ends the stream. If the client cancels, the walk stops before the next block. `ReadTable` with `background` and `channel` `queue` keeps walking after the RPC returns and discards each batch. Any other channel is rejected, and `channel` without `background` is rejected. A failure mid-stream fails the RPC. The client does not treat a short successful prefix as the whole table.

Header candidates are the first K non-empty rows only. That prefix is read once for a sheet and a K, then reused by `SelectHeader` and `ReadTable`. Opening a workbook does not copy every data cell into Python. Column names from `SelectHeader` or `SelectColumns` stay for the session. A header is one row. Named columns are the list the caller sent.

`EXCEL_MAX_SESSIONS` caps open workbooks. The default is 100. Opening one more closes the workbook that has been idle the longest. A workbook that is inside a Calc call stays open. If every open workbook is busy, the new open is rejected.

## Cell classification

Done in Calc after `calculateAll()`, so formulas are values. See [table.md](../table.md) for the protobuf kinds. Implementation notes that are easy to get wrong:

- Use the number-format type (date, time, date-time, logical, number), not a guess from the displayed string.
- Honor the workbook's 1904 date system. Read the serial through UNO and convert with the document's epoch so the civil date matches the sheet.
- Do not return the formula text.
- Percent and currency are `float_value`.
- Disable macro execution. A workbook must not run macros while it is opened.

Accepted files: `.xlsx`, `.xlsm`, `.xls`, `.ods`. Anything else is rejected at open. CSV is out of scope; it has no sheet list and no number formats.

## gRPC

`proto/irbis/excel/v1/excel.proto`

```protobuf
syntax = "proto3";
package irbis.excel.v1;

import "irbis/table/v1/table.proto";

service ExcelTooling {
  rpc OpenWorkbook(stream OpenChunk) returns (SessionView);
  rpc SelectSheet(SelectSheetRequest) returns (SessionView);
  rpc SelectHeader(SelectHeaderRequest) returns (irbis.table.v1.Table);
  rpc SelectColumns(SelectColumnsRequest) returns (irbis.table.v1.Table);
  rpc ReadTable(ReadTableRequest) returns (stream RowBatch);
  rpc ProvidePassword(ProvidePasswordRequest) returns (SessionView);
  rpc Close(CloseRequest) returns (CloseResponse);
}

message OpenChunk {
  oneof part {
    OpenMeta meta = 1;   // first message
    bytes data = 2;
  }
}

message OpenMeta {
  string filename = 1;                 // used for the extension check
  uint32 header_candidate_limit = 2;   // 0 means 10
  string password = 3;
}

message SessionView {
  string session_id = 1;
  repeated SheetInfo sheets = 2;
  uint32 selected_sheet_index = 3;
  repeated HeaderCandidate header_candidates = 4;
  bool password_required = 5;
}

message SheetInfo {
  string name = 1;
  uint32 index = 2;    // 0-based index in the workbook, including empty sheets that were not listed
}

message HeaderCandidate {
  uint32 excel_row = 1;
  repeated irbis.table.v1.Column columns = 2;
}

message SelectSheetRequest {
  string session_id = 1;
  uint32 sheet_index = 2;
  uint32 header_candidate_limit = 3;   // 0 means 10
}

message SelectHeaderRequest {
  string session_id = 1;
  uint32 excel_row = 2;
  uint32 subcolumn_row = 3;  // 0 means no subcolumn row
}

message SelectColumnsRequest {
  string session_id = 1;
  repeated irbis.table.v1.Column columns = 2;  // excel_id and name; no subcolumns
}

message ReadTableRequest {
  string session_id = 1;
  bool background = 2;
  string channel = 3;  // required with background; only "queue"
}

message RowBatch {
  repeated irbis.table.v1.Row rows = 1;   // at most 100
}

message ProvidePasswordRequest {
  string session_id = 1;
  string password = 2;
}

message CloseRequest {
  string session_id = 1;
}

message CloseResponse {}
```

`SelectHeader` and `SelectColumns` each return a `Table` with `columns` set and `rows` empty, so the caller can show the names before reading. `subcolumn_row` is `0` or one of the four rows after the chosen column row. When it is set, each parent column that spans empty cells on the column row lists those subcolumns, and data starts after the subcolumn row. `SelectColumns` has no subcolumn row: the names in the request are the column names, and data starts on the first used row. `ReadTable` then streams `RowBatch` messages, each with at most 100 data rows and no column list. The client already has the names. `ReadTable` before `SelectHeader` or `SelectColumns` fails. Selecting a sheet clears a previous header choice and a previous column choice, including a subcolumn row. The server's max send size stays 64 MiB so one batch is not capped by the default.

The upload is client-streaming so a workbook is not capped by the default gRPC message size. Set the server's max receive size to 64 MiB as well, for a single large chunk.

An encrypted workbook is not opened until a password is available, so Calc never sits on a password prompt. `OpenMeta.password` opens it on the first call. With no password, `OpenWorkbook` returns `password_required` and keeps the uploaded file for one minute. That wait holds no Calc lock. `ProvidePassword` then opens it. A wrong password leaves the wait in place. After one minute, or on `Close`, the file is deleted.

Sessions live in memory in the process that opened them. Idle time is 15 minutes from the last call on that workbook. A timer and every RPC drop each session that has been idle that long, then discard its file and Calc document. `Close` discards its session immediately. A restart drops every session. There is no shared volume protocol in v1: the bytes on the stream are the file.

Errors are gRPC status codes with a short reason: invalid file, no non-empty sheet, unknown session, sheet index not in the non-empty list, header row not in the candidate list, duplicate column name, or a `SelectColumns` entry that is empty, repeated, not a column letter, or has subcolumns.

## Process shape

One container, one LibreOffice listener, one Python gRPC server.

```text
entrypoint
  soffice --headless --nologo --nofirststartwizard --norestore
          --accept='socket,host=127.0.0.1,port=2002;urp;StarOffice.ComponentContext'
  python -m excel_tooling
```

Calc is not safe for concurrent document use on one process. The server keeps a lock around UNO calls and handles one workbook operation at a time. Scale by running more containers. The gRPC server can accept other connections while one call holds the lock; those calls wait.

Open loads the document through UNO (`com.sun.star.sheet.SpreadsheetDocument`) with `MacroExecutionMode` 0 and `UpdateDocMode` `NO_UPDATE`, so external links are not refreshed. It then runs `calculateAll()` and builds the sheet list and the default candidates. The compose network is internal, so the container has no route to other hosts. The document stays open on the session. `ReadTable` walks rows from the header in blocks of 2,000 sheet rows. It does not convert the file to CSV. CSV would drop the distinction between int, float, and date.

```text
modules/excel-tooling/
  pyproject.toml
  Dockerfile
  excel_tooling/
    server.py          gRPC handlers, session map
    workbook.py        UNO open, sheets, candidates, row walk
    values.py          format type to Value
    columns.py         index to letter A, Z, AA
  tests/
    fixtures/          small xlsx files: header on row 1, titles above the header,
                       50-row gap, dates, ints, formulas
```

Tests open those fixtures through the same `workbook.py` path. They do not need a golden PDF. A fixture with a formula cell checks that the calculated number comes back, not the formula.

## Docker

Base image with `libreoffice-calc` and the Python UNO bits that ship with it (`python3-uno` on Debian). No full desktop. Port `50051` is on the compose network `private` and is not published to the host. Callers on that network use plaintext `excel-tooling:50051`. TLS is the gateway's job. gRPC reflection stays off on `0.0.0.0:50051`. It is enabled only when the process listens on a loopback address. A read-only root and a writable `/tmp` for the files Calc opens. `MacroExecutionMode` forced off in the soffice profile baked into the image.

```text
docker compose up excel-tooling
```

## Later adapters

The session after open is source-shaped the same way, so the service grows new open methods rather than a new proto for each vendor:

- `OpenGoogleSpreadsheet` — spreadsheet id, and credentials carried in the first message (not a file upload)
- `OpenYandexSpreadsheet` — the same idea for Yandex Tables

Both return `SessionView`. Header candidates and `ReadTable` stay. The adapter maps that product's column position to `excel_id` letters and maps its cell types onto `Value`. Those open methods are not part of the first implementation.
