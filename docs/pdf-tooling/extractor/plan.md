# Extractor plan

Implementation plan for the PDF to document path. It closes the six gaps in [current-design.md](../current-design.md) and writes each embedded font out as a font file the writer can embed again.

The result is a target-schema document ([document-schema.md](../document-schema.md)). One page of that document is valid input for [struct-to-pdf v0](../struct-to-pdf/v0.md), including `source_path` on each embedded font. The gap list and the order of the text work stay the ones in [inventarization-plan.md](../inventarization-plan.md). This file is the build sequence.

## Output

`rust-reg input.pdf -o output` still writes `output/analysis.yaml` and `output/analysis.json`. It also writes:

```text
output/fonts/<id>.ttf    or .otf, .cff, .pfb
output/images/...
```

`schema_version: 1` is set only when the last step of this plan lands. Until then the file stays the current tree, so the writer keeps rejecting it.

## Step 0. One implementation

Done. The live functions from `structured_analyzer.rs` now live in the three modules. The stdout-only bodies are gone. `structured_analyzer` opens the PDF, calls the modules, and writes JSON and YAML.

| Module | After the move |
|---|---|
| `pdf_analyzer.rs` | Metadata, page size, resources walk, content-stream inflate and `Content::decode` |
| `text_analyzer.rs` | Text operators, decode, later the box merge |
| `image_extractor.rs` | Image resources, files, `cm` / `Do` rectangles |

**Done when** `cargo test` is empty but `cargo build` succeeds, `analyze_pdf` still returns the same tree as before the move, and nothing in `src/` prints a hex dump or writes `text_analysis.txt`.

## Step 1. Tests exist (gap 6)

Add `tests/fixtures/` and integration tests that call `analyze_pdf` on a fixture and assert fields. `print.pdf` is the first fixture. Do not copy `trace_operations/analysis.yaml` in as the expected value.

Every later step adds the tests named in its **Done when**. A step is not done if those tests are missing.

## Step 2. Decoding (gap 1)

Replace the hardcoded CID map. Parse the font’s `ToUnicode` CMap (`begincodespacerange`, `beginbfchar`, `beginbfrange`) and decode `Tj` / `TJ` with the codespace width. If there is no CMap, use `Encoding` (`WinAnsiEncoding`, `MacRomanEncoding`). If there is neither, keep a lossy Latin-1 string and set `decoded: false` on that text object.

**Done when** `print.pdf` yields `Сергей Курдюков` and `ООО БунтКофе` from the stored CMap, and a WinAnsi fixture yields its Latin text with no new hardcoded table.

## Step 3. Page boxes and text geometry (gap 2)

Read `TrimBox` and `BleedBox` with the PDF defaults and parent `Pages` inheritance. Fill `page_size` as trim size and `page_size.bleeds` as the four sides. `print.pdf` has only a `MediaBox`, so its bleeds are `0`.

Track text state: `BT`, `Tm`, `Td`, `TD`, `T*`, `Tf`, and `'` / `"` when present. Each showing operator is a fragment with its own baseline, font, size, and width from the PDF widths array (`Widths` or CID `W`). `TJ` numbers are kerning, applied to that width. Drop `len * font_size * 0.6` and the fixed `595.92` page width.

Coordinates are trim top-left, y down.

**Done when** a `Td` fixture matches the positions in its content stream, two lines share an x only when the PDF placed them there, and `print.pdf` reports bleeds of `0` with the current media size as the trim.

## Step 4. Style (gap 3)

From `BaseFont` and `FontDescriptor` `Flags`:

- `font_family` is the base name without the `AAAAAA+` subset prefix and without a style suffix
- `weight` is `bold` when the name or the flags say bold (`LiberationSerif-Bold` on `print.pdf`)
- `italic` comes from the name or the flags
- underline stays false until a drawn line under the glyphs is detected

Style is stored on the run. It does not create a second box.

**Done when** the `print.pdf` font resource has `font_family: LiberationSerif`, `weight: bold`, `italic: false`.

## Step 5. Font files

Write the embedded font program next to the analysis, and point `source_path` at it. This is the file [struct-to-pdf v0](../struct-to-pdf/v0.md) embeds. The extractor does not invent a font that was not in the PDF.

Walk `FontDescriptor`:

| PDF entry | File |
|---|---|
| `FontFile2` | The stream bytes, decompressed, as `.ttf` |
| `FontFile3` subtype `OpenType` | `.otf` |
| `FontFile3` subtype `Type1C` or `CIDFontType0C` | `.cff` |
| `FontFile` | Type 1, assembled from `Length1` / `Length2` / `Length3`, as `.pfb` |

Decode the stream with `lopdf` (filter name or filter array), not a hand-rolled `FlateDecode` check. One font object is written once even when several pages reference it. The path is relative to the analysis file:

```text
fonts/<family>-<weight>.ttf
fonts/<family>-<weight>-italic.ttf
```

The second form is used when `italic` is true.

Use the resource name as a suffix when two objects would share that filename. Set `source_path` to that relative path. Set `embedded: true`.

If the face has no font file stream, set `source_path: null` and `embedded: false`. Do not fail the extract. The writer will fail later if a text box needs that face.

`to_unicode` may stay on the resource during this step. After step 2 the text is already Unicode, so the CMap is not required in the target file; drop the raw CMap from the written YAML in step 7 so the analysis stays small.

**Done when** `print.pdf` produces a font file under `output/fonts/`, the `F6` resource’s `source_path` points at it, the file starts with a TrueType or OpenType signature (`0x00010000`, `OTTO`, or `true`), and a PDF whose font is not embedded yields `embedded: false` and `source_path: null`.

## Step 6. One box per column

After steps 2 and 3, merge fragments into the target text box. Rules are in the inventarization plan: same left or right edge, line-height gap, same column. Both edges matching is `horizontal: left`. `vertical` is `top`. Padding is `0` on every side.

The string is `preentered`. `content` is null. Runs that differ from the box default are `<span style="…">` inside that string. Assign `id` `p{page}-t{n}`.

**Done when** the two right-aligned lines in `print.pdf` are one box, a two-column fixture is two boxes, and a bold word inside a regular line is markup inside the box rather than another box.

## Step 7. Graphics state (gap 4)

- Push and pop the graphics state on `q` and `Q`, including the current matrix.
- Do not reset the matrix after each `Do`.
- Run Form XObjects with their own resources and matrix.
- Inherit `Resources` from a parent `Pages` node when the page has none.
- Image `id` is `p{page}-i{n}`. The rectangle is trim top-left, y down. `transformation_matrix` is optional and diagnostic.

**Done when** every image rectangle in `print.pdf` lies inside the media box or is marked clipped, and a `q`/`Q` fixture restores the matrix of the image after the `Q`.

## Step 8. Filters and image files (gap 5)

- Decode `Filter` as a name or an array, in order, via `lopdf`.
- JPEG and JPEG 2000 are written unchanged (`.jpg`, `.jp2`).
- BMP only for `DeviceRGB` and `DeviceGray`.
- `ICCBased` and `DeviceCMYK` are written as decoded bytes plus the real `color_space`, not as a mislabeled RGB BMP.
- Image `extracted_path` is relative to the analysis file, under `output/images/`, not `extracted/` at the working directory.

**Done when** a Flate filter-array fixture inflates, and a `DCTDecode` image from `print.pdf` is byte-identical to the embedded stream.

## Step 9. Target document

Write `schema_version: 1`. Each page has bleeds, text boxes with `id`, `alignment`, `padding`, `preentered`, and `content: null`, image placements with `id`, and font resources with `font_family`, `weight`, `italic`, `embedded`, and `source_path`. Drop the raw `to_unicode` CMap from the file.

**Done when** a page cut out of `analysis.yaml` is accepted by the struct-to-pdf v0 parser (once that parser exists), and the font `source_path` from step 5 resolves to a file beside the YAML.

## Order

| Step | Closes |
|---|---|
| 0 | Duplicate print analyzers |
| 1 | Gap 6, started |
| 2 | Gap 1 |
| 3 | Gap 2, plus bleeds |
| 4 | Gap 3 |
| 5 | Font files for the writer |
| 6 | Box unification, ids, markup |
| 7 | Gap 4 |
| 8 | Gap 5 |
| 9 | `schema_version: 1` |

Steps 2 through 6 are the text path and come first. Steps 7 and 8 are images. Font files are step 5, immediately after style, because they use the same `FontDescriptor` and the writer cannot embed a face without `source_path`.
