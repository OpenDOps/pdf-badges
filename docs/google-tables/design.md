# google-tables

Watches Google Sheets spreadsheets (Google Таблицы). Each data row has a UID. A formula column fingerprints the mapped cells. After Drive `version` has been still for 10 seconds, the service reads those two columns and downloads mapped cells only for rows that were added or changed. One LMDB file holds the snapshot. Subscribers receive the rows that were added, updated, or deleted.

The sheet is the source of identity. LMDB is the snapshot. The event log is the difference. The build sequence is [implementation-plan.md](implementation-plan.md).

Header finding, column naming, and cell types are the rules in [excel-tooling/design.md](../excel-tooling/design.md). The row type is [table.md](../table.md). This service is a Rust binary: it writes two hidden columns, watches Drive, and keeps the snapshot across restarts. excel-tooling stays Python because it drives LibreOffice. On the registration device this binary is the sheet source in [sheets-source.md](../registration-server/sheets-source.md).

## What the caller does

1. Open a spreadsheet by id. The reply lists the non-empty sheets and the header candidates of the first sheet.
2. Select another sheet when the first is the wrong one. The watch stores Google's `sheetId`. Tab position is display only.
3. Select a candidate row as the column names, or name the columns when the sheet has no header row.
4. Mark columns the fingerprint must skip. `NOW()`, `TODAY()`, and `RAND()` belong on that list. `SetColumnOptions` is the call that records it.
5. Ensure identity. Two hidden columns are added when their metadata is missing. Existing letters stay as they are.
6. Read the table, or start a watch. Both assign a UID to every data row that lacks one. A read streams the rows. A watch stores them and starts the loop.

```text
OpenSpreadsheet
    │
    ▼
SelectSheet (optional)
    │
    ▼
SelectHeader(row)              SelectColumns(excel_id, name)
    │                                  │
    ▼                                  ▼
SetColumnOptions
    │
    ▼
EnsureIdentity
    │  hidden UID column, hidden fingerprint column
    ▼
ReadTable                          StartWatch
    │  fill empty UIDs                 │  fill empty UIDs, then LMDB
    ▼                                  ▼
  caller                             10 quiet seconds
                                     UID column + fingerprint
                                     mapped cells of changed rows
                                     Subscribe / ReadStored
```

Setup calls live on an in-memory session, 15 minutes from the last call. `Close` and a process restart drop it. `StartWatch` copies the mapping into LMDB. A restart resumes each watch from that record.

## Shared scan

excel-tooling keeps those rules in Python (`scan.py`, `columns.py`, `values.py`) because that process is the Calc source. This crate implements the same rules and is checked against the same fixtures. The two protos share `irbis.table.v1` only. A source still exposes `bounds`, `has_values`, `nonempty_rows`, `read`, and `release`.

`SelectHeader`, `SelectColumns`, and a one-shot `ReadTable` use the excel scan rules: header candidates, the subcolumn window, duplicate names, and the stop after 50 empty rows. The watch reads through the last non-empty UID or fingerprint cell, so a gap of empty rows is part of the snapshot and the rows below it stay.

Google's date serial counts days since 1899-12-30. The Rust classifier uses that epoch, the same one `values.py` uses for Excel.

## Columns

Sheets developer metadata attaches to a spreadsheet, a sheet, one entire row, or one entire column. It does not attach to a cell. A sheet stores 30,000 characters of metadata keys and values, and the spreadsheet stores another 30,000. One UID on every row would spend the sheet budget at about a thousand rows. The UID is therefore a cell value. Metadata marks the column, which is a few dozen characters.

| Key | Where | Value |
|---|---|---|
| `irbis.uid` | the hidden UID column | empty. The cells hold the 16-character ids |
| `irbis.fingerprint` | the hidden fingerprint column | empty. The first cell holds the formula |
| `irbis.column` | each mapped column | the name from `SelectHeader` or `SelectColumns` |
| `irbis.exclude_hash` | a mapped column the fingerprint skips | `1` |

Inserting or moving a column moves its metadata with it. The service finds columns by these keys. Letters are what a `Row` shows for the current position. The stored cell and the content hash use the metadata name.

The fingerprint formula is an `ARRAYFORMULA` that joins, per row, the mapped columns which are not excluded. A blank join means the row has no mapped values. Sheets recalculates a row when one of those cells changes, so the next read of this column names the changed rows. The formula is rewritten, as one cell, when a mapped column changes index or when `SetColumnOptions` changes `exclude_from_hash`.

A column left in the fingerprint that contains `NOW()`, `TODAY()`, or `RAND()` changes on every recalculation, and every row it touches is reported updated on every cycle.

`EnsureIdentity` searches these keys on the selected `sheetId`.

- A missing UID column is appended, hidden, marked, and covered by a warning-only protected range. An editor who changes a UID is prompted and can still edit. The service account keeps edit access.
- A missing fingerprint column is appended, hidden, marked, and given its formula.
- A column already marked is left where it is. An empty formula cell is rewritten from the current column positions.

A deleted service column is recreated the same way on the next cycle, including the warning-only range. A mapped column whose metadata disappears pauses the watch with status `column_missing`. The cycle does not assign that name to another letter.

The mapping is fixed at `StartWatch`. A sheet column with no `irbis.column` metadata is ignored. Editing a header cell does not change the stored name. `SelectHeader`, `SelectColumns`, `SetColumnOptions`, and `EnsureIdentity` fail while that `sheetId` is watched. Changing them is `StopWatch`, a new session, and `StartWatch` again. The same LMDB prefix is kept when the spreadsheet and `sheetId` match.

## Cycle

Drive `files.get` returns `version` for a native Google Sheet. `headRevisionId` is empty on those files and is not read.

A rescan starts when `version` has been the same for 10 seconds. A newer value restarts the 10 seconds. Typing, a paste, and a UID write each move `version`, and the scan runs after the burst settles.

Drive `changes.watch` is one channel per account. The notice body is empty. `changes.list` with the stored page token returns the changed files, and each watched spreadsheet restarts its 10-second wait. A shared drive has its own change log and its own token. With push off, each watched file's `version` is polled every 60 seconds and then uses the same wait.

Sheets and Drive calls share one limiter: 60 reads and 60 writes per minute per service account, 300 per minute per project. A `429` backs that account off for `min(2^n, 60)` seconds plus up to one second of jitter. `n` starts at 0 and resets after the next success. The quiet period does not issue a call during the backoff.

The seen baseline is the `version` stored in LMDB at the last commit. A write of UID cells or of the formula moves `version` and commits nothing. The watch waits another quiet 10 seconds and reads the two columns again. The baseline changes only when that later read has the same `version` before and after it.

### Read

1. Read `version`. Call it V.
2. Search the four metadata keys and read the UID column and the fingerprint column in one `values.batchGet`, from the first data row through the last non-empty cell of either column.
3. Read `version` again. A value other than V drops the read. Wait another 10 quiet seconds.
4. Decide UID cells from that snapshot. The header row is never a UID.
   - An empty UID on a row with a fingerprint is filled with a stored UID of the same content hash when that UID is missing from the column. Otherwise it gets a new id.
   - A value that is not 16 characters from `0123456789ABCDEFGHIKLMNOPQRSTVXYZ` is replaced.
   - A UID that appears on more than one row stays on the row whose mapped cells hash to the stored content hash. The other copies get new ids. When none match, every copy gets a new id and the stored UID is deleted, under the mass-delete guard. When two match, the earlier sheet row keeps it.
   - Those decisions fetch mapped cells for the duplicate and restored rows only.
5. A snapshot that needs a UID write or a formula rewrite sends that batch (`RAW` for UIDs) and stops. Nothing is committed. Wait another 10 quiet seconds and begin at step 1.
6. A snapshot that needs no write compares fingerprints with the index. One `values.batchGet` fetches mapped cells for added UIDs and for UIDs whose fingerprint differs, with `UNFORMATTED_VALUE`, date `SERIAL_NUMBER`, and `numberFormat.type` on those cells.
7. Read `version` again. A value other than V drops the fetch and commits nothing. When it is still V, commit and store V as the baseline.

A UID is taken from a two-column read whose `version` matched on both sides. That write becomes part of the snapshot only on a later read that also matches.

Deletions are UIDs in the index and absent from this UID column. The column is read whole so a missing id is visible. Unchanged rows are not downloaded. A sort that only moves rows updates the stored sheet row and emits no row event.

The first cycle of a new watch finds every UID absent from the index, fetches those rows, stores them, and appends one `SNAPSHOT` event. It does not append one `ADDED` per row.

### Mass delete

The commit counts UIDs that would be deleted. Above 30 percent of the stored UIDs, and at least 10 stored, the commit is skipped. Status becomes `needs_confirm` and `pending_deletes` is that count. Subscribers hear nothing about those rows.

`SyncNow` with `confirm` runs one cycle and applies the deletes. Without `confirm` it is an ordinary rescan and holds again while the guard still trips. A later cycle under the threshold commits on its own.

### Events

The fingerprint chooses which rows to fetch. The content hash chooses the event. The hash is 16-byte BLAKE2b of a format byte, then each included cell as name, kind, and value, in name order. Excluded cells and the sheet row are left out. The encoding is the service's own, so a protobuf upgrade does not mark every row updated.

| Situation | Event |
|---|---|
| UID absent from the index | `ADDED` |
| UID present, content hash differs | `UPDATED`, with the names of the cells that differ |
| UID present, content hash equal | no row event. The stored sheet row is updated when the row moved |
| UID in the index, absent from the column | `DELETED`, unless the mass-delete guard holds the commit |

`ADDED` and `UPDATED` carry the new row. `DELETED` carries the last stored row.

A mapped-column set that differs from the set stored at `StartWatch` appends `MAPPING` with the new list. A missing mapped column still pauses the watch rather than emitting that event.

`Subscribe` replays events after `after_seq`, then follows live. `ReadStored` returns the rows and `snapshot_seq` from one LMDB read transaction. The client subscribes with that sequence. A sequence older than the retained log is `OUT_OF_RANGE`, and the client reads the snapshot again. Each stream buffers 1,000 events, then fails so the client can resume. The sync loop does not wait on a stream. Events stay until 100,000 newer events exist for that watch or the event is 7 days old.

## LMDB

One file, `data/google-tables.lmdb`, holds every watch. LMDB is one memory-mapped file, with no cache of its own and no background threads. A read transaction is a snapshot, which is `snapshot_seq`. One writer at a time matches a commit of the rows that changed.

The first `StartWatch` for a spreadsheet and `sheetId` assigns a 4-byte `wid`.

| Sub-database | Key | Value |
|---|---|---|
| `idx` | `wid` + UID | fingerprint, content hash (16 bytes), sheet row (uint32) |
| `row` | `wid` + UID | `StoredRow` |
| `evt` | `wid` + sequence (uint64) | `RowChange` |
| `watch` | `wid` | spreadsheet id, `sheetId`, credentials file name, column names, exclusions, Drive `version`, changes page token, last sequence, status |

The sheet UIDs are sorted, then merged with a cursor on `idx` for that `wid`. The UID column itself is in sheet order. Only UIDs that differ are loaded from `row`. Memory follows the sheets in a cycle, not the number of watches.

`map_size` is set at open and is address space, not allocated disk. Subscriber transactions close at the end of each batch. The file stays large after a watch is removed. An operator reclaims space with an offline compacting copy. `StopWatch` stops the loop and leaves the prefix.

```protobuf
message StoredRow {
  string uid = 1;
  uint32 sheet_row = 2;
  repeated NamedCell cells = 3;
}

message NamedCell {
  string name = 1;
  irbis.table.v1.Value value = 2;
}
```

Empty cells are omitted. `name` is the metadata name. A streamed `irbis.table.v1.Row` is built from this at send time: `excel_id` is the column's letter in that cycle, and `columns[].name` is the metadata name a PDF binding already uses.

## gRPC

`proto/irbis/google_tables/v1/google_tables.proto`

`SelectHeader`, `SelectColumns`, and `ReadTable` use the same fields as the excel calls. The messages live in this file. The only imported proto is `irbis/table/v1/table.proto`. `ReadTable` fills empty UID cells, streams at most 100 rows per message, and uses the 50-row stop. It fails while a watch is running for that spreadsheet and `sheetId`. The watch is then the only writer.

```protobuf
syntax = "proto3";
package irbis.google_tables.v1;

import "irbis/table/v1/table.proto";

service GoogleTables {
  rpc OpenSpreadsheet(OpenSpreadsheetRequest) returns (SessionView);
  rpc SelectSheet(SelectSheetRequest) returns (SessionView);
  rpc SelectHeader(SelectHeaderRequest) returns (irbis.table.v1.Table);
  rpc SelectColumns(SelectColumnsRequest) returns (irbis.table.v1.Table);
  rpc SetColumnOptions(SetColumnOptionsRequest) returns (irbis.table.v1.Table);
  rpc EnsureIdentity(EnsureIdentityRequest) returns (irbis.table.v1.Table);
  rpc ReadTable(ReadTableRequest) returns (stream RowBatch);
  rpc Close(CloseRequest) returns (CloseResponse);

  rpc StartWatch(StartWatchRequest) returns (Watch);
  rpc StopWatch(StopWatchRequest) returns (Watch);
  rpc SyncNow(SyncNowRequest) returns (SyncReport);
  rpc GetWatch(GetWatchRequest) returns (Watch);
  rpc Subscribe(SubscribeRequest) returns (stream RowChange);
  rpc ReadStored(ReadStoredRequest) returns (stream StoredBatch);
}

message OpenSpreadsheetRequest {
  string spreadsheet_id = 1;
  string credentials_ref = 2;         // one path segment under the credentials dir
  uint32 header_candidate_limit = 3;  // 0 means 10
}

message SessionView {
  string session_id = 1;
  repeated SheetInfo sheets = 2;
  uint32 selected_sheet_index = 3;
  int32 selected_sheet_id = 4;
  repeated HeaderCandidate header_candidates = 5;
}

message SheetInfo {
  string name = 1;
  uint32 index = 2;
  int32 sheet_id = 3;
}

message HeaderCandidate {
  uint32 excel_row = 1;
  repeated irbis.table.v1.Column columns = 2;
}

message SelectSheetRequest {
  string session_id = 1;
  int32 sheet_id = 2;
  uint32 header_candidate_limit = 3;  // 0 means 10
}

message SelectHeaderRequest {
  string session_id = 1;
  uint32 excel_row = 2;
  uint32 subcolumn_row = 3;           // 0 means no subcolumn row
}

message SelectColumnsRequest {
  string session_id = 1;
  repeated irbis.table.v1.Column columns = 2;
}

message SetColumnOptionsRequest {
  string session_id = 1;
  repeated ColumnOption options = 2;
}

message ColumnOption {
  string name = 1;
  bool exclude_from_hash = 2;
}

message EnsureIdentityRequest { string session_id = 1; }

message ReadTableRequest { string session_id = 1; }

message RowBatch {
  repeated irbis.table.v1.Row rows = 1;  // at most 100
}

message CloseRequest { string session_id = 1; }
message CloseResponse {}

message StartWatchRequest {
  string session_id = 1;
  bool drive_push = 2;
}

message StopWatchRequest { string watch_id = 1; }
message GetWatchRequest { string watch_id = 1; }

message SyncNowRequest {
  string watch_id = 1;
  bool confirm = 2;
}

message Watch {
  string watch_id = 1;
  string spreadsheet_id = 2;
  int32 sheet_id = 3;
  string status = 4;                  // running, needs_confirm, column_missing, or the last error
  uint64 last_seq = 5;
  string drive_version = 6;
  repeated irbis.table.v1.Column columns = 7;
  uint32 pending_deletes = 8;
}

message SyncReport {
  uint64 last_seq = 1;
  uint32 added = 2;
  uint32 updated = 3;
  uint32 deleted = 4;
  bool held = 5;
}

message SubscribeRequest {
  string watch_id = 1;
  uint64 after_seq = 2;
}

message ReadStoredRequest { string watch_id = 1; }

message StoredBatch {
  uint64 snapshot_seq = 1;
  repeated irbis.table.v1.Row rows = 2;
}

message RowChange {
  uint64 seq = 1;
  ChangeKind kind = 2;
  string uid = 3;
  irbis.table.v1.Row row = 4;
  bytes content_hash = 5;
  repeated string changed_columns = 6;
  repeated irbis.table.v1.Column columns = 7;  // MAPPING
}

enum ChangeKind {
  CHANGE_KIND_UNSPECIFIED = 0;
  CHANGE_KIND_ADDED = 1;
  CHANGE_KIND_UPDATED = 2;
  CHANGE_KIND_DELETED = 3;
  CHANGE_KIND_MAPPING = 4;
  CHANGE_KIND_SNAPSHOT = 5;
}
```

`EnsureIdentity` returns the mapped columns and omits the two hidden columns. `StartWatch` requires a mapping and identity. `credentials_ref` rejects a value that contains a slash.

Errors are gRPC status codes with a short reason: unknown spreadsheet, permission denied, unknown session, unknown `sheetId`, header row not in the candidates, duplicate column name, bad `SelectColumns` entry, a service column that cannot be recreated, unknown watch, `confirm` when the status is not `needs_confirm`.

## Google access

The spreadsheet is shared with the service-account address. Scopes:

- `https://www.googleapis.com/auth/spreadsheets` reads cells, appends the hidden columns, writes UIDs, the formula, metadata, and the protected range
- `https://www.googleapis.com/auth/drive.metadata.readonly` reads `version` and registers `changes.watch`

A quiet file makes no Sheets call. A change costs one metadata search, one two-column read, UID writes for rows that need them, and one read of the changed rows.

`POST /drive/push` checks `X-Goog-Channel-Token`, answers 200, and restarts the quiet wait for the spreadsheets on the next `changes.list` page. The channel lives 24 hours and is renewed at 12 hours. `GOOGLE_TABLES_PUSH_URL` is that public HTTPS address. The gateway terminates TLS and forwards `/drive/push`.

## Layout

```text
proto/irbis/google_tables/v1/google_tables.proto

modules/google-tables/              Rust crate, one static binary
  src/
    server.rs                       gRPC, sessions, subscribers
    sheets.rs                       Sheets source and the cycle
    uid.rs                          16-character ids
    watch.rs                        version, quiet period, changes feed
    store.rs                        LMDB, prefixes, hash
    push.rs                         /drive/push
modules/excel-tooling/              Python, workbook.py is the Calc source
```

The binary links LMDB and talks HTTPS. It does not link Python. Port `50052` is on the compose network `private` and is not published. Callers use plaintext `google-tables:50052`. `data/` and the credentials directory are mounted. The LMDB file is `data/google-tables.lmdb`. The release zip that contains `rust-reg` also contains this binary and `nats-server`, as [sheets-source.md](../registration-server/sheets-source.md) describes.

```text
docker compose up google-tables
```

`cargo test -p google-tables` makes no network calls. Sheets tests use recorded `values.batchGet` and metadata-search payloads:

- a changed fingerprint fetches that row only
- a 50-row gap keeps the rows below it
- a `version` change between the two reads commits nothing
- a UID write stores no baseline until a later two-column read matches around its `version`
- a duplicated UID stays on the row whose content hash still matches
- deleting 30 percent or more sets `needs_confirm` until `confirm` is set

## Left out

Yandex Tables remain a later opener on excel-tooling.

This service does not draw PDFs and does not write the registration event file. A subscriber handles `RowChange` itself.

An Apps Script `onEdit` hook is not part of the watch. The fingerprint column already names the changed rows, and a deleted row is visible because the UID column is read whole. `onChange` reports `REMOVE_ROW` without a row index, so a script would still need that column read.
