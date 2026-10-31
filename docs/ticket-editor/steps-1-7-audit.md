# Ticket editor steps 1–7 audit

Scope: steps 1–7 of [yaml-and-preview-plan.md](yaml-and-preview-plan.md). Steps 1–6 are the box frame in `src/struct_to_pdf/` (background, border, corner radius, opacity, scale and skew, letter-spacing). Step 7 is `template.yml` plus `meta.json` in `ticket-editor/`.

The issues below are logic and performance. The tests named in those steps stay green and do not catch them.

Severity:

- **High**: the PDF and the raster disagree, pixels are wrong, or a save destroys the badge.
- **Medium**: a narrower visual or geometry miss.

## Summary

| ID | Severity | Step | Issue |
|---|---|---|---|
| ✅ [TE-01](#te-01-framed-text-loses-its-antialiased-edge) | High | 1–5 | Framed text loses its antialiased edge |
| ✅ [TE-02](#te-02-pdf-opacity-fades-each-mark-the-raster-fades-the-box) | High | 4 | PDF opacity fades each mark; the raster fades the box |
| ✅ [TE-03](#te-03-save-and-migration-drop-the-badge-html) | High | 7 | Save and migration drop the badge HTML |
| ✅ [TE-04](#te-04-each-framed-box-allocates-a-full-page-buffer) | High | 1–5 | Each framed box allocates a full-page buffer |
| ✅ [TE-05](#te-05-the-border-covers-the-text-only-on-the-raster) | Medium | 2–5 | The border covers the text only on the raster |
| ✅ [TE-06](#te-06-a-rounded-stroke-loses-its-corner-band) | Medium | 3 | A rounded stroke loses its corner band |
| ✅ [TE-07](#te-07-positive-skew-shears-a-y-down-box-the-other-way) | Medium | 5 | Positive skew shears a y-down box the other way |

## High

### TE-01. Framed text loses its antialiased edge

✅ **Status: fixed.** `blend` is straight source-over, and both composite paths scale coverage by the stored alpha times the entry opacity. `framed_text_edge_matches_plain_text` checks a bordered box against plain text, and a scaled box for a gray fringe.

**Where:** `src/struct_to_pdf/raster.rs`, `draw_framed_entry` and `blend`.

**Flow:** Any box that is not plain (a background, a border, a radius, an opacity below 1, or a scale or skew) is painted onto a new layer, then copied onto the page. Glyphs are drawn with `blend`. A plain box is drawn straight onto the page, which is already an opaque fill.

**What is broken:** The layer from `RgbaImage::new` is transparent black. `blend` mixes the ink with the pixel already there and writes alpha 255. A half-covered edge is mixed with black and stored fully opaque. On the page that fringe stays a hard dark rim. The same function against the white page produces a soft edge, so the bug appears only after steps 1–5 send the box down the framed path.

**Fix:** Composite onto the layer with straight source-over: the ink color stays intact and coverage becomes alpha. `blend_layer` then uses that alpha, multiplied by the entry opacity.

### TE-02. PDF opacity fades each mark; the raster fades the box

✅ **Status: fixed.** A box whose fill or stroke sits under another mark is painted into an isolated transparency group, and `ca` / `CA` fade that group once. A box with only one mark still uses `ca` / `CA` on that mark. `box_opacity_fades_the_group` checks both.

**Where:** `src/struct_to_pdf/images.rs`, `frame_paint` and `place_entry`. The ExtGState stores `ca` and `CA`. The raster path is `composite_frame` / `blend_layer`.

**Flow:** `opacity: 0.4` on a filled text box wraps the fill, the stroke, and the glyphs. The raster paints the fill and the glyphs into one layer, then fades that layer once.

**What is broken:** `ca` and `CA` apply to each painting operation on its own. The orange fill composites at 0.4 against the page, then each glyph composites at 0.4 against that result, so the page shows through the letters. The raster treats the fill and the letters as one picture and fades it once. Step 4 asks for the same alpha on every pixel of the entry. That is the raster result. A box with both a fill and text (or a stroke that overlaps the text) will not match across the two outputs.

**Fix:** Paint the entry into a transparency group and set the group alpha. Leave `ca` / `CA` for a single mark that has nothing else in the box under it.

### TE-03. Save and migration drop the badge HTML

✅ **Status: fixed.** Posted `html` is stored in `meta.json` and returned by GET. A `page` body clears it. `migrateJson` copies the legacy HTML and leaves `template.json` in place. `PUT merges the page and ignores the optimistic lock` and `startup migrates template.json once and does not overwrite template.yml` cover both paths.

**Where:** `ticket-editor/app.mjs`, `applyWrite`, `writeTemplate`, `migrateJson`.

**Flow:** `/editor/:id` still saves HTML. `PUT` runs `applyWrite`, which skips `html` and `editor_state`, then `writeTemplate` rewrites `template.yml` with the new page size and the previous `contents`. The response body is `blankHtml`. On startup, or on the first request for an id that still has `template.json` and no `template.yml`, `migrateJson` builds a page from the page size, writes `template.yml`, and deletes `template.json`.

**What is broken:** The editor from [implementation-plan.md](implementation-plan.md) is already saving a badge into `template.json`. After step 7 that file is deleted and `contents` is empty, so the layout cannot be recovered. A later Save from the open screen sends the current HTML, the process discards it, and reload shows an empty sheet. Step 8 is what replaces `html` with the page object. Until that step, this PUT is the only save the screen has.

**Fix:** Keep the posted HTML in `meta.json` and return it from GET until step 9 accepts the page object. Leave `template.json` in place until that HTML has been turned into `contents`.

### TE-04. Each framed box allocates a full-page buffer

✅ **Status: fixed.** The layer is the pre-transform box plus half the border and one pixel. Fill, clip, stroke, and the copy walk only that buffer. Scale and skew still happen when the buffer is copied. `framed_layer_covers_the_box_not_the_page` checks the rectangle, and `framed_text_edge_matches_plain_text` checks the pixels.

**Where:** `src/struct_to_pdf/raster.rs`, `draw_framed_entry` and `blend_layer`.

**Flow:** For every entry whose frame is not plain, the raster allocates `RgbaImage::new(image.width(), image.height())`, paints the box into it, then `blend_layer` walks every pixel of that buffer.

**What is broken:** The buffer is the media box, including the empty area around a small label. An A4 page at 300 dpi is about 35 MB, zeroed, per background or border. A sheet of twenty labels does the allocation and the full scan twenty times. The pixels that matter are the transformed bounds of the entry plus half the border width.

**Fix:** Allocate the layer for that rectangle. Fill, clip, stroke, and composite only those pixels.

## Medium

### TE-05. The border covers the text only on the raster

✅ **Status: fixed.** The stroke is painted before the entry. A rounded clip clears only the entry buffer, then that buffer is copied onto the stroke. A scaled or skewed copy keeps samples out to the stroke's outer half. `border_sits_under_the_glyphs` and `scaled_border_keeps_its_outer_half` check both.

**Where:** `src/struct_to_pdf/images.rs`, `frame_paint`. `src/struct_to_pdf/raster.rs`, `draw_framed_entry` and `composite_frame`.

**Flow:** Step 2 strokes the frame after the fill and before the entry's own marks. The PDF path does that: fill, stroke, then the glyphs, inside a clip. The raster path fills, draws the entry, clips, then strokes.

**What is broken:** On the PDF a thick border's inner half sits under the glyphs. On the raster the stroke is painted after the glyphs, so it covers them. A scaled or skewed composite drops the outer half as well: `composite_frame` keeps samples inside the unscaled rounded rectangle, while the identity path copies the outer half of the stroke.

**Fix:** Stroke on the layer before the entry, and clip only the entry. When compositing a transform, include the stroke's outer half in the sample region.

### TE-06. A rounded stroke loses its corner band

✅ **Status: fixed.** The inner test keeps the outer radius as the corner zone and uses `border_radius - width / 2` only as the circle radius. `rounded_stroke_paints_its_corner_band` checks a 10 mm radius and a 2 mm stroke.

**Where:** `src/struct_to_pdf/raster.rs`, `stroke_frame` and `inside_round`.

**Flow:** A centered stroke is the set of pixels inside the outer rounded rectangle and outside the inner one. The inner radius is `border_radius - width / 2`. `inside_round` uses the radius it is given both as the circle radius and as the size of the corner zone.

**What is broken:** The inner test passes `border_radius - width / 2`, so the corner zone shrinks with it. A pixel in the real corner ring (inside the outer curve, outside the inner curve) is classified as interior. With a 10 mm radius and a 2 mm stroke, that ring is never painted.

**Fix:** Pass the outer radius as the corner zone and `border_radius - width / 2` as the circle radius.

### TE-07. Positive skew shears a y-down box the other way

✅ **Status: fixed.** `frame_matrix` builds the shear in document space and negates the terms that multiply the upward PDF y. The PDF `cm` and the raster copy share that matrix. `box_scale_and_skew_set_cm_about_the_top_left` expects `c = -1` for `skew_x: 45`, and `positive_skew_x_moves_the_bottom_right` checks the page direction.

**Where:** `src/struct_to_pdf/geometry.rs`, `frame_matrix`. The PDF `cm` is in `frame_paint`. The raster copy is `map_document` in `raster.rs`.

**Flow:** The page origin is the top-left and y grows downward. Scale and skew are applied about that top-left. `skew_x` is stored as `+tan(angle)` on the PDF `cm`, whose y axis grows upward.

**What is broken:** Scale about the top-left grows the box downward and matches the page. The shear follows the upward axis, so the bottom of the box moves left for a positive `skew_x`. A canvas control that sends the angle the user sees will preview the mirror image. The test locks the PDF-space numbers in, so the sign stays.

**Fix:** Build the shear in document space, then convert. Negate the coefficients that multiply the upward PDF y.
