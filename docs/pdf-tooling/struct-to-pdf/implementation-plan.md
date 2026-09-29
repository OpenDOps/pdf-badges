# Struct to PDF v0, implementation plan

Step-by-step build of [v0.md](v0.md). Each step is one change: the work below, then the tests below, and those tests stay green afterward. v0 is done at the end of step 6, when `target/struct-to-pdf/card.pdf` exists. Steps 3 and 8 are image placement. They are specified here and are not required for that file. Step 7 sets `auto_scale` on the card text box and lengthens that text so the shrink is what makes it fit.

The preview stops if a bare angle bracket appears outside a fenced code block, because the preview treats it as HTML. Keep generics and tags inside fences that start at column 0.

## Summary

Set **Status** to `not started`, `in progress`, or `done`.

| Step | What it covers | Status |
|---|---|---|
| [1. Parse and validate](#step-1-parse-and-validate) | Load one page and reject a bad field | done |
| [2. Blank page and boxes](#step-2-blank-page-and-boxes) | Empty PDF with trim and bleeds | done |
| [3. JPEG images](#step-3-jpeg-images) | Place a JPEG, including in the bleed | done |
| [4. Plain text, one font](#step-4-plain-text-one-font) | One line from a font file | done |
| [5. Wrap, alignment, padding, leading](#step-5-wrap-alignment-padding-leading) | Line breaking and the four edges plus center and middle | done |
| [6. Markup and the card PDF](#step-6-markup-and-the-card-pdf) | The 90 by 130 mm card | done |
| [7. Fit text into the box](#step-7-fit-text-into-the-box) | Shrink the card’s long text, and the other long-text cases, into the box | done |
| [8. BMP images](#step-8-bmp-images) | BMP placement, same rectangle as JPEG | done |

Review findings, with severity, are in [Follow-up](#follow-up-review-findings).

Shared rules for every step:

- Tests live in `tests/struct_to_pdf.rs` and call the library. They do not shell out.
- A failure returns `Err` and does not create the output file.
- Lengths in fixtures may be `mm` or `points`. Convert with `pt = mm * 72 / 25.4` before any PDF number. Compare PDF rectangles with a tolerance of 0.01 pt.
- Do not read `trace_operations/analysis.yaml`. Fixtures go in `tests/fixtures/struct-to-pdf/`.

## Step 1. Parse and validate

[Back to summary](#summary)

Done. `load_page` and `parse_page` are in `src/struct_to_pdf/page.rs`. The 16 scenarios below are `tests/struct_to_pdf.rs`, and the card file is `tests/fixtures/struct-to-pdf/card.yaml` (and `card.json`).

### Work

1. Add `src/struct_to_pdf/mod.rs` and `src/struct_to_pdf/page.rs`. Declare the module from `src/main.rs` or `src/lib.rs`. If the package is only a binary today, add

   ```toml
   [lib]
   path = "src/lib.rs"
   ```

   and move `mod modules` plus `mod struct_to_pdf` into `src/lib.rs`. `main.rs` calls the library. The extractor’s behavior does not change.

2. In `page.rs`, define the serde types for one page: `page_size` (`width`, `height`, `units`, `bleeds`), `resources.fonts`, `resources.images`, `contents` as an untagged text box or image placement. Text boxes include `id`, rectangle, `alignment`, `padding`, and `text` (`font`, `font_size`, `leading`, `line_height`, `font_style`, `preentered`, `content`). This is the target text box in [document-schema.md](../document-schema.md).

3. `load_page` reads `.yaml` or `.json` and returns a `Page` or a `RenderError`. The error carries a context string such as `id=card-text`, `page_size.bleeds.left`, or `contents[0]`.

4. Before returning the page, reject all of the following. Stop at the first error.

   - The file has a `pages` key or a `metadata` key.
   - `units` is not `mm` or `points`.
   - `bleeds` is missing, a side is missing, a side is negative, or a side is not finite.
   - Trim `width` or `height` is missing, negative, or not finite.
   - A content entry has neither `text` nor `image`, or has both.
   - `id` is missing or empty.
   - Box `width` or `height` is negative or not finite.
   - Text alignment `horizontal` is not `left`, `center`, or `right`.
   - Text alignment `vertical` is not `top`, `middle`, or `bottom`.
   - A padding side is missing or negative, or `left + right` is greater than `width`, or `top + bottom` is greater than `height`.
   - Text box is missing `font`, `leading`, or `line_height`.
   - Image placement is missing `image.resource_name`.

5. Do not open font files, do not build a PDF, and do not check that `font` exists in `resources.fonts`. That check is step 4, once a string is drawn.

### Test scenarios

| Scenario | Fixture shape | Assert |
|---|---|---|
| `loads_card_yaml` | The card page from [v0.md](v0.md), fonts not required on disk yet | `Ok`. Trim 90 by 130 mm, units `mm`, every bleed 3, one content entry `card-text`, `preentered` is the card string, `content` is null. |
| `loads_same_page_as_json` | That page as JSON | Same fields as the YAML load. |
| `rejects_document` | A file with `pages:` and `metadata:` | `Err` whose context contains `pages`. |
| `rejects_missing_bleeds` | Page with no `bleeds` | Context contains `bleeds`. |
| `rejects_negative_bleed` | `bleeds.left: -1` | Context contains `bleeds.left`. |
| `rejects_bad_units` | `units: inches` | Context contains `units`. |
| `rejects_entry_with_no_kind` | A content object with only `id` and a rectangle | Context contains that `id`. |
| `rejects_missing_id` | A text box with no `id` | Context contains `id`. |
| `rejects_bad_horizontal` | `horizontal: justified` | Context contains the box `id`. |
| `rejects_bad_vertical` | `vertical: center` | Context contains the box `id`. `center` is horizontal only. |
| `rejects_padding_wider_than_frame` | `padding.left` 40 and `padding.right` 40 on a box of width 70 | Context contains the box `id`. |
| `rejects_missing_font` | Text box with no `font` | Context contains `font`. |
| `rejects_missing_leading` | Text box with no `leading` | Context contains `leading`. |
| `rejects_missing_line_height` | Text box with no `line_height` | Context contains `line_height`. |
| `rejects_image_without_resource` | Image object with no `resource_name` | Context contains the placement `id`. |
| `rejects_negative_box_width` | `width: -1` | Context contains the box `id`. |

No test in this step calls `render_page`.

## Step 2. Blank page and boxes

[Back to summary](#summary)

Done. A blank page with the card trim and 3 mm bleeds is written to `target/struct-to-pdf/blank-page.pdf`. The text box is not drawn yet.

### Work

1. Add `src/struct_to_pdf/geometry.rs`.

   - `to_points(value, units) -> f32` using `pt = mm * 72 / 25.4`. `points` is unchanged.
   - `media_size(page) -> (width_pt, height_pt)` is trim plus left/right and bottom/top bleeds, in points.
   - `trim_box(page) -> [f32; 4]` is `[left, bottom, left + trimWidth, bottom + trimHeight]` in PDF user space (origin at the media lower-left, y up).
   - `media_box(page) -> [f32; 4]` is `[0, 0, mediaWidth, mediaHeight]`.
   - `pdf_point(page, document_x, document_y) -> (pdf_x, pdf_y)` with `document_y` measured from the trim top, y down:

     ```text
     pdfX = bleeds.left + documentX
     pdfY = bleeds.bottom + (trimHeight - documentY)
     ```

     All four inputs are converted to points first. For a rectangle, pass `posY + height` as `document_y` when the caller wants the rectangle’s bottom.

2. Add `render_page` in `mod.rs`. It returns PDF bytes or a `RenderError`. `base_dir` is unused in this step. Build an `lopdf::Document` with one page:

   - `MediaBox`, `BleedBox`, and `CropBox` are all `media_box`.
   - `TrimBox` is `trim_box`.
   - Do not write `UserUnit`.
   - Content stream is empty.
   - Ignore `contents` in this step only if it is empty. If `contents` is not empty, return an error `contents not drawable yet` so a later step is what starts painting. The card fixture is not rendered here.

3. `save_pdf(bytes, path)` writes the bytes only after `render_page` returns `Ok`. The tests that expect `Err` assert the path does not exist.

### Test scenarios

| Scenario | Input | Assert |
|---|---|---|
| `mm_to_points` | `to_points(25.4, mm)` and `to_points(72, points)` | Both equal 72. |
| `card_media_and_trim` | Card `page_size` only, empty `contents` | Load the PDF with `lopdf`. One page. `MediaBox`, `BleedBox`, and `CropBox` are `[0, 0, 96mm, 136mm]` in points. `TrimBox` is `[3mm, 3mm, 93mm, 133mm]` in points. No `UserUnit`. |
| `zero_bleed_trim_equals_media` | 90×110 mm, all bleeds 0, empty contents | `TrimBox` equals `MediaBox`, which is `[0, 0, 90mm, 110mm]` in points. |
| `points_input_is_not_scaled` | width 200, height 100, units `points`, bleeds 10 on each side, empty contents | `MediaBox` is `[0, 0, 220, 120]`. `TrimBox` is `[10, 10, 210, 110]`. |
| `non_empty_contents_not_yet` | Card page with the text box still present | Step 6 draws this page. Extracted text contains the three placeholders. |

## Step 3. JPEG images

[Back to summary](#summary)

Done. `tests/fixtures/struct-to-pdf/jpeg-page.yaml` is rendered to `target/struct-to-pdf/jpeg-page.pdf`. The JPEG bytes are `tests/fixtures/struct-to-pdf/images/pixel.jpg`. Text still returns `text not drawable yet`.

Not required for `card.pdf`. The card test does not use it.

### Work

1. Add `src/struct_to_pdf/images.rs`.
2. When walking `contents`, an image placement looks up `resources.images[resource_name]`. Resolve `extracted_path` relative to `base_dir`, which is the directory of the page file.
3. `file_format: jpg`: read the bytes and add an image XObject with `Subtype` `/Image`, `Filter` `/DCTDecode`, `ColorSpace` `/DeviceRGB`, and `Width` / `Height` from the resource. Paint it with `cm` then `Do`. The `cm` translation is `pdf_point` of the rectangle’s top-left converted so the image’s top-left sits on the document `(posX, posY)` and its size is the box `width` and `height` in points. Do not read `transformation_matrix`.
4. Clip the page content to the media box so a placement with a negative `posX` (in the left bleed) is still drawn.
5. Any other `file_format`, a missing file, or an unknown `resource_name` is `Err` naming the placement `id`. The message for a non-JPEG format is `not in v0 yet` until step 8.
6. Paint in `contents` order. Text entries in this step are still `Err` with `text not drawable yet` if you have not started step 4. Keep the card fixture out of this step’s tests.
7. Each scenario below is a page file under `tests/fixtures/struct-to-pdf/`, loaded with `load_page`. Do not build these pages as strings in the test. The JPEG bytes stay at `tests/fixtures/struct-to-pdf/images/pixel.jpg`. `jpeg-page.yaml` is the page `jpeg-page.pdf` is rendered from. The other four files are that same page with one field changed.

### Test scenarios

| Scenario | Input | Assert |
|---|---|---|
| `jpeg_rect_inside_trim` | `jpeg-page.yaml`: one JPEG, `posX` 10 mm, `posY` 20 mm, `width` 30 mm, `height` 40 mm, bleeds 3 mm, units `mm`, `extracted_path: images/pixel.jpg` | The content stream’s image transform maps back to that rectangle in trim space, within 0.01 pt. |
| `jpeg_in_left_bleed` | `jpeg-bleed.yaml`: same page, `posX` −2 mm | The PDF still contains the `Do`. The translation x is `1 mm` in points (`bleeds.left + posX`). |
| `jpeg_missing_file` | `jpeg-missing.yaml`: `extracted_path` points at a file that is not there | `Err` context is the placement `id`. No PDF bytes returned. |
| `jpeg_unknown_name` | `jpeg-unknown.yaml`: `resource_name` is not in `resources.images` | `Err` context is the placement `id`. |
| `bmp_rejected_until_step_7` | `jpeg-bmp.yaml`: `file_format: bmp` | Step 8 draws this page. The rectangle matches the JPEG test. |

## Step 4. Plain text, one font

[Back to summary](#summary)

Done. `tests/fixtures/struct-to-pdf/text-and-image.yaml` places the JPEG and the Hello line on one page, rendered to `target/struct-to-pdf/text-and-image.pdf`. The font file is `tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf`. A string that contains a less-than sign is still rejected.

### Work

1. Add `src/struct_to_pdf/fonts.rs` and `src/struct_to_pdf/text.rs`.
2. Add a font-metrics dependency (`ttf-parser` is enough). Do not add a second PDF crate. `lopdf` stays the only PDF writer.
3. Resolve `text.font` to `resources.fonts`. Open `source_path` relative to `base_dir`. Missing key or missing file is `Err` with the box `id`.
4. Embed the TTF bytes in the PDF as a font the page’s resources can select (`Identity-H` plus a `ToUnicode` CMap built from the characters actually drawn). Read advances and ascender from the TTF. Do not use the extractor’s hardcoded CID map.
5. Choose the string: if `content` is a string, including `""`, draw that; otherwise draw `preentered`. If both are missing, `Err` with the box `id`.
6. This step rejects the string when it contains a less-than sign. Markup is step 6. The error names the box `id`.
7. Draw one line. Honor only `horizontal: left` and `vertical: top` here. The baseline y in document space is `contentY + ascent`, where `contentY` is `posY + padding.top`. `padding` may be non-zero; the other alignment values wait for step 5. Emit `BT`, `Tf` at `font_size` converted to points, `Tm` at `pdf_point` of that baseline, `Tj` with the UTF-16BE or the encoding that matches the `ToUnicode` CMap, then `ET`.
8. An empty string draws nothing and still produces a valid one-page PDF.

### Test scenarios

| Scenario | Input | Assert |
|---|---|---|
| `text_and_image` | `text-and-image.yaml`: the JPEG at 10 mm, 15 mm, 30 mm by 40 mm, then a Hello line at `posY` 60 mm | Extracted text contains `Hello`. The image transform maps back to that rectangle. `Do` comes before `Tj`. |
| `plain_ascii_roundtrip` | `text-hello.yaml`: one box, preentered Hello, content null, font file on disk, left/top, padding 0 | `lopdf` text extraction of the page contains `Hello`. |
| `content_overrides_preentered` | `text-override.yaml`: preentered old, content new | Extracted text is `new` and does not contain `old`. |
| `empty_content_draws_nothing` | `text-empty.yaml`: content is an empty string, preentered hidden | Page exists, extracted text is empty. |
| `missing_both_strings` | `text-missing-strings.yaml`: no preentered and no content | `Err` context is the box `id`. |
| `missing_font_file` | `text-missing-font.yaml`: `source_path` does not exist | `Err` context is the box `id`. No PDF. |
| `unknown_font_key` | `text-unknown-font.yaml`: `text.font` is `missing` | `Err` context is the box `id`. |
| `markup_rejected_until_step_6` | `text-markup.yaml`: preentered is the card string | `Err` context is the box `id`. |
| `baseline_at_top_plus_ascent` | `text-baseline.yaml`: box at `posY` 30 mm, padding 0, font size 5 mm | The text matrix y equals `pdf_point` of document y `30mm + ascent`. |
| `cyrillic_roundtrip` | `text-cyrillic.yaml`: preentered Сергей, font Liberation Serif Regular | The font has a glyph for each character. Extracted text equals Сергей. |
| `cyrillic_warns_on_latin_font` | `text-cyrillic-latin-font.yaml`: preentered Сергей, font `fonts/LatinOnly.ttf` which only maps Latin A | A warning names the box id and each Cyrillic character. `Err`. No PDF. |

## Step 5. Wrap, alignment, padding, leading

[Back to summary](#summary)

Done. Plain text wraps on spaces, a raw newline is a forced break, and lines are placed with left, center, right, top, middle, and bottom inside the padding. `tests/fixtures/struct-to-pdf/text-center.yaml` is rendered to `target/struct-to-pdf/text-center.pdf`. A less-than sign is still rejected. The markup tags and styles are listed in [document-schema.md](../document-schema.md#markup-tags-and-styles).

### Work

1. In `text.rs`, build a content rectangle:

   ```text
   contentX      = posX + padding.left
   contentY      = posY + padding.top
   contentWidth  = width  - padding.left - padding.right
   contentHeight = height - padding.top  - padding.bottom
   ```

   Convert to points before measuring against glyph advances.

2. Split the plain string on spaces into words. Pack words into lines of at most `contentWidth`. A single word wider than `contentWidth` stays one line and may extend past the content box. Clip drawing to the frame (`posX`, `posY`, `width`, `height`), not to the content box, so padding stays empty and the overflow is clipped by the frame.

3. There is no `br` tag yet. A raw newline in the plain string is a forced break, so step 5 can test breaks before the markup parser exists.

4. Baseline step is `line_height + leading`, both converted to points. Every line in this step uses the box values.

5. Place lines:

   | `horizontal` | Line x |
   |---|---|
   | `left` | `contentX` |
   | `center` | `contentX + (contentWidth - lineWidth) / 2` |
   | `right` | `contentX + contentWidth - lineWidth` |

   | `vertical` | Block top |
   |---|---|
   | `top` | `contentY` |
   | `middle` | `contentY + (contentHeight - blockHeight) / 2` |
   | `bottom` | so the last baseline plus descent ends at `contentY + contentHeight` |

   `blockHeight` is `lineCount * line_height + (lineCount - 1) * leading`.

6. If `font_style.underline` is true, stroke a line under that line from the line’s x to x + lineWidth. Padding is not stroked.

### Test scenarios

| Scenario | Input | Assert |
|---|---|---|
| `wraps_on_spaces` | Content width fits two words and not three. String `one two three` | Two shown lines. First line is `one two` or whatever the measured widths allow; the third word is not on the first line. |
| `long_word_stays_intact` | One word wider than `contentWidth` | One `Tj` whose text is the whole word. |
| `left_top` | Two lines, padding 0 | First baseline’s document x equals `posX`. First line’s top equals `posY`. |
| `right_edge` | `horizontal: right`, one line, padding 0 | Line’s right edge equals `posX + width` within 0.01 pt. |
| `bottom_edge` | `vertical: bottom`, two lines, padding 0 | The block’s bottom equals `posY + height` within 0.01 pt. |
| `center_middle_in_frame` | 70×50 mm box, padding 0, two short lines, `center` / `middle` | Each line’s center x equals the box center x. The block’s center y equals the box center y. |
| `padding_insets_left` | `padding.left` 10 mm, `horizontal: left` | Line x equals `posX + 10mm`. |
| `leading_adds_to_line_height` | `line_height` 6 mm, `leading` 0.5 mm, two lines | Distance between baselines is 6.5 mm in points. |
| `underline_strokes` | `underline: true`, one line | The content stream contains a stroke whose width equals the line width. |

## Step 6. Markup and the card PDF

[Back to summary](#summary)

Done. This is the definition of done. `tests/fixtures/struct-to-pdf/card.yaml` is rendered to `target/struct-to-pdf/card.pdf`. The two faces are `fonts/LiberationSerif-Regular.ttf` and `fonts/LiberationSerif-Bold.ttf`. `rust-reg render-page` writes the same page.

### Work

1. Parse `content` if it is a string, otherwise `preentered`, into a list of runs and breaks. The grammar is the table in [document-schema.md](../document-schema.md). Implement it as a small scanner, not an HTML library.

   Tags, written here without angle brackets so the preview does not treat them as HTML:

   - Text nodes decode the entities for less-than, greater-than, and ampersand. `{{name}}` stays those eight characters.
   - A `br` tag is a forced break. Two `br` tags in a row are a break, an empty line, and a break.
   - `b` and `bold` set weight bold until the matching closer. They do not change the font key.
   - `i` and `u` set italic and underline.
   - `span` with `font="font1"`, and optional `leading` and `line-height`, overrides those fields for the text inside the span. A missing attribute inherits. The font is written as `font="font1"`. The closer only ends the span.
   - A bare token such as `span font1`, or a font key written on the closer, is an error. Text that needs another face is wrapped in its own span, `font="font2"`.
   - A `leading` tag whose body is a number sets leading until the next `leading` tag or the end of the current span. It does not change font or line-height.
   - Any other tag, or a `leading` body that is not a number, is `Err` with the box `id`. Do not drop the tag and continue.

2. A run’s font key must exist in `resources.fonts` and the file must open. On the card, `{{name}}` and `{{surname}}` are inside the `font1` span. `{{company name}}` is inside the `font2` span. `bold` keeps `font2` and marks the run bold. Because `font2` is already the bold file, embed that file for the company line. Do not look up a second face by family name.

3. Wrap and align with step 5, using each line’s own `line_height + leading`. The step after a line is the value from the run that ends that line. An empty line from two `br` tags still consumes one step.

4. The card string produces four lines:

   | Line | Text | Font key | Step after the line |
   |---|---|---|---|
   | 1 | `{{name}}` | `font1` | box `line_height + leading` (6.5 mm), unless the span overrides |
   | 2 | `{{surname}}` | `font1` | same |
   | 3 | empty | box font, the breaks sit between the spans | box step |
   | 4 | `{{company name}}` | `font2` from its span, bold | none |

5. Place that block with `horizontal: center` and `vertical: middle` inside the 70×50 mm box.

6. `render_page` of the card returns bytes. The integration test writes them to `target/struct-to-pdf/card.pdf` only after `Ok`.

7. Add the CLI in this same step. `main.rs` gains `render-page` with a page path and `-o` for the output path. It calls `load_page`, then `render_page` with the YAML file’s directory as `base_dir`, then writes the output. On `Err` it prints the context and exits non-zero without creating the output file.

### Test scenarios

| Scenario | Input | Assert |
|---|---|---|
| `card_pdf_is_written` | `tests/fixtures/struct-to-pdf/card.yaml` and the two font files | `target/struct-to-pdf/card.pdf` exists, starts with `%PDF`, and `lopdf` sees exactly one page. |
| `card_trim_and_bleeds` | That PDF | `TrimBox` is 90 mm by 130 mm. Each bleed side, measured as the inset of `TrimBox` inside `MediaBox`, is 3 mm. |
| `card_placeholders_present` | Extract text from that PDF | The text contains `{{name}}`, `{{surname}}`, and `{{company name}}`. It does not contain the words `span`, `bold`, or `font1`. |
| `card_two_fonts` | Inspect the page’s font resources | Two embedded font programs. The name line’s font file bytes match `font1`. The company line’s font file bytes match `font2`. |
| `card_block_is_centered` | The scaled card from step 7 | Each non-empty line’s center x equals the box center x. The scaled block’s center y equals the box center y. |
| `card_blank_line` | The scaled card from step 7 | The gap from the last surname baseline to the first company baseline equals two scaled steps. |
| `unknown_tag_writes_nothing` | `preentered` is a `foo` tag around `x` | `Err` context is the box `id`. The output path is absent. |
| `leading_tag_changes_step` | A `leading` tag whose body is `2`, between two plain lines, page units mm | Baseline distance is `line_height + 2mm`. |
| `span_font_attribute` | A `span` with `font="font1"` around `Hi` | The drawn font is the `font1` file. |
| `rejects_bare_span_font` | A `span` whose font is the bare token `font1` | `Err` context is the box `id`. |
| `rejects_span_font_switch` | A closer that names `font2` after `span` | `Err` context is the box `id`. |
| `amp_escape` | `&amp;` | Extracted text contains `&`. |
| `cli_card` | Run `rust-reg render-page tests/fixtures/struct-to-pdf/card.yaml -o target/struct-to-pdf/card-cli.pdf` | Exit 0. The file matches the library test on trim size and the three placeholders. |

Copy the two Liberation Serif files into `tests/fixtures/struct-to-pdf/fonts/` and point `source_path` at them. The test does not download fonts.

## Step 7. Fit text into the box

[Back to summary](#summary)

Done. `card-text` has `auto_scale: true` and long paragraphs, so the authored block is taller than 50 mm and the drawn text fits. `tests/fixtures/struct-to-pdf/card.yaml` is rendered to `target/struct-to-pdf/card.pdf`. The authored sizes in the page file stay as stored.

Do this after step 6. The card text box turns `auto_scale` on, and its string becomes long enough that the authored size does not fit. The same rules are the scaling design in [json-to-pdf.md](../json-to-pdf.md#fit-text-to-the-rectangle).

### Work

1. On a text box, read `auto_scale`. It sits on the rectangle, next to `alignment`, not inside `text`. Absent means false. `true` and `false` are the only values. Anything else is `Err` with the box `id`. Image placements do not use the field.
2. Add `fit_font_sizes` in `text.rs`. `render_page` calls it for every text box whose `auto_scale` is true, after markup is parsed and before that box is drawn. It does not write the page file. The authored `font_size` in the YAML stays as stored. Only the sizes used to wrap and paint change.
3. Lay the box out once at the authored sizes, with step 5 wrapping and step 6 runs. The text bounding rectangle is:

   ```text
   textWidth  = widest line
   textHeight = lineCount * line_height + (lineCount - 1) * leading
   ```

   Widths are glyph advances at each run’s font size. `line_height` and `leading` are the values of the run that ends each line, same as the baseline step. An empty string has no bounds. Draw nothing and do not divide by zero.
4. Compare that bounding rectangle to the content rectangle from step 5 (the structural frame inset by padding). Padding stays empty. If `textWidth` is at most `contentWidth` and `textHeight` is at most `contentHeight`, the scale is 1 and every authored size is kept. Do not enlarge text that already fits.
5. If the bounds exceed the content rectangle, estimate one scale:

   ```text
   scale = min(contentWidth / textWidth, contentHeight / textHeight)
   ```

   The scale is never greater than 1. Apply it and wrap again. If the lines are unchanged, stop. A taller run can sit in the middle of a line, so the first height misses it, and the next wrap can move that run to the end of a line and make the block taller. When the lines change and the block still overflows, estimate the scale from the new bounds and apply it again. Each pass only shrinks, and wrapping only merges lines, so this ends.
6. Multiply every font size inside the box by `scale`: the box `font_size` and each span font size. Multiply `line_height` and `leading` on the box and on each span by the same scale. Relative sizes stay. Then wrap, align, and draw at those sizes. `Tf` uses the scaled font size in points.
7. A non-empty string whose content width or content height is 0 cannot fit. That is `Err` with the box `id`.
8. On `card.yaml`, set `auto_scale: true` on `card-text`. Lengthen the string so that, at the authored sizes, the unscaled block is taller than the 50 mm content height. Keep the three placeholders and the same markup shape: name in `font1`, surname in `font1`, two forced breaks, company name bold in `font2`. Rewrite `target/struct-to-pdf/card.pdf`. Trim size, the three placeholders, and the two font files stay. `card_block_is_centered` and `card_blank_line` move to the scaled block, below.

```text
span font="font1": a long paragraph that contains {{name}}, forced break,
       a long paragraph that contains {{surname}}
close that span
two forced breaks, so one empty line, using the box font
span font="font2": bold, a long paragraph that contains {{company name}}
```

### Test scenarios

Each scenario is a page file under `tests/fixtures/struct-to-pdf/`, loaded with `load_page`. A shrink test first measures the text at the authored sizes and fails if that block already fits. The long string is what makes the scale smaller than 1.

| Scenario | Input | Assert |
|---|---|---|
| `card_long_text_shrinks` | `card.yaml`: `auto_scale: true` on `card-text`, long paragraphs as above | The unscaled block is taller than 50 mm. The drawn block’s width and height are at most the content box, within 0.01 pt. Every `Tf` is the authored size times `contentHeight / unscaledHeight`. Extracted text still contains `{{name}}`, `{{surname}}`, and `{{company name}}`. |
| `card_block_is_centered` | That same PDF | Each non-empty line’s center x equals the box center x (10 mm + 35 mm). The scaled block’s center y equals the box center y (30 mm + 25 mm). |
| `card_blank_line` | That same PDF | The gap from the last surname baseline to the first company baseline equals two scaled steps (the empty line plus the company line). |
| `shrinks_wide_word` | `auto_scale: true`, one long word wider than the content width, height large enough | The drawn line width equals the content width within 0.01 pt. `Tf` is the authored font size times `contentWidth / unscaledWordWidth`. |
| `shrinks_tall_block` | `auto_scale: true`, a long run of lines whose unscaled block height exceeds the content height, width large enough | The block height equals the content height within 0.01 pt. The baseline step equals the scaled `line_height + leading`. |
| `does_not_enlarge` | `auto_scale: true`, a short string that already fits | `Tf` equals the authored font size in points. |
| `off_keeps_authored_size` | The same long word as `shrinks_wide_word`, `auto_scale` absent | `Tf` equals the authored font size. The word may extend past the content box and is clipped by the frame. |
| `uniform_span_sizes` | `auto_scale: true`, a long line, a span font size twice the box font size, bounds too wide | Both drawn sizes are the authored sizes times the same scale. Their ratio stays 2. |
| `respects_content_box` | `auto_scale: true`, padding on the left and right, one long word wider than the content width | The fit target is the content width, not the frame width. The drawn line width equals that content width within 0.01 pt. |
| `empty_with_auto_scale_draws_nothing` | `auto_scale: true`, content is an empty string | Page exists, extracted text is empty. |
| `rejects_bad_auto_scale` | `auto_scale` is a number | `Err` context is the box `id`. No PDF. |

## Step 8. BMP images

[Back to summary](#summary)

Done. `jpeg-bmp.yaml` places an 8 by 8 pixel BMP on the same rectangle as the JPEG page. 24-bit and 32-bit sources are `DeviceRGB` with alpha dropped. 8-bit gray is `DeviceGray`. A 1-bit BMP and `jp2` still fail, and `jp2` still says it is not in v0 yet. The card tests are unchanged.

Not required for `card.pdf`. Do this after step 7 without changing the card assertions or the fit-text assertions.

### Work

1. In `images.rs`, accept `file_format: bmp`.
2. Decode the BMP header and pixels. 24-bit and 32-bit sources become `DeviceRGB` samples, dropping alpha. 8-bit gray becomes `DeviceGray`. Reject other bit depths with the placement `id`.
3. Add an image XObject with no `Filter` (or `/FlateDecode` around the raw samples). `Width` and `Height` come from the BMP. Paint with the same `cm` / `Do` math as step 3.
4. Remove the `not in v0 yet` error for `bmp`. Leave it in place for `jp2` and any other format.

### Test scenarios

| Scenario | Input | Assert |
|---|---|---|
| `bmp_matches_jpeg_rect` | The same rectangle as `jpeg_rect_inside_trim`, with a BMP of the same pixel size | The `cm` translation and scale match the JPEG test within 0.01 pt. |
| `bmp_rgb_color_space` | A 24-bit BMP | The XObject `ColorSpace` is `/DeviceRGB` and there is no `/DCTDecode`. |
| `bmp_gray` | An 8-bit gray BMP | `ColorSpace` is `/DeviceGray`. |
| `bmp_32bit_drops_alpha` | A 32-bit BMP whose first pixel has alpha | `ColorSpace` is `/DeviceRGB`. The samples are three bytes per pixel, and the alpha byte is not in the stream. |
| `bmp_bad_depth` | A 1-bit BMP | `Err` context is the placement `id`. |
| `jp2_still_rejected` | `file_format: jp2` | `Err` context is the placement `id`. The message contains `not in v0 yet`. |
| `card_still_passes` | Re-run `card_pdf_is_written` | Unchanged. |

## Follow-up. Review findings

[Back to summary](#summary)

Recheck of the writer after step 7. Severity is how wrong the result is, or how much it costs on a long box.

| Severity | Finding | Status |
|---|---|---|
| high | A span with a larger `line_height` can sit in the middle of a line. The first block height uses only the run that ends the line, so it misses that span. Shrinking can then move the span to the end of a line, and the block grows past the content box. On a 30 by 20 mm box the unscaled height was 12 mm and the drawn block was about 38 mm. | fixed |
| medium | Wrapping cloned the words so far and parsed the font file again for every trial word. `auto_scale` laid the box out twice, so a long paragraph did that work twice. | fixed |
| low | The scale converted the widest line from points to page units and back to points before dividing. That rounding can make the scale slightly too large, so a fitted word sits a fraction of a point past the content edge. | fixed |
| low | A character the font does not cover counts as width 0 while measuring. The fit can treat the line as narrower than the text that will be drawn. Paint then warns and returns `Err`, so no PDF is written. | fixed |
| low | Every drawn character is appended to the glyph list, including repeats. The width array and ToUnicode map drop duplicates. The list itself still grows with the string. | fixed |
| low | The plan and [v0.md](v0.md) described a 90 by 110 mm card while `card.yaml` and `card.json` are 90 by 130 mm. The text box is still 70 by 50 mm. | fixed |

The high finding is `tall_run_stays_inside_after_reflow`. After each scale the writer wraps again. If the lines changed and the block still overflows, it estimates the scale from the new bounds and applies it again. Each pass only shrinks, and wrapping only merges lines, so this ends. The card’s line height is uniform, so that box still takes a single scale.

The medium finding is one parse per font per box. Glyph advances are cached, and the next word adds its own width instead of remeasuring the line. The low scale finding divides the content width in points by the measured line width in points.

A missing character now fails while the advances are built, with the same warning as paint, before a width is used. A glyph id is stored once per font. The card trim in this plan and in [v0.md](v0.md) is 90 mm by 130 mm, matching `card.yaml`. The text box stays 70 mm by 50 mm at 10 mm, 30 mm.
