# pdf-badges

Tools that turn a spreadsheet into a multi-page PDF of badges.

`excel-tooling` reads a workbook through LibreOffice Calc and returns a typed table over gRPC. The Rust PDF writer fills a template page from that table, one page per row. The two sides share a protobuf `Table` and nothing else.

The PDF analyzer and `render-page` writer still live at the repository root (`src/`, `tests/`). They move to `modules/pdf-tooling/` when that split lands.

See [docs/README.md](docs/README.md) for the layout and design notes.
