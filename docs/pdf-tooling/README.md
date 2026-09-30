# pdf-tooling

Everything that builds a PDF lives in this folder. The extractor turns a PDF into a page document. The React editor is the frontend that sets up that document as a template: rectangles, text, and layout. The writer is a Rust library: it draws the document back to a PDF, and other languages wrap it. [design.md](design.md) is how a table of rows fills the template into one page per row. [runbook.md](runbook.md) is how Rust, C, Java, and Python link that library.

The shared table those rows come from is [../table.md](../table.md). The spreadsheet reader is [../excel-tooling/design.md](../excel-tooling/design.md).

| Doc | What it covers |
|---|---|
| [product-design.md](product-design.md) | The loop: upload, extract, edit the template, write a PDF |
| [react-editor.md](react-editor.md) | Frontend that sets up the template structure |
| [document-schema.md](document-schema.md) | Page YAML/JSON: boxes, text, images, coordinates |
| [current-design.md](current-design.md) | Extractor as it works today |
| [inventarization-plan.md](inventarization-plan.md) | Gaps in the extractor |
| [extractor/plan.md](extractor/plan.md) | Build sequence for the extractor, including font files |
| [json-to-pdf.md](json-to-pdf.md) | Writer: document to PDF |
| [struct-to-pdf/v0.md](struct-to-pdf/v0.md) | One-page writer, the current implementation target |
| [struct-to-pdf/implementation-plan.md](struct-to-pdf/implementation-plan.md) | Steps that landed that writer |
| [design.md](design.md) | Template boxes, mustache fields, column binding, multi-page render, the wrappable library, gRPC, Docker |
| [runbook.md](runbook.md) | Link the Rust library from Rust, C, Java, or Python |
| [integration.md](integration.md) | Embed the library from Rust and from the thin Python wrapper |
| [template/implementation-plan.md](template/implementation-plan.md) | Build sequence: scan mustache spans, fill them, export a Rust and C library, serve LoadTemplate and Render |
