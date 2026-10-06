# pdf-tooling

Turns a page structure plus a `Table` ([table.md](../table.md)) into a PDF with one page per data row. The draw path is the Rust library `struct_to_pdf`: same page schema, same markup, same rectangles. Rust programs link that crate. C, Java, and Python wrap it through a C ABI. The same library also runs in Docker and speaks gRPC. How to link it is [runbook.md](runbook.md).

Today `render-page` draws one page and leaves `{{name}}` in the text. This design adds three steps in front of that draw: mark template boxes, list their mustache fields, bind each field to a column name.

The extract half of the crate (PDF in, analysis YAML out) stays a library and a CLI inside this module. It is not a gRPC method in this design. The service described here is the fill-and-draw direction.

## Template page

The input is one page object, the same YAML or JSON that `render-page` accepts ([struct-to-pdf/v0.md](struct-to-pdf/v0.md), [document-schema.md](document-schema.md)). A text box gains `template`, and may set `delimiters`:

```yaml
contents:
  - id: card-text
    template: true
    delimiters:
      open: "{{"
      close: "}}"
    posX: 10
    posY: 30
    width: 70
    height: 50
    text:
      preentered: '<span font="font1">{{name}}<br>{{surname}}</span>'
      content: null
```

`template: true` means the box is scanned and substituted. Boxes without the flag are copied onto every page with their text unchanged, including any `{{` they happen to contain. Image placements are never templates. A barcode may set `template` on its payload. A photo is not a template. `template` defaults to false so existing fixtures keep drawing placeholders as literal text.

`ifSt` on any content entry hides that entry for a row whose value for that key is missing, empty, `false`, or `0`. An absent or empty `ifSt` draws the entry. The key is not a mustache field. Barcode types are `ean13`, `code128`, and `qr`. A photo reads `photo.field` from the row. Both are specified in [document-schema.md](document-schema.md) and drawn when the registration server integrates the page.

`delimiters.open` and `delimiters.close` are the strings that wrap a field. Absent means `{{` and `}}`. The field keys are in [document-schema.md](document-schema.md#target-text-box). One box has one pair. Two boxes on a page may differ. The strings are matched exactly, so a box whose open is three braces does not also treat two braces as a field.

```text
open "{{"    close "}}"     {{name}}
open "{{{"   close "}}}"    {{{name}}}
open "<%"    close "%>"     <%name%>
```

The string that is scanned is `content` when it is a string, otherwise `preentered`. That is the same choice the writer already makes when it paints a box.

## Mustache fields

A field is the text between that box's `open` and `close`, trimmed. With the default pair, `{{ name }}` and `{{name}}` are the field `name`. The card fixture's `{{company name}}` is the field `company name`. Spaces inside the name are significant after trimming.

Rules for v1:

- No nested delimiters, no sections, no inverted sections, no partials. An open token whose first name character is `#`, `/`, `^`, `!`, `>`, or `&` is an error that names the box id. With the default pair, `{{#row}}` is that error.
- An empty pair (`open` immediately followed by `close`), or a second `open` before `close`, is an error that names the box id.
- `delimiters` on a box whose `template` is not true, or on an image placement or a photo, is an error that names the box id. A barcode may set `delimiters` when its `template` is true.
- The scan runs on the raw string, so a placeholder sitting inside `<span>…</span>` is found. Tag names are not special-cased.
- The same field name in several boxes, or twice in one box, is one field. The binding applies to every occurrence.
- The reply lists each field once, in first-seen order, with the box ids that contain it.

`LoadTemplate` returns that list and does not draw anything.

## Binding

The caller sends pairs of mustache field to column name. The column name is `Table.columns[].name`, compared exactly, including case.

```text
{{name}}          →  Name
{{surname}}       →  Surname
{{company name}}  →  Company
```

`SetBindings` checks that every field from the template appears once. A column name that is not on the table yet is allowed: the table arrives at render. A field left unbound fails `SetBindings`. A repeated field in the binding fails. An optional `format` on a pair overrides the default formatting for that field.

At `Render`, every bound column name must exist on the table. A table column with no binding is ignored. A bound cell that is missing on a row substitutes an empty string, and the page is still emitted.

Substitution replaces each `{{field}}` on the raw string, then the existing markup parser runs. Inserted text is escaped for the markup subset: `&` `<` `>` become `&amp;` `&lt;` `&gt;`. A date does not bring raw `<` in, but a text cell might. The surrounding tags stay, so `<span font="font1">{{name}}</span>` becomes `<span font="font1">Ann</span>` and still selects `font1`.

Default formatting:

| `Value` kind | Inserted text |
|---|---|
| `text` | the string |
| `int_value` | decimal, no fraction |
| `float_value` | shortest round-trip decimal |
| `bool_value` | `true` or `false` |
| `date` with `has_time` false | `YYYY-MM-DD` |
| `date` with `has_time` true | `YYYY-MM-DD HH:MM:SS` |

`format` uses strftime patterns for dates (`%d.%m.%Y`). v1 implements date `format` only. A `format` on a non-date value is rejected. A fixed number of decimals for floats can be added later without changing the message.

## Render

For each row, in order:

1. Clone the template page.
2. In every template box, substitute the row into the scanned string.
3. Leave non-template boxes as they are.

The clones are drawn as one PDF. The current `render_page` builds a one-page document. The writer grows `render_pages(&[Page], base_dir) -> Vec<u8>` that writes one catalog with N page objects, reusing embedded fonts and image XObjects across pages when the resource bytes match. Page size, bleeds, and paint order are the template's. `auto_scale` runs per page, because a long value may need a smaller size than a short one.

An empty table (columns present, no rows) fails `Render`. The caller gets no PDF rather than a zero-page file.

The existing CLI stays for tests and local use:

```bash
pdf-tooling render-page page.yaml -o out.pdf
```

A second subcommand can wait. Callers use the library below or the gRPC methods. Both draw through `struct_to_pdf`.

## Library

Indexing a template, filling its fields, and drawing one page per row are functions on `struct_to_pdf`. A Rust program depends on the crate and calls them in process.

Other languages use one C ABI, `include/pdf_template.h`. The crate builds an `rlib` for Rust, a `cdylib` shared library, and a `staticlib`. C links either library. Java loads the shared library with Panama or JNI. Python loads it with ctypes or cffi. Symbol names start with `pdf_template_`. The header is the contract those wrappers compile against. The build of that ABI is step 8 of [template/implementation-plan.md](template/implementation-plan.md).

The gRPC handlers are another caller of these functions. A wrapper that wants the Docker service speaks gRPC. A wrapper that links the library calls the C ABI. Rust and Python embedding is [integration.md](integration.md).

## gRPC

`proto/irbis/pdf/v1/pdf.proto`

```protobuf
syntax = "proto3";
package irbis.pdf.v1;

import "irbis/table/v1/table.proto";

service PdfTooling {
  rpc LoadTemplate(stream TemplateChunk) returns (TemplateView);
  rpc SetBindings(SetBindingsRequest) returns (TemplateView);
  rpc Render(RenderRequest) returns (stream PdfChunk);
  rpc Close(CloseRequest) returns (CloseResponse);
}

message TemplateChunk {
  oneof part {
    TemplateMeta meta = 1;   // first message
    bytes data = 2;          // a zip archive
  }
}

message TemplateMeta {
  string filename = 1;       // the page file name inside the zip
}

message TemplateView {
  string session_id = 1;
  repeated TemplateField fields = 2;
  repeated Binding bindings = 3;
}

message TemplateField {
  string name = 1;
  repeated string box_ids = 2;
}

message Binding {
  string mustache_field = 1;
  string column_name = 2;
  string format = 3;         // empty means the default
}

message SetBindingsRequest {
  string session_id = 1;
  repeated Binding bindings = 2;
}

message RenderRequest {
  string session_id = 1;
  irbis.table.v1.Table table = 2;
}

message PdfChunk {
  bytes data = 1;
}

message CloseRequest {
  string session_id = 1;
}

message CloseResponse {}
```

The zip contains the page file named by `filename` and any images or fonts it references, with paths relative to that file, matching how `render-page` resolves `source_path` and `extracted_path` today. A page with no external files is still sent as a zip of one entry, so the server has a single upload shape.

`LoadTemplate` extracts the zip into a session directory, parses the page, and returns fields with `bindings` empty. `SetBindings` replaces the full binding; it is not a patch. `Render` requires a binding already set. PDF bytes come back as a server stream of chunks.

Sessions are in memory plus that directory, with a 15 minute idle time. `Close` deletes the directory. The server does not keep the PDF.

`Render` takes the table inline. The default max message size is raised to 64 MiB. A later streaming `Render` can take rows as a stream if a sheet outgrows that; v1 does not need it for the card-per-row use.

## Code

The crate moves from the repository root to `modules/pdf-tooling/` without a behavior change. Names below are the paths after that move.

```text
modules/pdf-tooling/
  Cargo.toml
  Dockerfile
  src/
    main.rs                      CLI: analyze, render-page; grpc server behind a subcommand
    lib.rs
    extract/                     today's src/modules (analyzer)
    struct_to_pdf/               today's writer
      template.rs                scan {{ }}, substitute, escape
      document.rs                render_pages
      ffi.rs                     C ABI over the same functions
    grpc.rs                      LoadTemplate, SetBindings, Render
  include/pdf_template.h         header Java and Python wrap
  tests/
    fixtures/struct-to-pdf/      existing one-page fixtures, unchanged
    fixtures/template/           card with template: true, plus a tiny Table
```

`TextBox` gains `template: bool`, default false in serde so current YAML stays valid. `template.rs` is used by the gRPC layer and by a unit test that does not build a PDF: given the card string, the field list is `name`, `surname`, `company name`. A render test binds those three fields, feeds two rows, and checks the PDF has two pages and that the first page's text operators contain the first row's values rather than the braces.

## Docker

The image is a Rust build of this crate. No LibreOffice. Port `50052`. Fonts and images come only from the uploaded zip, so the image does not need the caller's filesystem.

```text
pdf-tooling serve --listen 0.0.0.0:50052
```

## Caller sequence

The two services do not call each other.

```text
excel = ExcelTooling.OpenWorkbook(file)
excel = ExcelTooling.SelectSheet(...)          # only if not the first sheet
excel = ExcelTooling.SelectHeader(session, row)
table = ExcelTooling.ReadTable(session)

pdf = PdfTooling.LoadTemplate(zip)
pdf = PdfTooling.SetBindings(session, [
        name → Name,
        surname → Surname,
        company name → Company,
      ])
bytes = PdfTooling.Render(session, table)
```

Column names in the binding are the header strings `SelectHeader` returned. The mustache names are the strings `LoadTemplate` returned. The caller is the only place that knows both.
