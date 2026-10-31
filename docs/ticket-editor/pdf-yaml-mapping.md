# Badge editor and the PDF YAML page

The ticket editor stores a badge as HTML. The PDF tool stores a page as YAML (or the same tree in JSON): [document-schema.md](../pdf-tooling/document-schema.md), drawn by `struct_to_pdf`. A working badge in this repo is [scripts/badges/cnews-spb.yml](../../scripts/badges/cnews-spb.yml).

Both models are a sheet of rectangles. Each rectangle has a top-left, a width, a height, and something to draw inside. Positions are in millimetres. Y grows downward. That is enough to copy a simple badge across. The HTML editor can also store CSS the YAML page has no field for, and the YAML page has fields the HTML editor never writes.

## What already matches

A visual-mode badge is a list of absolutely positioned boxes. Reading the body children in order gives the YAML `contents` list. Paint order is that list: later entries draw above earlier ones.

| HTML | YAML |
|---|---|
| `page_width_mm`, `page_height_mm`, custom format | `page_size.width`, `page_size.height`, `units: mm` |
| `left` | `posX` |
| `top` | `posY` |
| `width` | `width` |
| `height` | `height` |
| DOM order | `contents` order |
| `{{ form_data.field_1 }}` inside one box | `template: true` and `preentered: "{{field_1}}"`, then a binding from `field_1` to a table column |
| `text-align: left \| center \| right` | `alignment.horizontal: left \| center \| right` |
| `font-weight`, italic, underline | `text.font_style.weight`, `italic`, `underline` |
| `color: #RRGGBB` on the block | `text.color` (the writer reads it; missing means black). A second color inside the same block is `<span style="color: #RRGGBB">` |
| EAN-13 image `{{ barcode_svg }}` | `barcode.type: ean13`, payload `preentered: "{{code}}"`, `template: true` |
| QR image `{{ qrcode_svg }}` | `barcode.type: qr` with the same payload rules |
| `{{ files.logo }}` with a rectangle | `image.resource_name` pointing at `resources.images` |
| `data-fit="min:Npt"` | `auto_scale: true` on the text box. Both shrink the font so the string stays in the rectangle. The algorithms differ; see below |
| Padding CSS | `padding.top/right/bottom/left` |
| One box with several `{{ }}` written in code mode | One `preentered` string, which is how `cnews-spb.yml` puts company, surname, and name in a single frame |

Unit conversion for type size: the editor stores `font-size` in points. The YAML `font_size`, `line_height`, and `leading` are in `page_size.units`. For a millimetre page, `mm = pt * 25.4 / 72`. A 12 pt label is `4.233` mm. `line-height: 1.2` with no unit is a multiplier of the font size. The YAML `line_height` is a length, and `leading` is an extra gap after the line. A unitless 1.2 at 12 pt is about `line_height: 5.08` mm and `leading: 0`.

A named office format (`A4`, `A5`, `A6`, `Letter`, `Legal`) becomes the same two numbers from `formats()`, swapped when `orientation` is `landscape`. The YAML page has no format name.

`editor_state` (the GrapesJS project) has no place in the YAML file. The rectangles and the strings are the page.

### One badge, both shapes

Editor HTML, 90×130 mm, one text block and no margins:

```html
<div class="tpl-text" data-text="1"
     style="position:absolute;left:13mm;top:49.5mm;width:64mm;height:37.5mm;
            font-size:24pt;line-height:1.2;text-align:center;color:#FF6A00">
  {{company}}<br>{{surname}}<br>{{name}}
</div>
```

The same rectangle in the PDF tool, which is the shape `cnews-spb.yml` already uses (fonts and the real markup omitted):

```yaml
page_size:
  width: 90
  height: 130
  units: mm
  bleeds: { top: 0, right: 0, bottom: 0, left: 0 }
resources:
  images: {}
  fonts: {}
contents:
  - id: badge
    posX: 13
    posY: 49.5
    width: 64
    height: 37.5
    template: true
    auto_scale: true
    alignment:
      horizontal: center
      vertical: middle
    padding: { top: 0, right: 0, bottom: 0, left: 0 }
    text:
      font: tektur
      font_size: 8.47
      leading: 0
      line_height: 5
      color: "#FF6A00"
      font_style: { weight: normal, italic: false, underline: false }
      preentered: "{{company}}<br>{{surname}}<br>{{name}}"
      content: null
```

`cnews-spb.yml` goes further than the visual editor usually does: one box, two font files, two colors, and `vertical: middle`. That file is still the same kind of rectangle. The visual editor would more often emit three `tpl-var` boxes, one field each. Both arrangements fit the YAML schema. One box with several mustaches is the closer match to a badge that scales as a single block.

## Where a rectangle is not enough

### Page margins and bleeds

The editor's four margins are the CSS `@page` margin. They shrink the printable area inside the sheet. The sheet size stays 100×100 if that is what was saved.

The YAML `bleeds` grow the media box outside the trim. A mark in the left bleed has a negative `posX`. Zero bleeds mean the trim is the whole page. Changing bleeds does not move existing boxes.

A 100×100 badge with 0 margins is `page_size` 100×100 and bleeds all 0. A badge with 2 mm margins is not a 2 mm bleed. The closest YAML reading is a content origin shifted in by 2 mm, or padding on each box. The schema has no page-margin field.

### Text fitting

`data-fit="min:8pt"` steps the used font down by 0.5 pt and stops at 8 pt, even if the text still overflows.

`auto_scale: true` multiplies every font size by `min(boxWidth / textWidth, boxHeight / textHeight)`, never above 1, and wraps again. There is no minimum size. A long company name can become unreadably small. The flag is also all or nothing: the editor's "do not shrink" (`data-fit` absent) is `auto_scale` absent.

### Variables

The editor's expression is a Django path plus filters, plus a prefix and a suffix stored beside the expression. Four pipes are part of the editor and of [implementation-plan.md](implementation-plan.md#step-7-field-pipes-and-ifst). The binding name is the text before the first `|`. The pipe stays on the mustache:

| Pipe | Mustache | Effect |
|---|---|---|
| `\|upper` | `{{name\|upper}}` | Whole value in uppercase |
| `\|title` | `{{name\|title}}` | Each word starts with an uppercase letter |
| `\|join:", "` | `{{tags\|join:", "}}` | A list is joined with that separator. A string is left as itself |
| `\|default:"Гость"` | `{{name\|default:"Гость"}}` | That string when the value is missing, empty, an empty list, boolean `false`, or integer `0`. A present value stays |

Pipes apply left to right in the order join, then default, then case. `{{tags|join:", "|upper}}` joins and then uppercases. An empty value with `{{name|default:"Гость"|upper}}` is `ГОСТЬ`. `format` on a binding stays strftime for dates only.

`ifSt` on the entry is the condition. The editor stores the same key as `data-if`. An absent or empty `ifSt` draws the box. Otherwise the string is a row key, not a mustache. The condition is true, and the box is drawn, when that value is non-empty, and when the value is boolean `true`. The condition is false, and the box is not drawn, when the value is integer `0` or boolean `false`.

| Row value for the `ifSt` key | Draw |
|---|---|
| missing | no |
| `""` | no |
| boolean `false` | no |
| integer `0` | no |
| boolean `true` | yes |
| any other non-empty value | yes |

That is the whole test. It is not a Django `{% if %}`. Code mode can still write `{% if %}`, and that HTML then refuses to open on the canvas.

A direct copy works for `{{surname}}` and `{{name}}`. These editor features still have no YAML field:

| Editor | Gap |
|---|---|
| `form_data.field_12` path | The YAML name is a flat key. The binding can hide the path, if the export strips `form_data.` |
| `data-prefix`, `data-suffix` | Can be written as literal characters around the mustache in `preentered`. The editor keeps them as attributes so the trait panel can edit them |

### Fonts

The editor names a CSS family (`font-family: Tektur`) and may upload a `.ttf` as an asset. The template must contain `@font-face` for that file. Visual mode will not open HTML that contains `<link>`.

The YAML page names a key in `resources.fonts` (`font: tektur`). The file is `source_path`, with `font_family`, `weight`, and `italic`. A span switches face with `font="tektur-medium"`. There is no CSS family string on the box.

An export has to assign those keys and copy the font bytes next to the YAML. A box that only says `font-family: sans-serif` has no file to embed.

### Codes

The editor draws a code as an image whose URL is filled at print time (`barcode_svg`, `qrcode_svg`, `vcard_qrcode_svg`). The width is locked to a whole number of dots per module at 203 dpi so a thermal printer does not blur the modules.

The YAML `barcode` entry draws the symbol into the rectangle from a string. Types are `ean13`, `code128`, and `qr`. The rectangle is scaled to `width` and `height`. There is no dots-per-module field.

| Editor code | YAML |
|---|---|
| EAN-13 of the ticket code | `barcode.type: ean13`, `preentered: "{{code}}"`, `template: true` |
| QR of the ticket code | `barcode.type: qr`, same payload |
| vCard QR | A `qr` whose payload is the vCard text. The editor builds that text from `vcard_template` on the ticket type. The YAML page has no vCard builder, so the row would have to contain the finished vCard string |
| PNG versus SVG variant | One symbol. The writer draws it. There is no raster/vector switch |
| `data-dots` | No field. Module size is whatever fits the rectangle |
| Caption under the bars | A separate text box in both models |

`code128` exists on the YAML side and is absent from the editor palette.

### Images and photos

`{{ files.logo }}` is a file shipped with the template. It maps to `resources.images` plus an `image` placement. The rectangle is the placement. The writer ignores `transformation_matrix`.

A guest portrait is the opposite. The YAML `photo` entry reads `photo.field` from the row (a path, or bytes supplied by the registration print) and draws jpeg, bmp, or png into the rectangle. The editor has no per-row image. Every picture is a template asset.

### Decoration the YAML page stores on the frame

These CSS properties now have a page field. The canvas writes them on the page object in [step 10](yaml-and-preview-plan.md#step-10-edit-the-page-on-the-canvas). [Step 8](yaml-and-preview-plan.md#step-8-send-the-page-as-json) is the GET that delivers that object.

| CSS on a block | YAML field |
|---|---|
| `background-color` | `background`, `#RRGGBB`. The painter fills the frame before the entry's marks |
| `border`, `border-radius` | `border` (`width`, `color`) and `border_radius` |
| `opacity` | `opacity`, from 0 to 1 |
| `transform` scale and skew | `scale_x`, `scale_y`, `skew_x`, `skew_y`, about the top-left. A text box may also set `rotation` to `0` or `90`. Images, barcodes, and photos have no rotation |
| `letter-spacing` | `text.letter_spacing`, a length in page units |

### Decoration the YAML page does not store

The style manager can still write these as CSS. The page has no field for them.

| CSS on a block | In the YAML schema |
|---|---|
| `white-space: nowrap` | The writer wraps to the content width. A single-line badge is a tall enough `line_height` or a box that happens to fit one line |
| `text-overflow: ellipsis` | No clipping with an ellipsis. Text outside the frame is clipped |
| `overflow` | The frame clips. There is no visible-overflow choice |
| `width` in `%` | Width is a length in page units |
| `z-index` other than DOM order | Paint order is the array order only |

A line in the editor is a thin box. The page stores that as a frame with `background` or `border`.

### Fields the YAML page has and the editor does not write

These are already specified in [document-schema.md](../pdf-tooling/document-schema.md). A badge exported from the visual editor leaves them at defaults.

| YAML field | Editor today |
|---|---|
| `id` on every entry | No stable id. DOM order is the identity. An export must assign `p1-t1`, `p1-i1`, and keep them if the file is reopened |
| `schema_version: 1` | No version |
| `page_size.bleeds` | Margins, which are a different inset. Export of a 0-margin badge should write bleeds of 0 |
| `alignment.vertical` (`top`, `middle`, `bottom`) | No trait. `cnews-spb.yml` uses `middle`. The editor would need a property before it can round-trip that |
| `leading` separate from `line_height` | One CSS `line-height` |
| `content` versus `preentered` | One HTML string. Export should set `preentered` and `content: null` |
| `delimiters` | Always `{{` `}}` |
| `ifSt` | Planned as `data-if` ([implementation plan step 7](implementation-plan.md#step-7-field-pipes-and-ifst)). The draw rule is the table in the variable section |
| `barcode.type: code128` | Palette is EAN-13, QR, vCard QR |
| `photo.field` | No per-row image |
| `resources.fonts[].source_path` | CSS family plus an optional uploaded font file |
| Inline `<span font="…">`, `<b>`, `<leading>` | One style for the whole `tpl-var`. Code mode can write tags the writer would reject (`<div>`, `class`, `style` on the div). The writer's subset is `br`, `b`/`bold`, `i`, `u`, `span`, `leading` |

### Fit text, side by side

| | Editor `data-fit` | YAML `auto_scale` |
|---|---|---|
| When | Attribute present | `true` |
| How | Subtract 0.5 pt until it fits | One scale from the overflow ratio, then wrap again |
| Floor | `min:6pt`, `min:8pt`, or `min:10pt` | None |
| What scales | The element's `font-size` | Every run's font size |
| Written back | The script sets `style.fontSize` on the live DOM. The saved HTML keeps the authored size and `data-fit` | The writer does not write the scaled sizes back into the file |

To carry the editor's floor, the YAML box would need a minimum size next to `auto_scale`, for example `min_font_size` in page units. That field is not in the schema.

## What the YAML design still lacks

For a badge the editor can draw, and for the badges already written as YAML (`cnews-spb.yml`), the page is a rectangle list and should stay one. The gaps are fields on those rectangles, not a second document type.

Needed to represent an editor template without loss:

1. **Page margins** as their own four lengths, or an explicit rule that bleeds are unused and margins are baked into `posX`/`posY`. Right now the two words describe different edges.
2. **Minimum font size** beside `auto_scale`, in page units, so shrink-to-fit can stop at 8 pt.
3. **Font embedding from an uploaded file** is already `resources.fonts`. The missing piece is on the editor side: a family name has to become a resource key. No schema change.
4. **Dots per module** on `barcode`, if thermal output must match the editor's snapped widths. The writer can keep scaling into the rectangle for office PDFs and use the density only for a raster target.
5. **`white-space: nowrap`** as a wrap mode on the text box. Ellipsis is the next step, and it matters less than nowrap for a one-line surname.

Already in the schema and worth keeping: `background`, `border`, `border_radius`, `opacity`, `scale_x`, `scale_y`, `skew_x`, `skew_y`, `text.letter_spacing`, `ifSt` with the draw rule above (non-empty or boolean `true` draws; integer `0` or boolean `false` skips), `alignment.vertical`, `photo`, `code128`, `bleeds`, span-level font and color. `cnews-spb.yml` depends on vertical centering, span fonts, and span colors. The visual editor does not write those yet. `ifSt` is [step 7 of the implementation plan](implementation-plan.md#step-7-field-pipes-and-ifst).

Not worth adding for a badge: GrapesJS `editor_state`, Django `{% for %}`, arbitrary HTML, percent widths, and a PNG-versus-SVG switch on codes.
