# JSON / YAML to PDF

Draft of the writer that turns a document ([document-schema.md](document-schema.md)) into a PDF. It is the other half of the extractor: same serde types, opposite direction. The React editor does not link this code; it passes a document and receives a PDF.

v0 is one page only. Its implementation plan is [struct-to-pdf/v0.md](struct-to-pdf/v0.md). The multi-page document interface below is the later tool.

## Role

Input is a target-schema document (`schema_version: 1`) and the resource files it points at (images on disk, fonts the writer can embed). Output is one PDF whose trim size is `page_size`, whose bleed and media boxes come from `page_size.bleeds`, and whose image placements and text boxes are drawn from their rectangles.

The writer does not replay content-stream operators and does not read `transformation_matrix`. After an edit, the box rectangle is the layout.

A file with no `schema_version` is rejected. That is the current extractor tree (per-line text, mixed coordinates). Supporting it would freeze the limitations the extractor is about to remove.

## Interface

Rust library next to the extractor, plus a CLI subcommand:

```bash
rust-reg render document.yaml -o out.pdf
```

`document.json` is the same tree. Paths in `extracted_path` resolve relative to the document file.

Later the webview calls the same library through a small local HTTP handler (`POST` document, response PDF bytes). The render logic stays in the library, not in the handler.

## Page build

For each page, in order:

1. Create a page whose trim is `page_size.width` by `page_size.height`. Write `MediaBox`, `BleedBox`, `TrimBox`, and `CropBox` from `page_size.bleeds` as specified in [document-schema.md](document-schema.md).
2. Walk `contents` in array order (later entries paint above earlier ones).
3. Image placement: load `resources.images[resource_name]`, draw the bitmap into `(posX, posY, width, height)` in the document system (top-left of the trim, y down). Shift into PDF user space by the left and bottom bleed. Clip to the media box, so artwork in the bleed is kept.
4. Text box: choose the string (`content` if it is a string, otherwise `preentered`), parse markup, wrap, then fit font sizes when `auto_scale` is set, then draw. The same bleed shift applies.

Empty `contents` still produces a blank page of the trim size, with the bleed boxes written. `metadata` is written to the PDF info dictionary (`title`, `author`, `creator`). `producer` identifies this writer.

## Images

Decode the file by `file_format` (`jpg`, `jp2`, `bmp`). Place it in the rectangle with the image scaled to `width` and `height` (the editor already decided aspect ratio). Do not apply the historical `cm` matrix.

Missing files and unknown formats are errors that name the placement `id` and the path. The writer does not skip a missing image.

Color: JPEG and JPEG 2000 are embedded as those filters. BMP is read as RGB or gray. A resource still marked `ICCBased` or `DeviceCMYK` waits on the image-processing work in [inventarization-plan.md](inventarization-plan.md); until the extractor stores a file the writer can embed, render fails with that color space named, rather than guessing RGB.

## Text

### String and style

Resolve the string, then parse the markup subset:

- text, with `&lt;` `&gt;` `&amp;` decoded
- `<b>` `<i>` `<u>`
- `<font name="…" size="…">`
- `<span style="font-family; font-size; font-weight; font-style; text-decoration">`

Anything else is an error that includes the box `id` and the tag. The parser output is a list of runs: text plus resolved family, size in points, weight, italic, underline. Properties omitted on a tag inherit from the surrounding run, then from the box `font_name`, `font_size`, and `font_style`.

### Wrap

Inset the frame by `padding` to get the content box (`contentX`, `contentY`, `contentWidth`, `contentHeight` in [document-schema.md](document-schema.md)). Wrap each run sequence to `contentWidth`:

- Break at spaces. `<br>` is a forced break.
- A single word longer than `contentWidth` is drawn on its own line and may extend past the content box (the frame clips).
- The distance from one baseline to the next is `line_height + leading` of the run that ends the line, in page units, converted to points with the page.

### Align and draw

| `alignment.horizontal` | Line x |
|---|---|
| `left` | `contentX` |
| `center` | `contentX + (contentWidth - lineWidth) / 2` |
| `right` | `contentX + contentWidth - lineWidth` |

| `alignment.vertical` | Block y |
|---|---|
| `top` | First line at `contentY` |
| `middle` | Block top at `contentY + (contentHeight - blockHeight) / 2` |
| `bottom` | Last line ends at `contentY + contentHeight` |

Baseline sits below the top of the line by the font ascent. Underline is a stroke under the run when `underline` is set. Lines outside the frame (`posY` through `posY + height`) are clipped. Padding is empty; text does not draw in it.

The PDF y axis is flipped once at draw time, and the bleed offsets the media origin from the trim origin:

```text
pdfX = bleeds.left + documentX
pdfY = bleeds.bottom + (page_size.height - documentY)
```

`documentX` / `documentY` are the trim top-left system. For a rectangle, `documentY` in that formula is the bottom edge in document space (`posY + height`).

### Fit text to the rectangle

A text box can ask the writer to keep its text inside the structural rectangle by scaling font size. The flag is `auto_scale` on the box, next to `alignment` and `padding`, not inside `text`. Absent or `false` leaves every size as authored. `true` runs the fit below. Any other value fails with the box `id`. The writer applies this to every text box that has the flag while it generates the PDF. It does not write the new sizes back into the page file.

`fit_font_sizes` is the function. It runs after markup is parsed and after the first wrap, and before that box is drawn.

1. Measure the text bounding rectangle at the current font sizes. Line width is the sum of glyph advances at each run’s font size. The block size is:

```text
textWidth  = widest line
textHeight = lineCount * line_height + (lineCount - 1) * leading
```

`line_height` and `leading` are the values of the run that ends each line. An empty string has no bounds, so the box draws nothing.

2. The structural rectangle is the box frame (`posX`, `posY`, `width`, `height`). The fit target is the content rectangle, the frame inset by padding, so padding stays empty and the text stays inside the frame. If the text bounds are already inside that content rectangle, the scale is 1. Text that fits is not enlarged.

3. If the bounds are wider or taller than the content rectangle, estimate one scale and stop:

```text
scale = min(contentWidth / textWidth, contentHeight / textHeight)
```

The scale is never greater than 1. Apply it and wrap again. If the lines are unchanged, stop. A taller run can sit in the middle of a line, so the first height misses it, and the next wrap can move that run to the end of a line and make the block taller. When the lines change and the block still overflows, estimate the scale from the new bounds and apply it again. Each pass only shrinks, and wrapping only merges lines, so this ends.

4. Multiply every font size inside the box by that scale: the box `font_size` and each span font size. Multiply `line_height` and `leading`, on the box and on each span, by the same scale. A span that was twice the box size stays twice the box size. Then wrap, align, and draw at the scaled sizes.

A non-empty string whose content width or content height is 0 cannot fit. That fails with the box `id`.

### Fonts

Map the run’s font key (`font1`, `font2`, or the box `font`) to `resources.fonts`. Embed that resource’s `source_path`. `<bold>` uses the same font file and sets the bold weight; if the current font resource is already the bold file, use it as-is. If the key is missing, fail with the box `id`. Do not substitute a silent default; a wrong font changes wrapping.

Shape text as Unicode. The hardcoded CID map is an extractor bug, not something the writer repeats. Subset the embedded font to the code points used on the page.

## Errors

Fail the whole render on the first structural error, with a path-like context (`pages[0].contents id=p1-t1`). Do not emit a partial PDF.

| Condition | Error |
|---|---|
| Missing `schema_version` | unsupported current tree |
| Unknown markup tag or style property | box id |
| `content` and `preentered` both absent | box id |
| Image path missing | placement id |
| Font family not on the page | box id |
| Non-finite or negative box size | box id |
| Negative bleed side, or a missing `bleeds` object | page number |
| Padding wider or taller than the frame, or a negative padding side | box id |
| `alignment.horizontal` other than `left`, `center`, or `right`, or `alignment.vertical` other than `top`, `middle`, or `bottom` | box id |
| `auto_scale` present and not a boolean | box id |
| `auto_scale: true` and the content width or height is 0 while the string is non-empty | box id |

## Tests

- A one-page document with one left-aligned text box and one image: the PDF trim size matches `page_size`, and extracted text (via the extractor, once decode is fixed) contains the string.
- A page with `bleeds` of 9 pt on each side writes `TrimBox` inset by 9 pt from `MediaBox` and `BleedBox`. Reading that PDF back yields the same four sides.
- `content` overrides `preentered`. Null `content` uses `preentered`.
- A `<span style="font-weight: bold">` run selects the bold font resource.
- `horizontal: center` places each line in the middle of the content width. `vertical: middle` places the block in the middle of the content height. A non-zero `padding` insets both.
- An unknown tag fails and leaves no output file.
- The card text box has `auto_scale: true` and long paragraphs that contain `{{name}}`, `{{surname}}`, and `{{company name}}`. At the authored size the block is taller than the 70×50 mm content box. The drawn block fits that content box, every font size is the authored size times that one scale, and the three placeholders are still extracted.
- A long word with `auto_scale: true` is shrunk to the content width. The same word with the flag absent keeps the authored size and may extend past the content box.
- A long run of lines with `auto_scale: true` is shrunk to the content height. The baseline step is the scaled `line_height + leading`.
- A span font size twice the box size, on a long line that does not fit, keeps that ratio after the scale.
- A short string that already fits is not enlarged. Padding shrinks the fit target to the content box, not the frame.

## Build order

1. Library API `render(document, base_dir) -> PDF bytes`, page size, and image rectangles only.
2. Plain text, left alignment, one embedded font.
3. Wrap, `left`/`center`/`right` and `top`/`middle`/`bottom` alignment, padding, leading, line-height, and the markup subset.
4. `auto_scale`: shrink every font size in a marked box so the text bounds fit the content rectangle.
5. CLI `render` subcommand.
6. HTTP handler for the webview, unchanged library.
