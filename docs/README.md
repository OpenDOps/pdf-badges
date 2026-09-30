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
docker-compose.yml              both services on a local network
```

Today the Rust code still lives at the repository root (`src/`, `tests/`, the root `Cargo.toml`). That tree becomes `modules/pdf-tooling/` when the split is done. Behavior of the extractor and of `render-page` stays. The move is a path change, not a rewrite.

`excel-tooling` is Python. LibreOffice exposes workbooks through UNO, and the binding used to drive Calc headless is Python. The PDF writer stays Rust (`lopdf`, the current `struct_to_pdf` code). The two modules meet only at protobuf.

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

## What is already true

- `rust-reg` reads a PDF and writes `analysis.json` / `analysis.yaml`.
- `rust-reg render-page page.yaml -o out.pdf` draws one page from the target page structure.
- Placeholders such as `{{name}}` in a text box are drawn as literal text. Nothing binds them to data yet. The card fixture in `tests/fixtures/struct-to-pdf/card.yaml` is the shape the template step will fill.
