# Inventarization plan

Inventory of the current tool and the gaps in [current-design.md](current-design.md). The build sequence that closes them, and writes font files, is [extractor/plan.md](extractor/plan.md). The product those gaps serve is [product-design.md](product-design.md).

## What exists

| Asset | State | Keep / fold |
|---|---|---|
| `main.rs` CLI, JSON and YAML write | Live | Keep. Add a `render` subcommand when the writer exists. |
| `structured_analyzer.rs` | Live extract path | This is the code to fix. |
| `data_structures.rs` | Live model | Extend in place: ids, pre-entered text, markup string, one coordinate system. See [document-schema.md](document-schema.md). |
| `utils.rs` BMP header | Live, used for Flate images | Keep behind image work. |
| `text_analyzer.rs`, `pdf_analyzer.rs`, `image_extractor.rs` | Not called from `main` | Do not add features here. Delete or quarantine once the live path has tests that cover what they were used to debug. |
| `print.pdf` and `*/analysis.yaml` | Manual fixtures | Promote `print.pdf` into a test fixture. Do not treat old YAML as a golden file until decode and coordinates change on purpose. |

## Groups

| Group | Gaps | When |
|---|---|---|
| Text processing | 1 decoding, 2 geometry, 3 style, then box unification | Next |
| Image processing | 4 graphics state, 5 filters and color | After text boxes are trustworthy |
| Test coverage | 6 no tests | Starts with the first text fix and stays on every change |

Text is next because the editor and the automatic fill tool both need boxes with real strings, real positions, and an id. Image matrices in the sample file are already outside the page; fixing them does not unblock text fill.

---

## Text processing

### 1. Decoding

Stop using the hardcoded CID map in `decode_text_bytes`.

For each font resource, parse the `ToUnicode` CMap already stored on `FontResource` (`begincodespacerange`, `beginbfchar`, `beginbfrange`). Decode `Tj` / `TJ` bytes with that map. Honor the codespace width (the sample font is two bytes per CID). If a font has no `ToUnicode`, fall back to the font `Encoding` (`WinAnsiEncoding`, `MacRomanEncoding`, a standard encoding). If neither exists, keep the bytes as a lossy Latin-1 string and record `decoded: false` on the box so the gap is visible.

Remove the duplicate map in `text_analyzer.rs` in the same change, or stop compiling that path.

**Done when** `print.pdf` yields `Сергей Курдюков` and `ООО БунтКофе` from the font CMap, and a second fixture with a WinAnsi font yields its Latin text without a new hardcoded table.

### 2. Geometry

Track a text state through the content stream instead of remembering only the last `Tm`:

- `BT` resets the text matrix and text line matrix
- `Tm` replaces them
- `Td` and `TD` move by the operand delta; `TD` also sets leading
- `T*` moves by leading
- `Tf` sets font and size, including size `0` (use the text-matrix scale)
- quote operators `'` and `"` if they appear

Each text-showing operator becomes a fragment: baseline origin, font id, font size, and a width from the font (widths array or a measured advance). The `len * font_size * 0.6` estimate goes away. Alignment uses the fragment’s own x and the page’s real width, not `595.92`.

Emit coordinates in the document system: top-left origin, y down, points. `posY` of a fragment is the top of the line (`pageHeight - baseline - ascent`), not the raw text-matrix Y.

**Done when** two lines no longer share one x unless the PDF placed them at the same x, and a `Td`-positioned fixture lands on the positions in the content stream.

### 3. Style

Fill style from data the extractor already has, then from the content stream:

- `BaseFont` / `FontDescriptor` `Flags`: bold and italic (the sample base font `LiberationSerif-Bold` must report bold)
- font family is the base name with the subset prefix (`AAAAAA+`) and the style suffix stripped
- underline only when the stream actually draws it (a line under the text); leave it false until that is detected
- `TJ` number operands are kerning adjustments, applied in the width calculation from step 2

Style stays on the run. It does not create a new box by itself.

### 4. Unify lines into wrapping boxes

This step runs only after 1 and 2. Fragments must already have correct text and correct edges.

**Build lines.** Sort fragments by baseline, then by x. Fragments on the same baseline (within about 0.5 pt) with the same direction form one visual line. A line has a left edge (min x), a right edge (max x + advance), a font size, and an ordered list of styled runs.

**Build boxes.** Walk lines from top to bottom. A line joins the open box when all of these hold:

- It shares an alignment edge with the box. Left-aligned: left edges within about 1 pt. Right-aligned: right edges within about 1 pt. If both edges match, treat the line as left-aligned. The schema also allows `center` and `middle`; the extractor does not emit those from PDF edges. A hand-authored page may set them.
- The vertical gap from the previous line is in a line-height band (about 0.8 to 2.5 times the box font size). A much larger gap starts a new box.
- The line’s horizontal span overlaps the box column. A line sitting in a different column starts a new box.

The box is then:

| Field | Value |
|---|---|
| `posX`, width | The column. Left alignment: x is the shared left edge, width is the widest line. Right alignment: the right edge is fixed, x is `right - width`. |
| `posY`, height | Top of the first line through the bottom of the last line |
| `alignment` | `horizontal: left` or `right` from the edge test. `vertical: top`. |
| `padding` | `top`, `right`, `bottom`, and `left` all `0`. |
| text | One string. Soft-wrapped lines of the same style join with a space. A style change stays inside the string as markup, not as another box. |

Example: the two right-aligned lines in `print.pdf` (`Сергей Курдюков`, then `ООО БунтКофе`, same right edge, 19 pt apart, same font) become one box, `alignment.horizontal: right`, `alignment.vertical: top`, padding `0`, with both lines in its text.

Inside the box, runs that differ from the box default are wrapped in the markup from [document-schema.md](document-schema.md): font name, font size, and font style (`bold`, `italic`, `underline`). Runs that match the default stay plain text. The box still has one `id`.

Assign `id` at this step (`p{page}-t{n}`), set `preentered` to the unified string, and leave `content` empty.

**Done when** those two sample lines are one box, a left-aligned paragraph fixture is one box, and a page with two columns produces two boxes. A bold word inside a regular line is a `<span>` (or `<font>`) inside that box, not a second box.

### Text order of work

1. Tests for decode on `print.pdf` and one WinAnsi fixture (group “test coverage” starts here).
2. ToUnicode / encoding decode.
3. Text state and real widths; coordinates from the trim’s top-left. Read `TrimBox` and `BleedBox` (with PDF defaults and parent `Pages` inheritance) into `page_size.bleeds`. `print.pdf` has only a `MediaBox`, so its bleeds are `0` and its page size stays the media size.
4. Style flags and family.
5. Line and box unification, ids, `preentered`, markup for runs that differ.

---

## Image processing

After text boxes are merged and covered by tests.

### 4. Graphics state

- Stack the graphics state on `q` and pop it on `Q`, including the current transformation matrix.
- Do not reset the matrix to the viewport after every `Do`.
- Resolve `Do` names that are Form XObjects by running their content stream with their own resources and matrix.
- If a page has no `Resources`, inherit from the parent `Pages` node.
- Emit the placement rectangle in the same top-left point space as text. Keep the raw matrix only as an optional debug field.

**Done when** every image in `print.pdf` lies inside the media box or is explicitly marked clipped, and a `q`/`Q` fixture restores the matrix.

### 5. Filters

- Accept `Filter` as a name or an array and decode in order (`FlateDecode`, `DCTDecode`, `JPXDecode`, `ASCII85Decode`, `LZWDecode` as they show up in fixtures).
- Use `lopdf` stream decode rather than a hand-rolled zlib check, so predictor and decode parms are handled.
- Write JPEG and JPEG 2000 bytes through unchanged. For raw samples, record color space honestly; produce BMP only for `DeviceRGB` and `DeviceGray`. Leave `ICCBased` and `DeviceCMYK` as a decoded file plus a `color_space` the writer can handle later, instead of a mislabeled RGB BMP.
- Write files under the `-o` directory, not a hardcoded `extracted/`.

---

## Test coverage

No separate “test project” at the end. Each item above lands with tests.

| Layer | What it locks |
|---|---|
| Unit | CMap parser, text-matrix math, line merge rules, markup builder, filter-name vs filter-array |
| Fixture | Small hand-built PDFs: WinAnsi text, `Td` moves, two-column text, `q`/`Q` around an image, Flate filter array |
| Golden | `print.pdf` document snapshot, updated only in the PR that intentionally changes coordinates or merging |

Golden YAML from `trace_operations/` and `final_check/` stays as historical output. It is not the expected value for new tests, because those files still contain the hardcoded decode, the shared x, and image positions outside the page.
