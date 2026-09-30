# Embedding pdf-tooling

Rust programs link the crate. Python loads the shared library through a thin wrapper. Both index a page, pass field values, and draw one PDF page per row. The gRPC service is a later caller of the same functions. How to start that service is [runbook.md](runbook.md).

The crate is `rust-reg` at the repository root. The module is `struct_to_pdf`. The C header is `include/pdf_template.h`. The Python wrapper is `modules/pdf-tooling/pdf_template.py`.

## Rust

Depend on the crate and call it in process.

```rust
use std::collections::HashMap;
use std::path::Path;

use rust_reg::struct_to_pdf::{index_template, load_page, render_rows};

let page = load_page(Path::new("page.yaml"))?;
let index = index_template(&page)?;
let rows = vec![HashMap::from([
    ("name".to_string(), "Ann".to_string()),
    ("surname".to_string(), "Lee".to_string()),
    ("company name".to_string(), "North".to_string()),
])];
let pdf = render_rows(&page, &index, &rows, Path::new("."))?;
```

`load_page` reads the page YAML. `index_template` lists the mustache fields. `render_rows` fills one clone per row and returns the PDF bytes. A Rust caller does not start `serve` and does not speak gRPC.

## Python

The wrapper embeds `librust_reg`. Build it with `cargo build`, then point `PDF_TEMPLATE_LIB` at `target/debug/librust_reg.dylib` on macOS, `target/debug/librust_reg.so` on Linux, or `target/debug/rust_reg.dll` on Windows. When that variable is unset, the wrapper looks in `target/debug` and `target/release` under the repository root.

```python
from pdf_template import Template

card = open("tests/fixtures/template/card.yaml", "rb").read()
with Template(card, "tests/fixtures/template") as template:
    print(template.fields())
    template.write(
        [
            {"name": "Ann", "surname": "Lee", "company name": "North"},
            {"name": "Bo", "surname": "Kim", "company name": "South"},
        ],
        "out.pdf",
    )
```

`fields` is the indexed names in first-seen order. `write` renders one page per row and stores the file. `render` returns the same PDF as bytes. An unknown field name, or zero rows, raises `PdfTemplateError`.

The check that this embedding draws the card template is:

```bash
cargo test --test template python_embed_writes_pdf
```

That writes `target/struct-to-pdf/template-python.pdf`.

## C

The header is the contract. Each call returns `0` on success. `pdf_template_last_error` is the message after a failure. `pdf_template_index_bytes` indexes a page. `pdf_template_render` returns one PDF, one page per row. Free the PDF bytes with `pdf_template_bytes_free` and the handle with `pdf_template_free`, each once. A Java wrapper can load the same shared library with Panama or JNI.
