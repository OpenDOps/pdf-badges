# google-tables implementation plan

Build sequence for [design.md](design.md). The service watches a Google Sheet, keeps a UID on each data row, and queues the rows that changed since the last committed scan.

The crate is Rust, `modules/google-tables`, and the release ships one static binary. Steps 1 through 12 do not change excel-tooling. Steps 13 through 16 move it onto the same Rust grid crate. Unit tests are `cargo test -p google-tables` and do not call Google. The live sheet is step 10. It is ignored unless `GOOGLE_TABLES_LIVE=1`.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing.

Set **Status** to `not started`, `in progress`, or `done`.

## Event queue

The queue is the LMDB `evt` log from the design. `Subscribe` reads it. One process appends a `RowChange` in the same transaction as the row write, so a subscriber that wakes after the commit sees the row. Order is the sequence number. A subscriber that was down resumes from `after_seq`. This is the queue step 8 and the live sheet use.

NATS JetStream is the bus for a second process. The registration server, or anything else that must keep its own cursor while google-tables restarts, consumes a durable on `sheets.<watch_id>`. Retention is 100,000 messages or 7 days, the same window as the log. google-tables publishes after the LMDB commit. LMDB stays the store of the rows. JetStream does not hold the snapshot, and the sheet can be watched before JetStream exists. That handoff is step 12.

Redis Streams and Cloud Pub/Sub can carry the same messages. Redis is a second database this service does not otherwise need. Pub/Sub replay is a seek on a subscription, which is a poorer fit for "resume from this sequence" than a JetStream durable. Neither replaces the LMDB log.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Grid rules](#step-1-grid-rules) | Port header, letter, and cell-type rules, checked against the excel fixtures | not started |
| [2. Package and proto](#step-2-package-and-proto) | google-tables package, stubs, empty server | not started |
| [3. Recorded grid](#step-3-recorded-grid) | A Sheets payload becomes a header and typed cells | not started |
| [4. UID and hash](#step-4-uid-and-hash) | Ids, the fingerprint formula, the content hash | not started |
| [5. LMDB snapshot](#step-5-lmdb-snapshot) | Diff, event append, mass-delete hold | not started |
| [6. Cycle](#step-6-cycle) | Quiet version, narrow read, backoff, on a fake Google | not started |
| [7. Setup RPCs](#step-7-setup-rpcs) | Open through ReadTable, still on the fake | not started |
| [8. Watch and subscribe](#step-8-watch-and-subscribe) | The loop queues events, Subscribe reads them | not started |
| [9. Container](#step-9-container) | Image, compose, smoke against the fake | not started |
| [10. Live sheet](#step-10-live-sheet) | A real editable sheet, diffs queued from the last scan | not started |
| [11. Drive changes](#step-11-drive-changes) | One account feed wakes the watched file | not started |
| [12. JetStream](#step-12-jetstream) | A second process consumes the same events | not started |
| [13. Shared grid crate](#step-13-shared-grid-crate) | One Rust crate for header, letters, and cell types | not started |
| [14. Calc source](#step-14-calc-source) | Rust drives headless Calc through the UNO libraries | not started |
| [15. Excel service](#step-15-excel-service) | The excel gRPC server on that crate | not started |
| [16. Retire the Python reader](#step-16-retire-the-python-reader) | The image is the Rust binary plus LibreOffice | not started |

Shared rules:

- Behavior is [design.md](design.md). This file is the order to build it.
- The grid rules match excel-tooling. This crate does not call Python.
- A cycle that sees a new Drive `version` commits nothing.
- Protobuf conversion happens in `server.py`. The cycle speaks `StoredRow` and `NamedCell`.

## Step 1. Grid rules

[Back to summary](#summary)

The Rust crate finds a header, names columns, and classifies cells the way excel-tooling does. excel-tooling stays Python.

### Work

1. Port `columns.py`, `scan.py`, and `values.py` into `modules/google-tables`. Letters, header candidates, the subcolumn window, the 50-row gap, and `classify` with the 1899-12-30 epoch.
2. Feed the same fixtures the Python tests use. Do not shell out to Python.

### Test plan

| Scenario | Assert |
|---|---|
| `letters_and_headers` | `cargo test -p google-tables grid` matches the Python column, scan, and value cases, including the 50-row gap. |
| `excel_unchanged` | `pytest modules/excel-tooling` still passes. The Python package is not edited. |

### Done when

Both commands above are green.

## Step 2. Package and proto

[Back to summary](#summary)

The Sheets service compiles and serves an empty gRPC server.

### Work

1. Add `proto/irbis/google_tables/v1/google_tables.proto` as in the design. It imports `irbis/table/v1/table.proto` and does not import `excel.proto`.
2. Generate Rust with `tonic-build` from the crate build. Do not hand-edit the generated code.
3. The crate depends on `tonic`, `prost`, and `lmdb` (or `heed`). No Python.
4. `google-tables` listens on `50052` and answers `GetWatch` for an unknown id with `NOT_FOUND`.

### Test plan

| Scenario | Assert |
|---|---|
| `proto_roundtrip` | A `RowChange` of kind `ADDED` with one text cell serializes and parses to the same fields. |
| `unknown_watch` | `GetWatch` of `missing` returns `NOT_FOUND`. |

### Done when

`cargo test -p google-tables proto` passes, and the process starts.

## Step 3. Recorded grid

[Back to summary](#summary)

A saved Sheets response becomes the same grid the Calc source produces. No network.

### Work

1. Commit a trimmed `spreadsheets.get` payload under `tests/fixtures/`: a title row, a header on row 2 (`Name`, `City`, `Amount`), one empty row, and two data rows. Include a date, an integer, a blank, and a text cell.
2. The Sheets source implements `bounds`, `has_values`, `nonempty_rows`, `read`, and `release` from that payload shape.
3. Run the step 1 rules for candidates, `SelectHeader`, and data rows. Classify with the 1899-12-30 epoch.

### Test plan

| Scenario | Assert |
|---|---|
| `header_is_row_2` | Candidates include row 2. Selecting it yields `Name`, `City`, `Amount`. The title row is not data. |
| `date_is_civil` | The date cell is a `DateValue` of the civil day in the fixture, not the serial. |
| `empty_row_skipped` | The empty row is absent. Both data rows are present. |

### Done when

`cargo test -p google-tables grid_payload` passes and does not open a socket.

## Step 4. UID and hash

[Back to summary](#summary)

Ids and the fingerprint are pure functions. The sheet is still a fixture.

### Work

1. Draw 16 characters from `0123456789ABCDEFGHIKLMNOPQRSTVXYZ`.
2. Build the `ARRAYFORMULA` text from the mapped column letters, omitting `exclude_from_hash` names.
3. Content hash: 16-byte BLAKE2b of the format byte plus name, kind, and value in name order. The sheet row is not an input. An excluded cell is not an input.

### Test plan

| Scenario | Assert |
|---|---|
| `uid_shape` | 100 ids match the alphabet and the length. A collision inside the set retries. |
| `formula_skips_now` | With `Amount` excluded, the formula text joins `Name` and `City` and has no `Amount` letter. |
| `hash_ignores_move` | The same cells at sheet row 5 and sheet row 9 produce the same digest. Changing `City` changes it. |

### Done when

`cargo test -p google-tables uid` passes.

## Step 5. LMDB snapshot

[Back to summary](#summary)

The diff and the event log exist, with no Google and no gRPC. This is the queue later steps append to.

### Work

1. Open one LMDB file with sub-databases `idx`, `row`, `evt`, and `watch`.
2. Sort the scanned UIDs and merge with the `idx` cursor.
3. One write transaction puts changed rows, deletes missing UIDs, appends one `RowChange` each, and stores `version` only when asked.
4. Above 30 percent deletes, and at least 10 stored UIDs, commit nothing and set `needs_confirm`.
5. `ReadStored` is a read transaction that returns rows and `snapshot_seq`.

### Test plan

| Scenario | Assert |
|---|---|
| `add_update_delete` | A new UID appends `ADDED`. The same fingerprint appends nothing. A new hash appends `UPDATED` and lists the changed name. A UID dropped from the scan appends `DELETED`. |
| `move_is_quiet` | A new sheet row with the same hash updates `idx` and appends no event. |
| `mass_delete_holds` | 10 stored UIDs and 4 missing from the scan leave the rows in place, status `needs_confirm`, and the log unchanged. A commit with `confirm` appends 4 `DELETED` events. |
| `snapshot_seq` | The read transaction's sequence equals the last appended event. |

### Done when

`cargo test -p google-tables store` passes on a temporary LMDB file.

## Step 6. Cycle

[Back to summary](#summary)

The watch rules run against a fake Google client that returns scripted `version` values and recorded payloads.

### Work

1. Implement the seven steps in the design. The fake client records every `values.batchGet` range.
2. A UID write or a formula rewrite returns a new `version` and does not commit.
3. The next call, with that version stable around the two-column read, commits and stores it.
4. A `429` waits `min(2^n, 60)` seconds plus jitter before the next call. Tests use a fake clock.
5. The read walks past 50 empty fingerprint rows.

### Test plan

| Scenario | Assert |
|---|---|
| `narrow_fetch` | After the first commit, changing one fingerprint fetches that row's mapped cells and no other data row. |
| `write_does_not_commit` | A cycle that assigns UIDs writes them and leaves LMDB without those rows. The following stable cycle appends `SNAPSHOT` and stores `version`. |
| `version_moves` | A `version` change between the two reads commits nothing and does not store the new version. |
| `gap_keeps_the_tail` | A fixture with 50 empty rows and one row after them still returns that last row. |
| `backoff` | Two `429` responses produce waits of about 1s and 2s. A success resets the exponent. |

### Done when

`cargo test -p google-tables cycle` passes and the fake client is the only Google.

## Step 7. Setup RPCs

[Back to summary](#summary)

A caller can open a fixture spreadsheet, pick the header, exclude a column, ensure identity, and read rows. Writes go to the fake client.

### Work

1. `OpenSpreadsheet`, `SelectSheet`, `SelectHeader`, `SelectColumns`, `SetColumnOptions`, `EnsureIdentity`, `ReadTable`, `Close`.
2. `EnsureIdentity` tells the fake client to append the two hidden columns, the metadata keys, and the warning-only range. It does not shift existing letters.
3. `ReadTable` fills empty UIDs, streams at most 100 rows, and stops after 50 empty rows. That stop stays on this RPC. The watch does not use it.
4. Sessions expire 15 minutes after the last call.

### Test plan

| Scenario | Assert |
|---|---|
| `select_header` | The fixture's row 2 returns columns `Name`, `City`, `Amount` and no data rows. |
| `identity_appends` | After `EnsureIdentity` the fake sheet has two hidden columns at the right. `Name` is still column A. |
| `read_fills_uids` | `ReadTable` streams both data rows, each with a 16-character UID, and the fake client received one `RAW` batch for the empty UID cells. |
| `exclude_roundtrip` | `SetColumnOptions` of `Amount` makes the formula recorded by the fake client omit that column. |

### Done when

`cargo test -p google-tables session` passes, and steps 3 through 6 stay green.

## Step 8. Watch and subscribe

[Back to summary](#summary)

`StartWatch` runs the cycle on the quiet-period clock. Events land in the LMDB log. `Subscribe` is how a caller reads the queue.

### Work

1. `StartWatch` persists the mapping and starts the loop. The test clock fires the 10-second wait.
2. `Subscribe` yields the log after `after_seq`, then waits for new commits.
3. A sequence older than the retained window returns `OUT_OF_RANGE`.
4. `SyncNow` runs one cycle. `confirm` applies a held mass delete.
5. Restart the server object against the same LMDB file and show the watch resumes without `OpenSpreadsheet`.

### Test plan

| Scenario | Assert |
|---|---|
| `first_scan_is_snapshot` | The first stable cycle appends one `SNAPSHOT`. `ReadStored` returns both data rows and that sequence. |
| `edit_queues_updated` | The fake sheet changes `City` on one UID. The next stable cycle appends one `UPDATED` for that UID and names `City`. No other row is fetched. |
| `delete_queues_deleted` | Removing one UID from the fake column appends one `DELETED`. |
| `resume` | `Subscribe` with `after_seq` of the snapshot returns the later events and not the snapshot again. |
| `restart` | A new server on the same file serves `GetWatch` for that id and does not assign new UIDs to rows that already have them. |

### Done when

`cargo test -p google-tables watch` passes. A subscriber sees the queue move from the last committed scan to the next one without calling Google.

## Step 9. Container

[Back to summary](#summary)

The image runs the same server the tests call.

### Work

1. The image is `FROM scratch` or a distroless base plus the static `google-tables` binary and CA certificates. No Python and no LibreOffice.
2. Compose service `google-tables`, port `50052` on network `private` only. Mount `data/` and a credentials directory.
3. A smoke client runs the step 8 fixture through `StartWatch` and prints the queued kinds.

### Test plan

| Scenario | Assert |
|---|---|
| `smoke` | `docker compose up google-tables` becomes healthy. The smoke client prints `SNAPSHOT`, then `UPDATED` after the fixture edit. |
| `no_runtime` | The image has no `python` and no `soffice` binary. |

### Done when

The smoke command exits 0, and steps 1 through 8 still pass on the host.

## Step 10. Live sheet

[Back to summary](#summary)

A real spreadsheet is edited by hand. The service queues the diff from the last scan. This is the first step that uses a Google credential.

The credential is a service-account JSON key. An API key is not enough: an API key can read a sheet that is public to anyone with the link, and it cannot write the UID column, the formula, or metadata.

### Credential

1. Open [Google Cloud Console](https://console.cloud.google.com/) and create a project.
2. APIs & Services → Library. Enable **Google Sheets API** and **Google Drive API**.
3. IAM & Admin → Service Accounts → Create service account. No role is required on the project.
4. Open the account → Keys → Add key → Create new key → JSON. Save the file as `modules/google-tables/credentials/sheets.json`. That directory stays gitignored.
5. Copy `client_email` from the JSON.

### Sheet

Create a blank spreadsheet while logged in as a Google user. Name it `google-tables live`. Share it with `client_email` as **Editor**. The id is the path segment in `https://docs.google.com/spreadsheets/d/<id>/edit`.

Type this on the first sheet. Row 1 is the header. Leave row 4 empty on purpose.

| | A | B | C | D |
|---|---|---|---|---|
| 1 | Name | City | Amount | Clock |
| 2 | Ann | Oslo | 2 | `=NOW()` |
| 3 | Bo |  | 3 | `=NOW()` |
| 4 |  |  |  |  |
| 5 | Kim | Riga | 4 | `=NOW()` |

`Clock` is the volatile column. The live client marks it `exclude_from_hash`.

### Work

1. A live client, skipped unless `GOOGLE_TABLES_LIVE=1`, reads `GOOGLE_TABLES_CREDENTIALS` and `GOOGLE_TABLES_SPREADSHEET_ID`.
2. It calls `OpenSpreadsheet`, `SelectHeader` on row 1, `SetColumnOptions` for `Clock`, `EnsureIdentity`, and `StartWatch` with push off.
3. It stays subscribed and prints each queued event as one line: sequence, kind, UID, and the names of the changed columns.

### Test plan

Run the client. Wait until it prints one `SNAPSHOT` and `ReadStored` would list Ann, Bo, and Kim, each with a UID. In the browser, the UID and fingerprint columns are hidden at the right. `Name` is still column A. Then:

| Action in the sheet | Assert on the queue, after the 10 quiet seconds |
|---|---|
| Change Bo's `City` from empty to `Bergen` | One `UPDATED`. `changed_columns` is `City`. Ann and Kim are absent. |
| Add a row `Ida`, `Talinn`, `5` under Kim | One `ADDED` whose row contains those three cells and a new UID. |
| Delete Kim's row | One `DELETED` whose UID is the UID Kim had in the snapshot. |
| Change nothing and wait | No further event. |
| Edit a UID cell | The sheet prompts because of the warning-only range. After the edit is kept, the next cycle writes the previous UID back when the other cells still match that hash. |

A second scan with no edit must not print a second `SNAPSHOT`.

### Done when

The four edits above produce those four events, in that order, from one running process. `cargo test -p google-tables` stays green without credentials.

## Step 11. Drive changes

[Back to summary](#summary)

The account change feed wakes the sheet, so a quiet file is not polled every 60 seconds once push is on.

### Work

1. Store the `changes.list` page token on the watch record.
2. `POST /drive/push` checks the channel token, answers 200, and runs `changes.list`. A watched spreadsheet id restarts that sheet's 10-second wait.
3. Renew the channel at 12 hours. A shared drive uses its own token.
4. Push off keeps the 60-second poll from step 8.

### Test plan

| Scenario | Assert |
|---|---|
| `notice_wakes_one` | A recorded push plus a `changes.list` page that names the watched id schedules one cycle. A page that names another file schedules none. |
| `live_edit_moves_version` | With `GOOGLE_TABLES_LIVE=1`, editing Ann's `Amount` changes Drive `version`. The subscribed client prints one `UPDATED` for `Amount` without a 60-second poll. Skip this row when no public URL is set for the push channel, and run the poll path instead. |

### Done when

`cargo test -p google-tables push` passes on the recorded notice. The live row is recorded in the test output as passed or skipped.

## Step 12. JetStream

[Back to summary](#summary)

A second process receives the same queue. Build this when that process exists. Until then, `Subscribe` is the consumer.

### Work

1. After each LMDB commit, publish the new `RowChange` bytes to `sheets.<watch_id>` on `NATS_URL`.
2. A durable consumer named by the subscriber keeps its own sequence. Retention is 100,000 messages or 7 days.
3. Publishing is skipped when `NATS_URL` is empty. The LMDB log and `Subscribe` stay the source either way.

### Test plan

| Scenario | Assert |
|---|---|
| `durable_resume` | Compose includes `nats`. The step 8 fixture edit is published. A consumer that acks the snapshot, disconnects, and reconnects receives the `UPDATED` and not the snapshot again. |
| `unset_url` | With `NATS_URL` empty, step 8 still passes and opens no NATS socket. |

### Done when

`docker compose up nats google-tables` and the durable-resume check exit 0. Watching a sheet does not require NATS.

## Step 13. Shared grid crate

[Back to summary](#summary)

Header finding, column letters, and cell classification live in one Rust crate. google-tables and the rewritten excel reader both depend on it.

### Work

1. Move the step 1 rules into `modules/sheet-grid`. The crate has no gRPC, no HTTP, and no LibreOffice link.
2. `google-tables` depends on `sheet-grid` and deletes its copy of those rules.
3. Keep the source shape: `bounds`, `has_values`, `nonempty_rows`, `read`, `release`.

### Test plan

| Scenario | Assert |
|---|---|
| `grid_crate` | `cargo test -p sheet-grid` passes the letter, header, subcolumn, 50-row gap, and date cases from step 1. |
| `watcher_unchanged` | `cargo test -p google-tables` still passes. |

### Done when

Both commands are green, and `google-tables` contains no second copy of the scan.

## Step 14. Calc source

[Back to summary](#summary)

A Rust source opens a workbook in headless Calc and fills the same grid. LibreOffice has no crates.io client for this. `rust_uno`, shipped inside LibreOffice 26.2, builds extensions that load in the soffice process and links `librust_uno.so` plus the UNO C++ libraries. The Python reader instead connects to `soffice --accept=socket`. This step does that from Rust: the binary links those same UNO libraries and talks to the socket. It is not a static binary. Calc has to be installed beside it. `libreoffice-pure` writes and converts documents in pure Rust. It does not run `calculateAll()` or read Calc number formats, so it is not this source.

### Work

1. Start `soffice` with the same flags excel-tooling uses: headless, macros off, no external-link update.
2. Connect over the UNO socket. Implement `bounds`, `has_values`, `nonempty_rows`, `read`, and `release` by reading Calc after `calculateAll()`.
3. Map Calc number formats onto `sheet-grid` cell types, including the 1904 date system the Python reader already honors.
4. Accept `.xlsx`, `.xlsm`, `.xls`, and `.ods`. Reject anything else. A password-protected file stays closed until a password is provided.

### Test plan

| Scenario | Assert |
|---|---|
| `formula_is_value` | A fixture cell whose formula is `=1+2` comes back as the integer 3. |
| `date_matches_python` | The date fixtures in `modules/excel-tooling/tests` classify to the same civil dates the Python tests expect. |
| `macros_off` | An `.xlsm` opens with macro execution disabled and still returns the stored cells. |

### Done when

`cargo test -p excel-tooling calc` passes inside an image that contains LibreOffice. The host test without Calc does not start soffice.

## Step 15. Excel service

[Back to summary](#summary)

The excel gRPC service is a Rust binary on `sheet-grid` and the Calc source. The proto stays `proto/irbis/excel/v1/excel.proto`. Callers see the same RPCs.

### Work

1. Implement `OpenWorkbook`, `SelectSheet`, `SelectHeader`, `SelectColumns`, `ReadTable`, `ProvidePassword`, and `Close` with the session rules in [excel-tooling/design.md](../excel-tooling/design.md).
2. `SelectHeader` and `SelectColumns` call `sheet-grid`. `ReadTable` streams at most 100 rows and stops after 50 empty rows.
3. Keep the upload streaming, the 15-minute idle close, and the one-at-a-time Calc lock.

### Test plan

| Scenario | Assert |
|---|---|
| `titles_fixture` | Opening `titles.xlsx`, selecting the header, and `ReadTable` returns the two data rows, typed, with the title row omitted and the empty row skipped. |
| `same_columns` | For that fixture, column names and cell kinds match a recorded reply from the Python service. |

### Done when

`cargo test -p excel-tooling server` passes in the LibreOffice image. `cargo test -p sheet-grid` and `cargo test -p google-tables` stay green.

## Step 16. Retire the Python reader

[Back to summary](#summary)

The excel image runs the Rust binary next to soffice. The Python package is no longer the server.

### Work

1. The image installs LibreOffice Calc and copies the static-as-far-as-UNO-allows `excel-tooling` binary. It does not install `python3-uno` for this service.
2. The entrypoint starts soffice, then the Rust server, on port `50051` on network `private`.
3. Delete `excel_tooling/server.py`, `workbook.py`, `scan.py`, `columns.py`, and `values.py` once the Rust tests cover them. Leave the fixtures.

### Test plan

| Scenario | Assert |
|---|---|
| `smoke` | `docker compose up excel-tooling` serves the Rust binary. The smoke client against `titles.xlsx` prints the same two rows as the Python runbook. |
| `no_python_server` | The image has no `python -m excel_tooling` entrypoint. `soffice` is still on `PATH`. |

### Done when

The smoke command exits 0. google-tables and excel-tooling both link `sheet-grid`, and a header-rule change is a change to that crate alone.
