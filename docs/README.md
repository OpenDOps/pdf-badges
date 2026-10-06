# Document tooling

This repository builds PDFs from tabular data. Two services do the work. A caller sits between them. They share one protobuf table and nothing else.

```text
spreadsheet file
        │
        ▼
excel-tooling          LibreOffice Calc in Docker, gRPC
        │
        │  Table protobuf
        │  columns + typed cells
        ▼
pdf-tooling            Rust library other languages wrap; also Docker, gRPC
        │
        │  template page, boxes marked as templates,
        │  {{field}} bound to a column name
        ▼
multi-page PDF         one page per data row
```

`excel-tooling` reads a workbook, lets the caller pick a sheet and a header row, and returns the table. `pdf-tooling` is a Rust library. It loads a page structure, finds mustache fields in the boxes marked as templates, accepts a binding from column name to field, and draws one PDF page for each row. A Rust program links the crate. C, Java, and Python wrap the C ABI. The Docker service calls the same functions. Google Sheets and Yandex Tables will be further openers on `excel-tooling`. They produce the same `Table`.

The caller is not a third service yet. It opens the workbook, selects the sheet and the header, loads the template, sets the binding, and posts the table to the PDF service.

## Repository layout

Source is split by module. Documentation stays in one `docs/` tree.

```text
Cargo.toml                      workspace (pdf-tooling only)
proto/
  irbis/table/v1/table.proto    shared table
  irbis/excel/v1/excel.proto
  irbis/pdf/v1/pdf.proto
modules/
  pdf-tooling/                  Rust crate, Dockerfile, tests
  excel-tooling/                Python package, Dockerfile, tests
docs/
  README.md                     this file
  table.md                      shared Table contract
  pdf-tooling/                  PDF module: extract, template editor, writer
  excel-tooling/                reader, gRPC, later sheet adapters
  basic-excel-pdf-integration/  scan, fill, gRPC, excel caller
  registration-server/          on-device HTTP, print, and the first event-sync host
  event-sync/                   sync library: catalog slices and the C ABI
docker-compose.yml              both services on a local network
```

Today the Rust code still lives at the repository root (`src/`, `tests/`, the root `Cargo.toml`). That tree becomes `modules/pdf-tooling/` when the split is done. Behavior of the extractor and of `render-page` stays. The move is a path change, not a rewrite.

`excel-tooling` is Python. LibreOffice exposes workbooks through UNO, and the binding used to drive Calc headless is Python. The PDF writer stays Rust (`lopdf`, the current `struct_to_pdf` code). The two modules meet only at protobuf.

`registration-server` is the process on the venue device: one weak core, local HTTP and WebSocket, print jobs, and a timed download from the remote registration server. Browsers call that HTTP API and decode JSON natively. Login and the event choice are `registration-admin/`. The registration screens are `registration-form/`, a separate React Router app. The Registration gRPC service is compiled for tests only. The skeleton is `src/registration_server`, started with `rust-reg registration-server`. [registration-server/design.md](registration-server/design.md) is how that work is scheduled so local requests and printing keep the core. Drawing a badge is a separate crate, `modules/ticket-render`. The registration server links it and sends the bytes to CUPS. [registration-server/printing.md](registration-server/printing.md) is that split. Keeping the event file in step with the remote server is the `event-sync` library. The registration desk is its first host. An access-control station, a hall scanner, and a door reader bind the same library with a smaller catalog. [event-sync/design.md](event-sync/design.md) is that split.

## Documents

| Path | What it is |
|---|---|
| [table.md](table.md) | Column ids, typed cells, how a row is keyed |
| [excel-tooling/design.md](excel-tooling/design.md) | LibreOffice reader, sheet and header selection, gRPC, Docker |
| [excel-tooling/implementation-plan.md](excel-tooling/implementation-plan.md) | MVP build sequence for that reader |
| [excel-tooling/runbook.md](excel-tooling/runbook.md) | How to start the service and check a workbook by hand |
| [pdf-tooling/README.md](pdf-tooling/README.md) | PDF module: extract, template editor, writer |
| [pdf-tooling/design.md](pdf-tooling/design.md) | Template fill, the Rust library, and the gRPC service |
| [pdf-tooling/runbook.md](pdf-tooling/runbook.md) | Link the library from Rust, C, Java, or Python |
| [pdf-tooling/integration.md](pdf-tooling/integration.md) | Embed the library from Rust and from Python |
| [basic-excel-pdf-integration/implementation-plan.md](basic-excel-pdf-integration/implementation-plan.md) | First path from a workbook to a multi-page PDF |
| [registration-server/design.md](registration-server/design.md) | On-device process: local HTTP, WebSocket, and print keep the single core; remote sync runs behind them |
| [registration-server/network-quality.md](registration-server/network-quality.md) | Remote probe every five seconds: green, yellow, and red corner signal, and the timeouts that follow |
| [registration-server/login-and-token.md](registration-server/login-and-token.md) | Remote login, the project token, and the calls that send it |
| [registration-server/porting.md](registration-server/porting.md) | Log in once, store the token, switch to the selected event, sync, print, serve the local HTTP API, the registration UI, then the GitHub release and the checked update |
| [registration-server/db-design.md](registration-server/db-design.md) | Event SQLite file plus the in-memory full-text index and list pages, so search stays off the flash |
| [registration-server/db-implementation-plan.md](registration-server/db-implementation-plan.md) | Build sequence for that database: init, search, write, the list page, the JSON row |
| [registration-server/switch-implementation-plan.md](registration-server/switch-implementation-plan.md) | Build sequence for opening the selected event: init, background copy, switch, select, clear |
| [registration-server/token-implementation-plan.md](registration-server/token-implementation-plan.md) | Build sequence for the remote-server client: attach the token, probe it, drop a dead binding |
| [registration-server/sync-implementation-plan.md](registration-server/sync-implementation-plan.md) | Build sequence for the sync queue: one cap shared with the sockets, same-path downloads in order, cursors, then the event file |
| [event-sync/design.md](event-sync/design.md) | The sync library hosts bind: registration desk, access control, a hall scanner, a door reader |
| [registration-server/printing.md](registration-server/printing.md) | CUPS queues, IPP submit, and the ticket-render crate that draws a badge |
| [registration-server/printing-preload.md](registration-server/printing-preload.md) | What is decoded when the event opens, and which layer is drawn before the visitor's fields |
| [registration-server/printing-implementation-plan.md](registration-server/printing-implementation-plan.md) | Build sequence for that print path: layout, draw, find a queue, route, submit |
| [registration-server/http-implementation-plan.md](registration-server/http-implementation-plan.md) | Build sequence for the local HTTP API: each old route, what it does, and the `/api` route that replaces it |
| [registration-server/form-design.md](registration-server/form-design.md) | The registration form: one React engine for the desk page and the kiosk screens |
| [registration-server/pickers.md](registration-server/pickers.md) | Shared list for category, printer, country, city, and the phone calling code |
| [registration-server/registration-ui-design.md](registration-server/registration-ui-design.md) | Operator screens around that form: key, menu, visitor list, print, settings, printers |
| [registration-server/registration-ui-implementation-plan.md](registration-server/registration-ui-implementation-plan.md) | Build sequence for those screens: cookie gate, lazy routes, then one tested step per screen |
| [registration-server/form-implementation-plan.md](registration-server/form-implementation-plan.md) | Build sequence for that form: its own React Router app, tested from fixtures before printing and the local HTTP API |
| [registration-server/credentials.md](registration-server/credentials.md) | Credential file: JSON document and atomic replace |
| [registration-server/credentials-implementation-plan.md](registration-server/credentials-implementation-plan.md) | Build sequence for that file |
| [registration-server/admin-implementation-plan.md](registration-server/admin-implementation-plan.md) | Step 3: React admin, Russian catalog, static files, login, choose an event |

## What is already true

- `rust-reg` reads a PDF and writes `analysis.json` / `analysis.yaml`.
- `rust-reg render-page page.yaml -o out.pdf` draws one page from the target page structure.
- Placeholders such as `{{name}}` in a text box are drawn as literal text. Nothing binds them to data yet. The card fixture in `tests/fixtures/struct-to-pdf/card.yaml` is the shape the template step will fill.
