# pdf-badges

PDF tooling for badge templates. A PDF is read into a page structure, set up as a template, and written back out. Each data row becomes one page. The writer is a Rust library that C, Java, and Python can wrap. The same library is served over gRPC.

See [docs/pdf-tooling](docs/pdf-tooling/README.md). The rest of the docs are indexed in [docs/README.md](docs/README.md).

Excel tooling is an extra feature: a reader that can supply that table from a workbook. See [docs/excel-tooling](docs/excel-tooling/design.md).
