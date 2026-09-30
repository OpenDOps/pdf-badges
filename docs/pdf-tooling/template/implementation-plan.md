# pdf-tooling template implementation plan

Step-by-step build of the PDF half of [basic-excel-pdf-integration](../../basic-excel-pdf-integration/implementation-plan.md): scan a page YAML for mustache fields, fill those fields from string values, and draw one page per row. That draw lives in `struct_to_pdf`, beside `render_page`. A Rust program calls it directly. A C program calls the same functions through a C ABI, which is the boundary a Java or Python wrapper adapts. The gRPC service is another caller of those functions. The Excel caller stays in the integration plan. This plan stops when a Rust test, a C caller, and the `pdf-tooling` service can each draw one or many pages from the same template.

The service accepts strings keyed by mustache field name. It does not read a `Table` and it does not bind column names. That later API is [design.md](../design.md).

Each step is one change. The tests named in the step land with it and stay green. A failure returns `Err` and does not write a PDF.

Set **Status** to `not started`, `in progress`, or `done`.

The preview stops if a bare angle bracket appears outside a fenced code block. Keep tags and generics inside fences that start at column 0.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Template flag](#step-1-template-flag) | `template` on a text box, default false | done |
| [2. Scan one string](#step-2-scan-one-string) | One pass from braces to holes | done |
| [3. Delimiter pair](#step-3-delimiter-pair) | Open and close symbols on a template box | done |
| [4. Index the template](#step-4-index-the-template) | Field list and spans for every template box | done |
| [5. Fill from spans](#step-5-fill-from-spans) | Substitute values without scanning again | done |
| [6. Multi-page writer](#step-6-multi-page-writer) | One PDF, N pages, shared font and image objects | done |
| [7. Render rows](#step-7-render-rows) | Fill each row, then draw | done |
| [8. Embeddable library](#step-8-embeddable-library) | Rust crate, C ABI, and a thin Python wrapper | done |
| [9. Proto and process](#step-9-proto-and-process) | `pdf.proto`, tonic, `serve` | done |
| [10. LoadTemplate](#step-10-loadtemplate) | Zip in, index once, return field names | done |
| [11. Render and Close](#step-11-render-and-close) | Field values in, PDF path out; chunks are optional | done |
| [12. Docker service](#step-12-docker-service) | Image and compose on port 50052 | done |

Shared rules:

- Scan, fill, and drawing stay in `src/struct_to_pdf/`, the same module as `render_page`. There is one draw path. `src/pdf_server.rs` is the gRPC caller. The C ABI is a thin wrapper over `render_rows`. Moving the crate to `modules/pdf-tooling/` is a later path change.
- Tests call the library. They do not shell out. Scan and fill tests do not build a PDF. PDF tests use `lopdf::Document::load_mem`.
- `render-page` and `tests/fixtures/struct-to-pdf/card.yaml` keep drawing placeholders as literal text. A box is scanned only when `template` is true.
- The string that is scanned is `content` when it is a string, otherwise `preentered`. That is the choice `chosen_text` in `src/struct_to_pdf/text.rs` already makes.
- Spans are byte offsets into the original string. A template box chooses the open and close symbols. When it does not, the pair is `{{` and `}}`. A match starts on a character boundary. The original string is kept. Load time does not rewrite it.

## Step 1. Template flag

[Back to summary](#summary)

Accept `template` on a text box. Do not look at braces.

### Work

1. Add `template: bool` to `TextBox` in `src/struct_to_pdf/page.rs`.

2. Read it in `parse_content`, next to `parse_auto_scale`. The key is `template`.

   - Absent means false.
   - `true` and `false` are the only values. Anything else is `Err`. Context is the box id. Message is `template must be a boolean`.
   - A `template` key on an image placement is `Err`. Context is the box id. Message is `template is only valid on a text box`.

3. `load_page` and `parse_page` stay the entry points. Both YAML and JSON already go through `parse_value`, so the flag works for both.

### Test scenarios

Start `tests/template.rs`. These tests call `parse_page` and do not draw.

| Scenario | Fixture shape | Assert |
|---|---|---|
| `template_defaults_false` | A text box with no `template` key | `template` is false. |
| `template_true` | `template: true` | `template` is true. |
| `template_not_a_bool` | `template: yes` | `Err`. Context is the box id. Message contains `template`. |
| `template_on_image` | An image placement with `template: true` | `Err`. Context is the box id. |
| `card_yaml_unchanged` | `tests/fixtures/struct-to-pdf/card.yaml` | Loads. The text box has `template` false. `cargo test --test struct_to_pdf` stays green. |

### Done when

`cargo test --test template` and `cargo test --test struct_to_pdf` pass.

## Step 2. Scan one string

[Back to summary](#summary)

Turn one template string into holes. A hole is a field name plus the byte span of the braces in the original string.

```text
struct Hole {
    name: String
    from: usize    // index of the first brace
    to: usize      // index just after the closing braces
}
```

`scan(source, box_id)` returns the holes or a `RenderError`. It lives in `src/struct_to_pdf/template.rs`. Declare `mod template` from `src/struct_to_pdf/mod.rs`.

### Work

One forward pass. Do not use a regex, and do not search again later for the field name.

```text
i = 0
while i < source length:
    if source[i] and source[i+1] are the two opening braces:
        start = i
        inner starts at i+2
        close = the next two closing braces
        if there is no close: error, message contains "unclosed"
        if the inner bytes contain two opening braces: error, message contains "nested"
        name = inner trimmed
        if name is empty: error, message contains "empty"
        if the first character of name is one of hash, slash, caret, bang, greater-than, ampersand:
            error, message contains "unsupported"
        record Hole { name, from: start, to: close + 2 }
        i = close + 2
    else:
        i = i + 1
```

`RenderError.context` is `box_id` for every scan error.

Trimming uses the usual Unicode trim, so `{{ name }}` and `{{name}}` are the field `name`. Spaces that remain inside the name stay, so `{{company name}}` is `company name`. A single `{` that is not part of `{{` is literal text. A `}` that is not part of `}}` is literal text.

The slice `source[from..to]` is the raw placeholder, braces included. The bytes before `from` and after `to` are the literal neighbors.

### Test scenarios

Call `scan`. Do not build a page and do not build a PDF.

| Scenario | Source | Assert |
|---|---|---|
| `spans_cover_braces` | `Hello {{name}}` | One hole, name `name`. `source[from..to]` is the six-character placeholder. The bytes before it are `Hello `. |
| `trim_inside` | `{{ name }}` | Name `name`. The span covers the braces and the inner spaces. |
| `inner_space_stays` | `{{company name}}` | Name `company name`. |
| `repeated_name_two_spans` | `{{name}} and {{name}}` | Two holes, both named `name`, in order. Each span covers its own braces. |
| `literal_brace` | `a {b} c` | No holes. |
| `empty_braces` | `{{}}` | `Err`. Message contains `empty`. |
| `nested` | `{{na{{me}}` | `Err`. Message contains `nested`. |
| `unclosed` | `{{name` | `Err`. Message contains `unclosed`. |
| `section_tag` | `{{#row}}` | `Err`. Message contains `unsupported`. |
| `card_string` | The card `preentered` from `tests/fixtures/struct-to-pdf/card.yaml` | Hole names in order: `name`, `surname`, `company name`. |

### Done when

`cargo test --test template` passes. Done for the two-brace pair. Step 3 makes the open and close symbols selectable.

## Step 3. Delimiter pair

[Back to summary](#summary)

A template box chooses the symbols that wrap a field. `scan` uses that pair. One box has one pair. Two boxes on the same page may differ. The field list stays names and box ids. The symbols are not a field, and `LoadTemplate` does not return them.

When `template` is true and `delimiters` is absent, the pair is `{{` and `}}`. The card and the step 2 tests keep that pair.

```yaml
- id: card-text
  template: true
  delimiters:
    open: "{{"
    close: "}}"
```

Other pairs the scan accepts:

```text
open "{{{"   close "}}}"
open "<%"    close "%>"
```

The configured strings are matched exactly. A box whose open is three braces does not also treat two braces as a field.

### Work

1. Store `delimiter_open` and `delimiter_close` on `TextBox`. Read them from `delimiters.open` and `delimiters.close` in `parse_content`.

   - `template` true and no `delimiters` key: `{{` and `}}`.
   - `template` true and `delimiters` present: both `open` and `close` are required strings. An empty string is `Err`. Context is the box id. Message contains `empty`. A missing side, a non-string, or a `delimiters` value that is not an object is `Err`. Context is the box id. Message contains `delimiters`.
   - `template` is not true and `delimiters` is present: `Err`. Context is the box id. Message contains `delimiters`.
   - A `delimiters` key on an image placement is `Err`. Context is the box id. Message contains `delimiters`.
   - The strings are not trimmed. They are matched as written.

2. `scan(source, open, close, box_id)` replaces `scan(source, box_id)`. The step 2 tests pass `"{{"` and `"}}"`.

   Walk so each attempt starts on a character boundary. The remainder starts a hole when it begins with `open`. The close search starts just after that open token. `from` is the start of `open`. `to` is the index just after `close`. Inner text is the bytes between them.

   - No close: `Err`. Message contains `unclosed`.
   - Inner contains `open`: `Err`. Message contains `nested`.
   - Trimmed inner is empty: `Err`. Message contains `empty`.
   - The first character of the name is one of hash, slash, caret, bang, greater-than, or ampersand: `Err`. Message contains `unsupported`. This check does not depend on which pair the box uses.

   `RenderError.context` is `box_id`.

### Test scenarios

Parse tests call `parse_page`. Scan tests call `scan` and do not build a PDF. The percent-bracket source is the second line of the fence above.

| Scenario | Assert |
|---|---|
| `delimiters_default` | `template: true` and no `delimiters` key. The box stores `{{` and `}}`. |
| `triple_braces` | Open and close are three braces. `{{{name}}}` is one hole named `name`, and the span covers the whole placeholder. `{{name}}` in that same string is not a hole. |
| `percent_pair` | Open and close are the percent-bracket pair. The placeholder is one hole named `name`. A two-brace placeholder beside it is literal text. |
| `delimiters_without_template` | `delimiters` is set and `template` is absent. `Err`. Context is the box id. Message contains `delimiters`. |
| `empty_open` | `open` is an empty string. `Err`. Message contains `empty`. |
| `missing_close` | `open` is set and `close` is absent. `Err`. Message contains `delimiters`. |
| `delimiters_on_image` | An image placement with `delimiters` is `Err`. Context is the box id. |
| `step2_still_passes` | `cargo test --test template` still passes. The step 2 cases call `scan` with the default pair. |

### Done when

`cargo test --test template` passes.

## Step 4. Index the template

[Back to summary](#summary)

Scan every template box and publish the field list. The spans stay on the index. The page text is left as written.

```text
struct IndexedBox {
    box_id: String
    source: String
    holes: Vec<Hole>
}

struct TemplateField {
    name: String
    box_ids: Vec<String>
}

struct TemplateIndex {
    boxes: Vec<IndexedBox>
    fields: Vec<TemplateField>
}
```

`index_template(page)` returns a `TemplateIndex` or a `RenderError`. `IndexedBox::spans(name)` returns every `(from, to)` for that name in one box.

### Work

1. Walk `page.contents` from top to bottom.

2. Skip image placements.

3. Skip a text box whose `template` is false. Its braces are not fields.

4. For a template box, take `content` when it is `Some`, otherwise `preentered`. Both missing is `Err` with the box id and the same message `chosen_text` uses: `preentered and content are both missing`.

5. `scan` that string with the box's `delimiter_open` and `delimiter_close`. Push an `IndexedBox` even when the string has no holes, so a later fill still owns the box.

6. Build `fields` while scanning:

   - A name is listed the first time it is seen.
   - First-seen order is contents order, then left to right inside the string.
   - The same name in another box appends that box id.
   - Two holes of the same name in one box list that box id once.

7. A page with no template boxes indexes to empty `boxes` and empty `fields`. That is success.

8. Export `index_template` from `src/struct_to_pdf/mod.rs`.

Grouping by name is a view over `holes`. The stored order is the hole list. A helper used by tests returns every `(from, to)` for one name in one box.

### Test scenarios

Build the smallest page the parser accepts: page size, bleeds, empty font and image maps, and the boxes named below. Call `index_template`.

| Scenario | Fixture shape | Assert |
|---|---|---|
| `card_fields` | One template box whose text is the card string | Fields `name`, `surname`, `company name`, in that order, each with that box id. |
| `two_boxes` | Box `a` has `{{name}}`. Box `b` has `{{name}}` and `{{city}}` | Fields `name`, `city`. `name` lists `a` then `b`. `city` lists `b`. |
| `box_id_once` | One box with `{{name}} and {{name}}` | One field. `box_ids` has that id once. The box has two holes. |
| `skips_unmarked_box` | One box with `{{name}}` and `template` false, one template box with `{{city}}` | The only field is `city`. `boxes` has one entry. |
| `content_wins` | `preentered` is `{{name}}`, `content` is `{{city}}`, `template` true | The only field is `city`. The indexed source is the `content` string. |
| `empty_braces_names_the_box` | Template box `card-text` with `{{}}` | `Err`. Context is `card-text`. |
| `no_template_boxes` | The current card fixture, `template` false | Empty fields. |
| `missing_text` | `template: true` and both strings absent | `Err`. Context is the box id. |
| `two_pairs` | Box `a` uses the default pair and `{{name}}`. Box `b` uses the percent-bracket pair from step 3 around `city` | Fields `name`, `city`. `name` lists `a`. `city` lists `b`. |

### Done when

`cargo test --test template` passes. `index_template` of the current card fixture returns no fields.

## Step 5. Fill from spans

[Back to summary](#summary)

Build the filled string from the holes recorded in step 2. Do not scan the string again.

`fill(box, values) -> String`. `values` is a map from field name to string. A missing name appends nothing.

### Work

```text
capacity = source.len() - sum of hole spans + sum of escaped value lengths
out = empty string with that capacity
cursor = 0
for each hole, in order:
    append source[cursor..hole.from]
    append the escaped value for hole.name, or nothing when the name is absent
    cursor = hole.to
append source[cursor..]
```

The hole bytes are not copied. After `fill`, every hole still satisfies `source[from..to]` equal to the original placeholder. `fill` does not take `&mut IndexedBox`.

Escape while appending the value. Do not escape the literal slices copied from `source`. An ampersand in a value becomes the `amp` entity. A less-than becomes the `lt` entity. A greater-than becomes the `gt` entity. Encode the ampersand first so the entities are not encoded again. A value is not scanned for `{{`. A value that contains braces is copied through, after escaping.

Unknown names are not an error inside `fill`. Step 7 checks the row against the indexed field list before any box is filled. A name that belongs to another box and does not appear in this box's holes is simply unused here.

### Test scenarios

| Scenario | Assert |
|---|---|
| `fill_is_one_pass` | Source is `{{name}}`, then a forced break, then `{{surname}}`. Values Ann and Lee. The result is Ann, the break, Lee, and no braces. The holes still cover the original placeholders. |
| `repeated_name_same_value` | Two `name` holes. One value Ann. Both holes become Ann. |
| `value_with_braces_stays` | Value `{{nope}}` is copied through. The output contains those characters. `scan` of the original source is unchanged. |
| `escapes_markup` | A value containing an ampersand and both angle brackets is inserted as the three entities. The literal tags around the hole in `source` are unchanged. |
| `missing_value_is_empty` | The row omits `surname`. That hole contributes no characters. The literals on either side stay. |
| `capacity_holds` | The filled string's length equals the reserved capacity. No reallocation is required for that string. Build it with the computed capacity and assert the length matches. |

Put the forced-break source in a fence that starts at column 0:

```text
{{name}}<br>{{surname}}
```

### Done when

`cargo test --test template` passes. The fill test keeps the indexed holes and asserts they still point at the original placeholders after `fill`.

## Step 6. Multi-page writer

[Back to summary](#summary)

`render_pages(pages, base_dir)` returns the PDF bytes or a `RenderError`. It lives in `src/struct_to_pdf/mod.rs`, beside `render_page`. Multi-page drawing is that module. One catalog, one `Pages` node, one page object per input page. `render_page` becomes a call to `render_pages` with that single page, so the current tests keep their entry point.

### Work

1. An empty slice is `Err`. Context `page`. Message contains `no pages`. `render_page` never passes an empty slice.

2. Paint every page first, with the existing `images::paint_contents`. Each call starts its own font index, so each content stream names fonts `F1`, `F2`, and images `Im1`, `Im2`, the way one page does today. Do not draw into the document until every page has painted. A paint error fails the whole call and produces no PDF bytes.

3. Open one `Document` the way `render_page` does now: version `1.5`, one `Pages` object, then one `Page` object per painted page. Each page gets its own `Contents` stream, `MediaBox`, `BleedBox`, `CropBox`, `TrimBox`, and `Resources`. Those rectangles come from `media_box` and `trim_box` of that page. `Kids` is the page ids in order. `Count` is the number of pages.

4. Share font objects. `PendingFont` already carries the font file bytes and the glyphs that page used (`src/struct_to_pdf/text.rs`). Group by the file bytes. Union the glyph pairs. The same glyph id keeps the first Unicode scalar. Call `fonts::embed` once for that group. Each page's `Resources` font dict maps that page's resource name (`F1`, `F2`) to the shared object. Two pages that use one file and different characters still extract both characters, because the union is embedded once.

5. Share image objects. Group `PaintedPage` image streams by their encoded bytes. Add each distinct stream once. Each page's `XObject` dict maps that page's name (`Im1`, `Im2`) to the shared object.

6. `auto_scale` needs nothing new. `draw_text` already scales the string it was given, and each page is painted on its own.

### Test scenarios

| Scenario | Assert |
|---|---|
| `one_page_matches_render_page` | `render_pages` of the current card fixture has one page. Extracted text contains `{{name}}`, `{{surname}}`, and `{{company name}}`. Page count and that text match `render_page`. |
| `two_blank_pages` | Two empty pages, different trim sizes. The PDF has two pages. Each `TrimBox` matches its input. |
| `shared_font_object` | Two pages that both use `fonts/LiberationSerif-Regular.ttf` from the card fixture. The document contains one embedded font file stream, and both page resource dicts reference it. |
| `empty_pages` | `render_pages` of an empty slice is `Err`. Message contains `no pages`. |

Extract page 1 with `Document::extract_text(&[1])`, the same helper `page_text` in `tests/struct_to_pdf.rs` uses. Page 2 uses `&[2]`. Count pages from the `Pages` object's `Count`.

### Done when

`cargo test --test struct_to_pdf` and `cargo test --test template` pass.

## Step 7. Render rows

[Back to summary](#summary)

`render_rows(page, index, rows, base_dir)` returns the PDF bytes or a `RenderError`.

One row is a map from field name to string. Row order is page order.

### Work

1. Zero rows is `Err`. Context `page`. Message contains `no rows`. Do not call `render_pages`.

2. Before filling, check every row. A key that is not in `index.fields` is `Err`. Context `page`. Message contains the unknown name. A field that the row omits is an empty string at fill time. Do this check on the row map, not by scanning the template again.

3. For each row, clone `page`. `Page` already implements `Clone`. For each `IndexedBox`, find the text box with that id and set `text.content` to `fill(box, row)`. Leave `preentered` as the authored template. Leave every box that is not in `index.boxes` as the clone copied it, including a text box whose `template` is false.

4. An indexed box id that is not on the page is `Err`. Context is that id. Message contains `missing box`. The page and the template index are produced together, so this is a programming error, and it still fails closed.

5. Pass the clones to `render_pages` with `base_dir`. Substitution has already happened on the raw string. `draw_text` then runs `markup::parse` on `content`, because `chosen_text` prefers `content`.

6. Export `render_rows` from `src/struct_to_pdf/mod.rs`.

7. The `two_pages` test writes its PDF with `save_pdf` to `target/struct-to-pdf/template-pages.pdf` after `render_rows` returns `Ok`. That file is the copy to open by hand. The template box has `auto_scale: true`. Six rows: Ann Lee North, Bo Kim South, Ida Berg Oslo, then Alexandria Bartholomew-Worthington with a long company name, then Maximilian Supercalifragilisticexpialidocious with a long company name, then Chen Wu East. The two long-name pages are drawn smaller than the short-name pages.

### Test scenarios

Use a short template page, not the long card paragraphs. One text box, `template: true`, the card's three names, the Liberation fonts under `tests/fixtures/struct-to-pdf/fonts/`. Put it at `tests/fixtures/template/card.yaml`.

| Scenario | Assert |
|---|---|
| `two_pages` | Six rows: Ann Lee North, Bo Kim South, Ida Berg Oslo, two long-name rows, then Chen Wu East. The PDF has six pages. Each page contains its row and does not contain `{{`. Page 2 does not contain Ann. The long-name pages use a smaller font than Ann's page. `target/struct-to-pdf/template-pages.pdf` exists, starts with `%PDF`, and `lopdf` sees those six pages. |
| `unmarked_box_unchanged` | A second box, `template` false, text `{{name}}`. Both pages still extract `{{name}}` from that box, and the template box extracts the row value. |
| `unknown_field_errors` | A row that includes `nope` is `Err` before a PDF exists. |
| `missing_value_is_empty` | The row omits `surname`. The page still draws. Extracted text does not contain `{{surname}}`. |
| `zero_rows` | `Err`. Message contains `no rows`. |
| `holes_survive_render` | After `render_rows` of two rows, the indexed holes still cover the original placeholders. |

### Done when

`cargo test --test template` and `cargo test --test struct_to_pdf` pass. `target/struct-to-pdf/template-pages.pdf` is a six-page file a person can open. The short-name pages show Ann, Bo, Ida, and Chen at the authored size. The two long-name pages are drawn smaller.

## Step 8. Embeddable library

[Back to summary](#summary)

The functions from steps 4 through 7 are the module other programs link. Rust calls them on `struct_to_pdf`. C calls the same functions through a C ABI. Java and Python load that shared library and call the header. They do not get a second fill or a second writer. The gRPC service in the later steps is one more caller of `render_rows`.

### Work

1. Keep the public Rust surface on `rust_reg::struct_to_pdf`, which `src/lib.rs` already exports:

   - `load_page` and `parse_page`
   - `index_template`
   - `fill`
   - `render_page`, `render_pages`, `render_rows`

   A Rust program depends on this crate and calls those functions. It does not start `serve` and it does not speak gRPC.

2. The package builds three library kinds from the one crate: `rlib` for Rust, `cdylib` for a shared library, and `staticlib` for a C program that links the object file. The shared library is what Java and Python load. The crate stays `rust-reg` until the later move to `modules/pdf-tooling/`. Symbol names start with `pdf_template_`, so the file name of the shared library can change with that move without renaming the calls.

3. `src/struct_to_pdf/ffi.rs` and `include/pdf_template.h`. The header is the contract. Each call returns `0` on success and a non-zero code on failure. The message is thread-local, read with `pdf_template_last_error`. A panic at this boundary becomes that error and does not unwind into the caller.

```c
int pdf_template_index_bytes(
    const uint8_t *yaml, size_t len, const char *base_dir,
    PdfTemplate **out);
size_t pdf_template_field_count(const PdfTemplate *t);
const char *pdf_template_field_name(const PdfTemplate *t, size_t index);
int pdf_template_render(
    const PdfTemplate *t,
    const PdfTemplateValue *values, const size_t *row_lengths, size_t row_count,
    uint8_t **out_bytes, size_t *out_len);
void pdf_template_bytes_free(uint8_t *bytes, size_t len);
void pdf_template_free(PdfTemplate *t);
const char *pdf_template_last_error(void);
```

   `PdfTemplate` is an opaque pointer holding the page and its `TemplateIndex`. `base_dir` resolves font and image paths the way `render-page` does. `values` is packed in row order. `row_lengths[i]` is how many entries belong to row `i`. One row is one PDF page. `pdf_template_render` calls `render_rows`. An unknown field fails, a missing field is an empty string, and zero rows fails. On failure `out_bytes` is not set. The caller frees a successful buffer with `pdf_template_bytes_free` and the handle with `pdf_template_free`, each once.

4. A thin Python wrapper embeds that shared library. It lives at `modules/pdf-tooling/pdf_template.py` and uses ctypes against the header. `Template` indexes a page YAML, returns the field names, and renders rows of field to value into PDF bytes or a file. Java is still a later wrapper over the same header. Rust and Python embedding are written up in [integration.md](../integration.md).

### Test scenarios

The C cases are Rust tests that call the `extern "C"` functions the way a C program would. They do not shell out to a C compiler. The Python case runs `modules/pdf-tooling/tests/test_embed.py`, which loads the shared library in process.

| Scenario | Assert |
|---|---|
| `rust_crate_renders_rows` | A test calls `index_template` and `render_rows` through `rust_reg::struct_to_pdf` only. Two rows produce two pages. |
| `c_abi_two_pages` | `pdf_template_index_bytes` on the card template, then `pdf_template_render` of two rows. The PDF has two pages. Extracted text matches `render_rows` of the same rows. Field names come back as `name`, `surname`, `company name`. |
| `c_abi_unknown_field` | A value named `nope` returns non-zero. `pdf_template_last_error` contains `nope`. No PDF bytes are returned. |
| `c_abi_zero_rows` | `row_count` of zero returns non-zero. The message contains `no rows`. |
| `python_embed_writes_pdf` | The wrapper indexes the card template, lists `name`, `surname`, and `company name`, and writes `target/struct-to-pdf/template-python.pdf` from two rows. The file starts with `%PDF` and has two pages. Page 1 contains Ann, Lee, and North. Page 2 contains Bo, Kim, and South. An unknown field and zero rows fail in the wrapper. |

### Done when

`cargo test --test template` passes, including the C ABI cases and `python_embed_writes_pdf`. `cargo build` produces the shared library and the static library.

## Step 9. Proto and process

[Back to summary](#summary)

Add the service definition and a process that can listen. The server is a caller of the library from step 8. The RPCs return `UNIMPLEMENTED` until steps 10 and 11.

`proto/irbis/pdf/v1/pdf.proto`

```protobuf
syntax = "proto3";
package irbis.pdf.v1;

service PdfTooling {
  rpc LoadTemplate(stream TemplateChunk) returns (TemplateView);
  rpc Render(RenderRequest) returns (RenderResponse);
  rpc RenderChunks(RenderRequest) returns (stream PdfChunk);
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
}

message TemplateField {
  string name = 1;
  repeated string box_ids = 2;
}

message RenderRequest {
  string session_id = 1;
  repeated ValueRow rows = 2;
}

message RenderResponse {
  string path = 1;           // absolute path of the written PDF
}

message ValueRow {
  repeated FieldValue values = 1;
}

message FieldValue {
  string field = 1;
  string value = 2;
}

message PdfChunk {
  bytes data = 1;
}

message CloseRequest {
  string session_id = 1;
}

message CloseResponse {}
```

This file does not import `table.proto`. One `ValueRow` is one PDF page. Several rows are one document, in order. Spans are not in `TemplateView`.

### Work

1. Add the proto file above.

2. Add `build.rs` on the root crate. `tonic-build` compiles `proto/irbis/pdf/v1/pdf.proto` into `OUT_DIR`. Generated code is not committed. Dependencies: `tonic`, `prost`, `tokio` with `macros` and `rt-multi-thread`, `uuid` with `v4`, and `zip` for step 10. Dev-dependency: `tonic` for the test client, and `tonic-reflection` so `grpcurl` can list the service the way excel-tooling does.

3. `src/pdf_server.rs` holds the service. `src/lib.rs` declares it. `src/main.rs` gains a `serve` command beside `render-page`:

   ```text
   rust-reg serve --listen 0.0.0.0:50052
   ```

   `--listen` defaults to `0.0.0.0:50052`. The server enables gRPC reflection. Max receive size is 64 MiB, the same ceiling excel-tooling uses for an upload. The three RPCs exist and return `UNIMPLEMENTED`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `server_lists_rpcs` | An in-process server on `127.0.0.1:0`. A client calls `Close` with an unknown id and receives `NOT_FOUND`. |

### Done when

`cargo test --test pdf_grpc` passes. `rust-reg serve --listen 127.0.0.1:0` starts and can be stopped.

## Step 10. LoadTemplate

[Back to summary](#summary)

`LoadTemplate` is client streaming. It indexes the page once and stores the template index on a session. The reply is the field list from step 4.

### Work

1. The first message is `TemplateMeta`. A first message that is bytes, or a missing `filename`, is `INVALID_ARGUMENT`. `filename` is a single path component: no slash, no backslash, not `.` and not `..`. Anything else is `INVALID_ARGUMENT`.

2. Later messages are zip bytes. Append them. When the stream ends, unzip into `std::env::temp_dir()` under `pdf-tooling/` and a new UUID. Reject a zip entry whose resolved path is outside that directory. Reject a zip whose uncompressed size exceeds 64 MiB. Both are `INVALID_ARGUMENT`, and the directory is removed.

3. The page file is the entry named by `filename`. Missing entry is `INVALID_ARGUMENT`. `parse_page` reads it. The extension must be `.yaml`, `.yml`, or `.json`, the same rule as `load_page`. `base_dir` for later drawing is the directory that contains that file, so `source_path` and `extracted_path` resolve the way `render-page` resolves them when the page and the fonts sit together.

4. `index_template` the page. A parse error or a mustache error is `INVALID_ARGUMENT`. The directory is removed. No session is stored. `RenderError.context` and `message` are the status message.

5. Store the session: id, directory, `base_dir`, owned `Page`, owned `TemplateIndex`, and `last_used`. The reply is `session_id` and `fields` in index order, each with its `box_ids`. Spans stay on the session.

6. Sessions sit behind a mutex. Idle time is a `Duration` on the server, default 15 minutes. Each RPC drops sessions whose `last_used` is at least that old, deletes their directories, and then looks up the id. An expired id is `NOT_FOUND`. Tests construct the server with `Duration::ZERO`, so the next call after `LoadTemplate` already finds the session expired.

7. Clone `Page` and `TemplateIndex` out of the lock before any later draw. Step 11 draws outside the lock. This step only stores them.

### Test scenarios

The test builds the zip in memory from `tests/fixtures/template/card.yaml` and the font files that page references.

| Scenario | Assert |
|---|---|
| `load_returns_fields` | Fields `name`, `surname`, `company name`, and a session id. |
| `bad_mustache` | A template box with `{{}}` is `INVALID_ARGUMENT`. A following `Render` with that id is `NOT_FOUND`. |
| `missing_page_entry` | `filename` is `missing.yaml` and the zip has another name. `INVALID_ARGUMENT`. No session. |
| `filename_escape` | `filename` is `../card.yaml`. `INVALID_ARGUMENT`. |
| `zip_slip` | An entry whose path leaves the session directory is `INVALID_ARGUMENT`. The directory is gone. |
| `data_before_meta` | The first message is bytes. `INVALID_ARGUMENT`. |

### Done when

`cargo test --test pdf_grpc` passes. `LoadTemplate` of the card template returns the three field names.

## Step 11. Render and Close

[Back to summary](#summary)

`Render` fills and draws. `Close` drops the session.

### Work

1. `Render` looks up the session. Unknown or expired is `NOT_FOUND`. Refresh `last_used` on a hit.

2. Zero `rows` is `INVALID_ARGUMENT`. Message contains `no rows`.

3. Build one map per `ValueRow`. A repeated `field` inside one row is `INVALID_ARGUMENT`. Message contains `duplicate`. A `field` that is not in the indexed field list is `INVALID_ARGUMENT`. Message contains that name. An indexed field omitted from the row is left out of the map, and `render_rows` treats it as an empty string.

4. Clone the page, the template index, and `base_dir` under the lock. Call `render_rows` after releasing the lock. That is the same function the C ABI calls. Build the PDF bytes to the end before any reply, so a fill or draw error returns a status and leaves no file and no PDF bytes. `Render` writes that PDF under the temp `pdf-tooling` directory and returns the absolute path in `RenderResponse.path`. `Close` does not delete that file. `RenderChunks` is the optional stream: `PdfChunk` messages of 64 KiB, and a shorter last chunk. The server does not keep the PDF bytes.

5. `Close` deletes the directory and the map entry. A second `Close`, or `Render` after `Close`, is `NOT_FOUND`.

6. `LoadTemplate` and `Close` take the session mutex. `Render` holds it only for the lookup, the idle check, and the clone.

### Test scenarios

| Scenario | Assert |
|---|---|
| `render_one_page` | One `ValueRow` with Ann, Lee, and North. The PDF has one page. Extracted text contains those values and does not contain `{{`. |
| `render_two_pages` | Two rows. `Render` returns a file path. That PDF has two pages, in row order. |
| `render_chunks` | The same two rows on `RenderChunks`. One PDF, two pages. No file is written. |
| `unknown_field` | A value named `nope` is `INVALID_ARGUMENT`. |
| `duplicate_field` | Two `FieldValue` entries named `name` in one row is `INVALID_ARGUMENT`. |
| `empty_rows` | `Render` with no rows is `INVALID_ARGUMENT`. |
| `unknown_session` | A random id on `Render` and on `Close` is `NOT_FOUND`. |
| `close_then_render` | After `Close`, `Render` is `NOT_FOUND`. |
| `expired_session` | Server idle time is zero. `LoadTemplate`, then `Render`. The render is `NOT_FOUND` and the directory is gone. |

### Done when

`cargo test --test pdf_grpc` passes. A loaded card template renders two pages from two value rows.

## Step 12. Docker service

[Back to summary](#summary)

The image is a Rust build of this crate. No LibreOffice. Fonts and images come only from the uploaded zip.

### Work

1. `docker/pdf-tooling/Dockerfile`. Build stage `rust:1-bookworm`, runtime `debian:bookworm-slim`. The runtime contains the `rust-reg` binary and `ca-certificates`. Command: `rust-reg serve --listen 0.0.0.0:50052`. Publish port 50052.

2. `docker-compose.yml` gains a service `pdf-tooling`, port `50052:50052`, on network `private`. excel-tooling stays on that network with `50051` unpublished. A TCP healthcheck connects to `127.0.0.1:50052`.

3. Reflection is on, so `grpcurl -plaintext localhost:50052 list` prints `irbis.pdf.v1.PdfTooling`.

### Done when

`docker compose up pdf-tooling` is healthy, and `grpcurl` lists `irbis.pdf.v1.PdfTooling`. The behavioral checks stay `cargo test --test template` and `cargo test --test pdf_grpc`.

That is the PDF service. The same multi-page draw is the Rust crate and the C ABI from step 8. The caller that reads excel-tooling and posts `ValueRow`s is step 4 of [basic-excel-pdf-integration](../../basic-excel-pdf-integration/implementation-plan.md).
