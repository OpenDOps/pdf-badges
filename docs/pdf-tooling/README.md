# pdf-tooling

Everything that builds a PDF lives in this folder. The extractor turns a PDF into a page document. The React editor is the frontend that sets up that document as a template: rectangles, text, and layout. The writer draws the document back to a PDF, and [design.md](design.md) is how a table of rows fills the template into one page per row.

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
| [design.md](design.md) | Template boxes, mustache fields, column binding, multi-page render, gRPC, Docker |
| [grpc/implementation-plan.md](grpc/implementation-plan.md) | Build sequence: scan mustache spans, fill them, serve LoadTemplate and Render |
