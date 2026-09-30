# pdf-tooling runbook

pdf-tooling is a Rust library. Other languages wrap it and draw through the same functions. The fill design is [design.md](design.md). Rust and Python embedding is [integration.md](integration.md). Building the C ABI is step 8 of [template/implementation-plan.md](template/implementation-plan.md).

The crate is `rust-reg` at the repository root. It later moves to `modules/pdf-tooling/`. The module is `struct_to_pdf`.

## Rust

Depend on the crate and call the library in process.

```rust
use rust_reg::struct_to_pdf::{fill, index_template, load_page, render_page};
```

`load_page` reads the page YAML. `index_template` lists the mustache fields and their spans. `fill` substitutes one box. `render_page` draws one page. `render_rows` draws one page per row of field values. That function is step 7 of the template plan. The C ABI below wraps it.

## C

Link the shared library or the static library and include `include/pdf_template.h`. Symbols start with `pdf_template_`. `pdf_template_index_bytes` indexes a page. `pdf_template_render` returns one PDF, one page per row. Free the PDF bytes with `pdf_template_bytes_free` and the handle with `pdf_template_free`. A failure code leaves the error text in `pdf_template_last_error`.

## Java and Python

Python embeds the shared library through `modules/pdf-tooling/pdf_template.py`. That wrapper indexes a template, accepts field values, and writes a PDF. The calls and the Rust equivalent are in [integration.md](integration.md). Java loads the same shared library with Panama or JNI.

## gRPC service

The Docker process on port 50052 is another caller of this library. `LoadTemplate` indexes the uploaded page. `Render` writes a PDF and returns its path. `RenderChunks` streams the PDF bytes instead.

```text
docker compose up --build pdf-tooling
```

`grpcurl -plaintext localhost:50052 list` prints `irbis.pdf.v1.PdfTooling`.
