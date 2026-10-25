# Sheet as the sync source

The admin can bind the device to a Google Sheet instead of the remote registration server. [porting.md](porting.md) step 13 is that work. The sync thread then applies row events from the local [google-tables](../google-tables/design.md) watcher. The remote catalog in [porting.md](porting.md) step 7 stays the other source. One event uses one source.

## What the operator chooses

On the event screen, after login, the source is `remote` or `sheet`.

`remote` is the path that exists today. The operator picks an event, `profile/synchdev` stores the project token, and the sync thread runs the download and upload queue.

`sheet` stores the spreadsheet id, the Google `sheetId`, the header row or the named columns, and the columns excluded from the fingerprint. The service-account JSON is a file on the device, the same file google-tables reads. Choosing the sheet opens the local event file the way step 5 does. The event id is the spreadsheet id. No project token is required, and the sync thread does not call kuprin.

The column names from the sheet are bound to visitor fields on that same screen. A name with no binding is stored on the row and is not a visitor column. Saving the binding restarts the watch.

## What runs on the device

Three static binaries, shipped in the same release zip as `rust-reg`. None of them needs a language runtime installed on the machine.

| Binary | Role |
|---|---|
| `rust-reg` | HTTP, print, and the sync thread, as today |
| `google-tables` | The watcher. Rust. Reads the sheet, writes UIDs, publishes each committed `RowChange` |
| `nats-server` | JetStream on `127.0.0.1` only. The subject is `sheets.<watch_id>` |

`rust-reg` starts the other two when the stored source is `sheet`, and stops them when the source changes or the binding is cleared. The tray still starts `rust-reg`.

google-tables is Rust because the watcher is HTTPS, JSON, and LMDB. LibreOffice is why excel-tooling is Python, and this component does not open Calc. A frozen Python bundle (PyInstaller, Nuitka, PyOxidizer) still embeds CPython and the Sheets client, and it has to be built again for Linux, both Macs, and Windows. The header, letter, and cell-type rules stay the ones in excel-tooling. This crate implements those rules and is checked against the same fixtures. It does not run the Python package.

## What the sync thread does

With source `sheet`, the sync thread subscribes to `sheets.<watch_id>` as a durable consumer named `registration-server`. It does not fill the remote download queue.

| Event | Local event file |
|---|---|
| `SNAPSHOT` | Replace the visitor set with the stored rows, in batches of 100, the same size as a `UniRegUser` page |
| `ADDED` | Insert the visitor |
| `UPDATED` | Replace that UID |
| `DELETED` | Remove that UID |
| `MAPPING` | Keep the new column list. Rows wait until the binding covers every name that was bound before |

The UID on the sheet is the visitor uid. A consumer that restarts resumes from its JetStream cursor. The LMDB file inside google-tables remains the sheet snapshot. JetStream is only the bus between the two processes.

Local HTTP and printing stay ahead of this consumer, on the same gate as a remote download. An upload queue is not started: the sheet is the source, and this device does not write visitor fields back except the UID column, which google-tables owns.
