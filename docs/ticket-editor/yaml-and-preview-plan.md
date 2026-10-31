# Save the badge as PDF-tool YAML and download a preview

PDF here is the file format (ISO 32000), not this repository's writer. A length in a PDF is a number of points (1/72 inch). The page YAML is what `struct_to_pdf` loads and draws.

| Feature | PDF standard | YAML parameter |
|---|---|---|
| Position | Yes. User space | `posX`, `posY` |
| Width and height in mm or points | Yes, once stored as points. The file has no millimetre unit | `width`, `height` |
| Width or height in `%` | No | No |
| Page size | Yes. `MediaBox` | `page_size.width`, `page_size.height`, `page_size.units` |
| Bleeds | Yes. `BleedBox` and `TrimBox` | `page_size.bleeds` |
| Whole-sheet fill | Yes. A filled path | `page_size.background` |
| Box background | Yes. A filled path | `background` |
| Border | Yes. A stroked path | `border.width`, `border.color` |
| Corner radius | Yes, as a curve. No radius field | `border_radius` |
| Opacity | Yes. ExtGState fill and stroke alpha (PDF 1.4) | `opacity` |
| Text color | Yes | `text.color` |
| Font and size | Yes. `Tf` | `text.font`, `text.font_size` |
| Bold, italic | No weight flag. A bold or italic face is a separate font | `text.font_style` |
| Underline | No text attribute | `text.font_style.underline` |
| Horizontal alignment | No | `alignment.horizontal` |
| Vertical alignment | No | `alignment.vertical` |
| Line height | Yes. `TL` is the distance between baselines | `text.line_height` |
| Extra gap after a line | Yes. The same leading | `text.leading` |
| Padding | No | `padding` |
| Text rotation, any angle | Yes. The text matrix | `rotation`, only `0` or `90` |
| Scale or skew | Yes. The text matrix and `cm` | `scale_x`, `scale_y`, `skew_x`, `skew_y` |
| Letter-spacing | Yes. `Tc` | `text.letter_spacing` |
| Nowrap, ellipsis | No. The page does not wrap or truncate a string | No |
| Shrink-to-fit | No | `auto_scale` |
| Show or hide a box from data | No. Optional content is a viewer layer, not a field test | `ifSt` |
| Mustache text | No | `template`, `preentered`, `content`, `delimiters` |
| A second font or color inside one box | Yes. Font and color can change between text shows | `<span>` inside `preentered` |
| Placed image | Yes. An image XObject | `image.resource_name` |
| EAN-13, Code 128, QR | No barcode object | `barcode.type` |
| Guest photo | The picture is an image. A data field is not PDF | `photo.field` |
| Stacking other than paint order | No `z-index` | `contents` order |

Box background, border, corner radius, opacity, scale and skew, and letter-spacing are page fields. [Steps 1–6](#step-1-box-background) added them. `rotation` stays `0` or `90`, because that parameter already existed.

This plan is the gate before [Run](implementation-plan.md#step-6-run) in [implementation-plan.md](implementation-plan.md). It starts when [steps 1–5](implementation-plan.md#step-5-plug-toolbar-actions-into-the-route) of that plan are done. Pass this plan through [Run](#step-18-run) before that click-through. [Field pipes and ifSt](implementation-plan.md#step-7-field-pipes-and-ifst) stays after that Run.

The editor at `/editor/:id` already loads and saves through the Node process in `ticket-editor/`. That process stores the page as `template.yml` and the editor fields in `meta.yml`. A `meta.json` left from before is read when `meta.yml` is absent. A save writes `meta.yml`. [Step 8](#step-8-send-the-page-as-json) sends that page to the browser as JSON. [Step 9](#step-9-save-the-page-json-as-yaml) writes the JSON the browser sends back to `template.yml`.

The document the screen edits is that page object. JSON on the wire and YAML on disk are the same object. GrapesJS is the sheet that draws the blocks: text, barcodes, and images are dragged and resized there. The page format does not require it. The HTML GrapesJS holds is the picture of the current JSON. A drag, a resize, or a trait edit writes the matching field on the entry immediately. Field notes for that picture are [pdf-yaml-mapping.md](pdf-yaml-mapping.md).

A later CRDT would merge those entries. Each entry is already one map with its own id, and `contents` is their order. This cut does not sync two browsers. It keeps the object in a shape that sync can take up without a second document.

Preview goes to the pdf-tooling service. The form backend (the `ticket-editor/` process) runs in Docker. Compose starts it beside pdf-tooling and sends preview there. Превью PDF downloads the PDF.

The screen moves into `ticket-editor/frontend`, beside this process, before preview. It serves three routes and no registration pages. Zebra preview, test print, and ZIP export stay on `/operator/templates/:id` in the PIR app.

## Steps

Each step has a Check. The check uses only what that step added.

| Step | What it does | Status |
|---|---|---|
| [1. Box background](#step-1-box-background) | A filled rectangle behind one entry. | ✅ Completed |
| [2. Border](#step-2-border) | A stroked frame on one entry. | ✅ Completed |
| [3. Corner radius](#step-3-corner-radius) | The fill and the stroke follow one rounded path. | ✅ Completed |
| [4. Opacity](#step-4-opacity) | One alpha for the whole entry. | ✅ Completed |
| [5. Scale and skew](#step-5-scale-and-skew) | `cm` about the box top-left. | ✅ Completed |
| [6. Letter-spacing](#step-6-letter-spacing) | `Tc` on a text box. | ✅ Completed |
| [7. Store the page as YAML](#step-7-store-the-page-as-yaml) | `template.yml` plus `meta.json`. `npm test` reads and writes that page. | ✅ Completed |
| [8. Send the page as JSON](#step-8-send-the-page-as-json) | GET returns the page object. The browser builds the GrapesJS picture from it. | ✅ Completed |
| [9. Save the page JSON as YAML](#step-9-save-the-page-json-as-yaml) | PUT sends that object. The process writes `template.yml`. | ✅ Completed |
| [10. Edit the page on the canvas](#step-10-edit-the-page-on-the-canvas) | A drag or a trait writes the page object, then the canvas shows it. | ✅ Completed |
| [11. Form backend and pdf-tooling in Compose](#step-11-form-backend-and-pdf-tooling-in-compose) | Both containers. The editor process is what the screen calls. | ✅ Completed |
| [12. List the templates](#step-12-list-the-templates) | `GET /api/print_template/list` reads the template directories. | ✅ Completed |
| [13. Editor app and its route table](#step-13-editor-app-and-its-route-table) | `ticket-editor/frontend` serves only the three editor routes. | ✅ Completed |
| [14. Canvas in the editor app](#step-14-canvas-in-the-editor-app) | `/editor/:id` is the page editor, served from that app. | ✅ Completed |
| [15. Template list](#step-15-template-list) | `/editor/list` lists the templates and opens one. | ✅ Completed |
| [16. Preview endpoint](#step-16-preview-endpoint) | `POST …/preview` returns a PDF from pdf-tooling. | ✅ Completed |
| [17. Preview button opens the PDF](#step-17-preview-button-opens-the-pdf) | Превью PDF on `/editor/:id` opens the PDF in a tab. | Not started |
| [18. Run](#step-18-run) | Compose, the editor app, then edit, save, and download. | Not started |

## Checking a step

| What the step added | How to check it, before Run |
|---|---|
| A new page field | `cargo test --test struct_to_pdf` from the repository root. No Node process and no browser. |
| The page file, read as JSON and written back from JSON | `npm test` in `ticket-editor/`. No server and no browser. |
| The GrapesJS picture and live edits of the page object | The unit test for `page-view.ts` covers the picture. After [step 14](#step-14-canvas-in-the-editor-app) that test lives in `ticket-editor/frontend`. The click-through is [step 10](#step-10-edit-the-page-on-the-canvas). |
| The editor app and its three routes | `npm test` in `ticket-editor/frontend`. The registration app is not started. |
| Compose and the PDF bytes | `docker compose up --build` in `ticket-editor/`, then curl. The Angular app is required only for the button. |

## The page the editor stores

`load_page` accepts one page. A file with a `pages` or `metadata` key is rejected. Editor fields that are not part of that page live in `meta.yml`. A directory that still has only `meta.json` is read from that file. The next save writes `meta.yml` and removes `meta.json`.

```text
ticket-editor/data/templates/<id>/
  template.yml
  meta.yml
  files/<name>.<ext>
  fonts/LiberationSerif-Regular.ttf
```

`template.yml` is the page. `meta.yml` holds `id`, `name`, `source`, `is_default`, `updated_at`, the four margins, and `manifest`. GET returns that object on `page` and `html` null. `editor_state` stays null. The process does not store GrapesJS project data or an HTML document. It does not store `ticket_type`, `ticket_type_name`, `page_format`, or `orientation`.

The sheet size is `page_size.width` and `page_size.height` in `template.yml`. `units` is `mm`.

Margins stay in `meta.yml`. They are the CSS `@page` inset from the first plan. The page has no margin field. Preview draws `template.yml` as stored. Bleeds stay on `page_size.bleeds`. A page this editor creates writes every bleed side as `0`. A file that already has bleeds keeps those four numbers across save. The toolbar does not edit them.

The blank page written when `template.yml` is missing:

```yaml
page_size:
  width: 100
  height: 100
  units: mm
  bleeds: { top: 0, right: 0, bottom: 0, left: 0 }
resources:
  images: {}
  fonts:
    text:
      name: text
      font_family: Liberation Serif
      weight: normal
      italic: false
      embedded: true
      source_path: fonts/LiberationSerif-Regular.ttf
contents: []
```

The font file is the Cyrillic face the PDF tests already load, `tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf`. Seeding a template copies it to `fonts/LiberationSerif-Regular.ttf` under that template. Every text box this editor creates uses the resource key `text`.

If `template.yml` is missing and `template.json` is present, build the page from that JSON once, write `template.yml` and `meta.json`, and delete `template.json`. Later reads use the YAML.

Lengths on a text box are millimetres, the same unit as `page_size.units`. The canvas stores `font-size` in points. `mm = pt * 25.4 / 72`. An 18 pt label is `6.35` mm. A unitless CSS `line-height: 1.2` at that size is `line_height: 7.62` and `leading: 0`.

`content` on a text box or a barcode this editor writes is `null`. The writer then paints `preentered`.

## How the canvas shows a page entry

The browser holds the page object. `pageToHtml(page)` in `frontend/src/app/operator/templates/page-view.ts` builds the HTML GrapesJS draws: a body of absolutely positioned blocks, body size equal to `page_size`. Visual mode opens that string. Code mode is a text view of the same object, pretty-printed JSON, highlighted by CodeMirror. A fold marker on an object or array collapses that field. Applying code mode parses that JSON, replaces the object, and rebuilds the canvas. YAML stays on the server. The `yaml` package is what the Node process uses to read and write `template.yml`.

`contents` is the paint order. The matching block carries `data-yaml-id` equal to the entry `id`. The browser assigns an id when it appends an entry: `p1-t<n>` for text, `p1-i<n>` for an image, `p1-b<n>` for a barcode, using the next free number among ids already on the page. A move writes `posX`, `posY`, `width`, and `height` on that entry and leaves `preentered` as it was, spans included. A text edit writes `preentered`. The canvas is then rebuilt from the object, so GrapesJS does not become a second copy of the badge.

| Canvas | Page entry |
|---|---|
| `left`, `top`, `width`, `height` in mm | `posX`, `posY`, `width`, `height` |
| `tpl-text` | A text entry. `preentered` is the literal string. `template` is omitted |
| `tpl-var` | A text entry with `template: true`. The mustache is the flat name below |
| `tpl-code` `barcode` | `barcode.type: ean13`, `template: true`, `preentered: "{{code}}"` |
| `tpl-code` `qrcode` | `barcode.type: qr`, same payload |
| `tpl-file` | An image entry. `image.resource_name` is the manifest key |
| `data-if` | `ifSt` on the entry. An empty value omits the key |
| `text-align` | `alignment.horizontal` |
| `data-valign` | `alignment.vertical` (`top`, `middle`, `bottom`). A new box writes `top` |
| CSS `padding` in mm | `padding.top/right/bottom/left`. A missing side is `0` |
| `font-weight`, italic, underline | `text.font_style` |
| `color` | `text.color` as `#RRGGBB`. Black omits `color` |
| `data-font` | `text.font`, a key in `resources.fonts`. A new box writes `text` |
| `data-auto-scale="true"` | `auto_scale: true`. Absent writes the key omitted |
| `data-leading` | `text.leading`. Absent writes `0` |
| `data-rotation` | `rotation` on a text entry. `0` or `90`. Absent writes `0`. `90` is clockwise and the line reads upward. Any other angle is 400 |
| `background-color` | `background`. Absent writes the key omitted |
| `border` width and color | `border.width`, `border.color`. Absent writes `border` omitted |
| `border-radius` | `border_radius`. Absent or `0` writes `0` |
| `opacity` | `opacity`. Absent writes the key omitted, which the writer treats as `1` |
| `letter-spacing` | `text.letter_spacing`. Absent writes `0` |
| scale and skew | `scale_x`, `scale_y`, `skew_x`, `skew_y`. Absent writes the identity (`1`, `1`, `0`, `0`) |

`template: true` sits on the entry, beside `text` or `barcode`, which is where `load_page` reads it. `preentered` for a barcode sits inside `barcode`. A symbol whose flag is nested under `barcode` is drawn as the characters `{{code}}`.

An image resource:

```yaml
logo:
  name: logo
  extracted_path: files/logo.png
```

An uploaded `.ttf` or `.otf` is also a font resource. The key is the asset name. `source_path` is `fonts/<name><ext>`, and `manifest[name]` is that same path so `GET …/assets/:name` still returns the bytes. `font_family` is that name, so the canvas can list it. The block that uses it sets `data-font` to the key.

A guest portrait (`photo.field`) and `barcode.type: code128` have no palette item. They stay in `contents`. The view draws a placeholder block with that `data-yaml-id`. The palette does not insert either. Save sends the entry unchanged when the user has not edited it.

### Flat mustache names

The PDF tool's field name is the whole string between `{{` and `}}`. Pipes stay on that string. The name before the first `|` is the flat key.

| Editor path | Hole before pipes | Preview value |
|---|---|---|
| `form_data.field_1` | `field_1` | `Петрова` |
| `form_data.field_5` | `field_5` | the list `Доклад`, `Стенд` |
| `ticket.number` | `number` | `A-00042` |
| `ticket.ticket_code.code` | `code` | `2000000000008` |
| `ticket.ticket_type.name` | `ticket_type_name` | `Standard` |

`data-prefix` and `data-suffix` are literal characters around the mustache in `preentered`. `{{ form_data.field_1|upper }}` is stored as `{{field_1|upper}}`.

`ifSt` is the flat key (`field_6`), with no braces and no pipes. The draw rule is the one in the first plan: a missing value, `""`, boolean `false`, or integer `0` skips the box; boolean `true` and any other non-empty value draw it.

### Fields the picture can edit

Steps 1–6 give the page a field for box background, border, corner radius, opacity, scale, skew, and letter-spacing. [Step 10](#step-10-edit-the-page-on-the-canvas) keeps those controls. Each one writes the page object. The page cannot store `white-space`, ellipsis, overflow, a percent width, or a `z-index` other than `contents` order, because the PDF standard has no such feature. Step 10 takes those controls off the editor route.

A page object the writer would reject is 400 `{ "detail": "..." }` and leaves `template.yml` unchanged. That includes a bad color, a radius larger than half the shorter side, and an entry that is not text, image, barcode, or photo.

`preentered` on the object is the string the writer paints, including `<span>` markup from a file such as `scripts/badges/cnews-spb.yml`. The view copies it onto `data-preentered` so a drag does not ask GrapesJS to reinterpret the tags. A move does not change that string. A text edit replaces `preentered` on the entry.

## Step 1. Box background

[Steps](#steps)

✅ Completed.

A content entry may set `background: "#RRGGBB"`. Absent means the box is not filled. The same parser as `text.color` accepts the string. Text, image, barcode, and photo entries all take it.

The PDF painter fills that rectangle before the entry's own marks. The raster does the same. `page_size.background` is unchanged: it still fills the sheet, underneath every box.

`load_page` rejects any other shape. [document-schema.md](../pdf-tooling/document-schema.md) gains the field.

### Check

```bash
cargo test --test struct_to_pdf
```

A text box with `background: "#FF6A00"` paints a filled rectangle of its frame before the glyphs. An entry with no `background` paints no rectangle. A bad string fails to load.

## Step 2. Border

[Steps](#steps)

✅ Completed.

A content entry may set `border`. Both keys are required when the object is present.

```yaml
border:
  width: 0.4
  color: "#112233"
```

`width` is a finite length in `page_size.units`, greater than 0. `color` is `#RRGGBB`. The path is the frame. The stroke is centered on that path, which is the PDF default. The painter strokes it after the fill from [step 1](#step-1-box-background) and before the entry's own marks. The raster strokes the same rectangle.

### Check

`cargo test --test struct_to_pdf`. A box with that border emits a stroke of the frame in that color. `width: 0` and a missing `color` fail to load. An entry with no `border` is unchanged.

## Step 3. Corner radius

[Steps](#steps)

✅ Completed.

A content entry may set `border_radius`, a length in `page_size.units`, `>= 0`. Absent or `0` keeps the sharp rectangle from steps 1 and 2. A radius larger than half the shorter side fails to load, so the curves cannot cross.

The fill and the stroke use one rounded path, built with Bézier curves. Text, images, barcodes, and photos are clipped to that path. The raster uses the same outline.

### Check

`cargo test --test struct_to_pdf`. A box with `border_radius: 2` and both a background and a border paints one rounded path for the fill and the stroke. `border_radius: 0` stays a rectangle. A radius of half the short side plus a millimetre fails to load.

## Step 4. Opacity

[Steps](#steps)

✅ Completed.

A content entry may set `opacity`, a number from 0 to 1 inclusive. Absent means 1. The painter saves the graphics state, sets an ExtGState whose fill alpha and stroke alpha are that number, paints the fill, the stroke, and the entry, then restores the state. The raster uses the same alpha on every pixel of that entry.

### Check

`cargo test --test struct_to_pdf`. `opacity: 0.4` wraps that entry's operators in a graphics state whose alpha is 0.4. `opacity: 1` adds no state. `1.5` and `-0.1` fail to load.

## Step 5. Scale and skew

[Steps](#steps)

✅ Completed.

Four optional numbers on a content entry. Each absent value is the identity.

| Field | Absent | Meaning |
|---|---|---|
| `scale_x`, `scale_y` | `1` | Finite, and not `0` |
| `skew_x`, `skew_y` | `0` | Degrees. The off-diagonal of the matrix is the tangent of that angle |

`width` and `height` stay the size before this transform. The anchor is the top-left, `posX` and `posY`. The painter sets `cm` for the entry, then paints the fill, the stroke, and the contents. The raster applies the same matrix. Text rotation `0` or `90` is applied inside that matrix. It is not replaced by a free angle.

### Check

`cargo test --test struct_to_pdf`. `scale_x: 2` doubles the drawn width about the top-left and leaves `width` in the file at its authored value. `skew_x: 45` shears by that angle. `scale_x: 0` fails to load. An entry that sets none of the four is painted as before.

## Step 6. Letter-spacing

[Steps](#steps)

✅ Completed.

A text box may set `text.letter_spacing`, a finite length in `page_size.units`. Absent means `0`. Negative is allowed. The painter converts it to points and sets `Tc` for that box. The raster adds the same extra advance between glyphs. A span does not override it.

Image, barcode, and photo entries reject the key.

### Check

`cargo test --test struct_to_pdf`. A text box with `letter_spacing: 0.5` sets `Tc` to that length in points. Absent stays `0`. A barcode entry that carries `letter_spacing` fails to load.

## Step 7. Store the page as YAML

[Steps](#steps)

✅ Completed.

Replace the page half of `template.json` in `ticket-editor/app.mjs`. `templateFile` points at `template.yml`. `readTemplate` loads the YAML and `meta.json` and returns the detail object the editor already accepts. `writeTemplate` writes both files. `blankTemplate` is the page in the previous section plus the same meta the blank JSON used (`name` `Badge`, `source` `local`, ticket type `Standard`).

`PUT` still accepts `PrintTemplateWrite` and still checks side lengths and margins with `assertPage`. The page size it stores is `page_size.width` and `page_size.height`. Margins go to `meta.json`.

`npm test` in `app.test.mjs` asserts the YAML file. The first read is a 100×100 page with empty `contents` and the `text` font. The 90×130 write is stored on `page_size`. A 5 mm side is 400 and leaves 90×130. Asset upload, read, and delete stay on `files/`. Upload still updates `manifest` in `meta.json` and in the JSON response. `resources.images` inside `template.yml` is filled in [step 9](#step-9-save-the-page-json-as-yaml).

### Check

From `ticket-editor/`:

```bash
npm test
```

The blank-template assertions pass against `template.yml`. `data/templates/1/template.json` is not required. The font file exists at `data/templates/1/fonts/LiberationSerif-Regular.ttf`.

## Step 8. Send the page as JSON

[Steps](#steps)

✅ Completed.

`GET /api/print_template/:id` reads `template.yml` and returns `PrintTemplateDetail` with `page` set to that object (`page_size`, `resources`, `contents`) and `html` null. `editor_state` stays null. The Angular screen does not parse YAML.

`pageToHtml(page)` in `page-view.ts` builds the GrapesJS document. The editor opens visual mode from that string. The object it keeps is `page`.

One text entry becomes one `div`. A `preentered` that is a single mustache, with optional literal prefix and suffix, becomes `tpl-var`. The traits (`data-var`, `data-case`, `data-join`, `data-default`, `data-prefix`, `data-suffix`) are filled so the trait panel can edit them. Any other string becomes `tpl-text`, and the text node is the `preentered` string. Both carry `data-yaml-id`, `data-preentered`, `data-font`, and `data-valign`.

A barcode entry becomes `img.tpl-code`. `ean13` sets `data-code="barcode"`. `qr` sets `data-code="qrcode"`. The image `src` on the canvas is the demo data URI the editor already uses for that code. The stored entry stays the barcode object.

An image entry becomes `img.tpl-file` with `data-file` equal to the resource key. The canvas still loads the bytes through `GET …/assets/:name`.

`font-size` on the element is the point size, `pt = mm * 72 / 25.4`. `line-height` is the unitless ratio `line_height / font_size` when `leading` is `0`. A non-zero `leading` is `data-leading`. `font-family` is `resources.fonts[text.font].font_family`.

`ifSt` becomes `data-if`. The canvas show/hide from the first plan's step 7 applies to that attribute. `rotation: 90` becomes `data-rotation="90"`. `0` omits the attribute. `background`, `border`, `border_radius`, `opacity`, and `text.letter_spacing` become `background-color`, `border`, `border-radius`, `opacity`, and `letter-spacing`. `scale_x`, `scale_y`, `skew_x`, and `skew_y` become `data-scale-x`, `data-scale-y`, `data-skew-x`, and `data-skew-y`. An identity value is omitted.

### Check

`npm test` in `ticket-editor/`. The blank GET has `page` equal to the 100×100 object, empty `contents`, the `text` font, and `html` null. `template.yml` is unchanged.

The frontend spec for `page-view.ts` loads a page with three entries: a text box `{{field_1|upper}}` at `10,20` size `80×12`, a QR, and an image `logo`. The HTML has `tpl-var` with `data-var="form_data.field_1"` and `data-case="upper"`, `data-code="qrcode"`, and `data-file="logo"`. Positions are millimetres. `data-yaml-id` matches the three ids.

## Step 9. Save the page JSON as YAML

[Steps](#steps)

✅ Completed.

`PUT /api/print_template/:id` accepts the meta fields and `page`. `page` is the object the browser has been editing. The process checks it with the same rules `load_page` uses, writes `template.yml`, and leaves `meta.yml` for the name, margins, and manifest. A page the writer would reject is 400 and does not write. `editor_state` in the body is ignored.

`resources.images` is rebuilt from the manifest. `resources.fonts` keeps `text` and adds one entry per uploaded font. Ids in `contents` are the ids the browser already assigned. The reply's `page` is the object just written, including those resources. `html` in the reply is null. The client keeps that reply as its page object.

A move that only changed `posX` leaves `preentered` byte for byte, spans included, because the browser never rewrote that field.

### Check

`npm test` covers three writes:

1. PUT the three-entry page from [step 8](#step-8-send-the-page-as-json). `template.yml` has those ids, the mustache `{{field_1|upper}}`, the QR entry, and the `logo` image path.
2. PUT a page whose `contents` gained one text entry, id `p1-t2`, `preentered: "{{field_2}}"`, `template: true`. The file has that entry and the previous ones.
3. PUT a page with `background: "orange"` returns 400 and leaves the file unchanged.

Then the HTTP sequence from the first plan's template test: PUT a page with one variable, and `template.yml` on disk contains `posX`, `template: true`, and `{{field_1}}`.

## Step 10. Edit the page on the canvas

[Steps](#steps)

✅ Completed.

The screen loads `page` from [step 8](#step-8-send-the-page-as-json) and draws it with `pageToHtml`. Traits and the style manager write that object. The canvas shows the result. The fields are the ones in [pdf-yaml-mapping.md](pdf-yaml-mapping.md): what the page can store, and none of the CSS it cannot.

The changes are gated by `yamlPage: true` in `src/environments/environment.editor.ts`. `/operator/templates/:id` keeps the Django style manager, `data-fit`, and `{{ form_data.field_1 }}`.

Files: `page-view.ts`, `visual-editor.component.ts`, `template-editor.component.html`, `template-editor.component.ts`, `code-editor.component.ts`, `template-snippets.ts`, `print.models.ts`.

### Traits the page already has

These are the rows in the mapping under "Fields the YAML page has and the editor does not write", limited to what a badge box needs. Each one becomes a trait on `/editor/:id`. Changing it writes the page field and refreshes the block from that entry.

| Trait | Attribute | Page field | Canvas |
|---|---|---|---|
| По вертикали `top` / `middle` / `bottom` | `data-valign` | `alignment.vertical` | The text block sits on that side of the box. A new box is `top`. `cnews-spb.yml` opens as `middle` |
| Уместить текст | `data-auto-scale="true"` | `auto_scale` | Authored `font-size` stays on the box. The writer shrinks at preview. The trait replaces `data-fit` |
| Интервал, mm | `data-leading` | `text.leading` | Extra gap after the line. Default `0`. The style manager's `line-height` stays the unitless ratio. The editor writes that ratio into `text.line_height` in millimetres |
| Показывать если | `data-if` | `ifSt` | The flat key, `field_6`. The show/hide rule is the first plan's step 7 |
| Шрифт | `data-font` plus `font-family` | `text.font` and `resources.fonts` | The list is the key `text` (Liberation Serif) and each uploaded `.ttf` or `.otf`. A new box uses `text` |

`editor_state` stays null. A palette drop appends an entry and assigns `p1-t<n>`, `p1-i<n>`, or `p1-b<n>` on the object, then the view draws that entry. A drag writes position and size on the entry with that id.

The text node of a variable is the flat mustache. `variableExpression` on this route writes `{{field_1|upper}}` into `preentered`. `data-var` stays `form_data.field_1` so the trait panel still knows which palette row it is. Prefix and suffix are characters around that mustache. `content` is not a trait. The entry keeps `content: null`. Delimiters stay `{{` `}}` and have no control.

Code mode edits the page JSON. A successful apply replaces the object and rebuilds the canvas. A JSON value the writer would reject stays on screen and does not replace the object.

`fit_text.js` is not loaded into this canvas. There is no minimum size next to `auto_scale`. That floor is a schema gap in the mapping.

### One box and the writer subset

A badge that scales as one block is one text entry with several mustaches, which is the `cnews-spb.yml` shape. Dropping a palette field while a text box is selected appends `<br>` and the flat mustache inside that box. A drop with no text box selected still creates a new box.

Double-click edits the `preentered` string, tags included. The stock rich-text session is not used on this route, so GrapesJS does not rewrite the string into extra `div`s. The canvas renders the subset below for display and keeps the source on `data-preentered`.

| In the string | Page |
|---|---|
| `<br>` | A line break inside `preentered` |
| `<b>`, `<bold>`, `<i>`, `<u>` | The same tags |
| `<span font="…" style="color: #RRGGBB">` | A run whose face is a `resources.fonts` key and whose colour overrides the box |
| `<leading>…</leading>` | A leading change for the following run, in millimetres |

The box traits are the defaults for a run that does not override them. A second colour or a second face is a span in the string, not a second trait. Any other tag makes the page fail the check in [step 9](#step-9-save-the-page-json-as-yaml).

### Style manager

`yamlPage` replaces the sectors in `visual-editor.component.ts`. Each control writes the page field named here.

| Control | Page |
|---|---|
| `left`, `top`, `width`, `height` in mm only | `posX`, `posY`, `width`, `height`. The `%` unit is removed |
| `font-size` in pt | `text.font_size` via `mm = pt * 25.4 / 72` |
| `font-weight`, italic, underline | `text.font_style` |
| `text-align` | `alignment.horizontal` |
| `line-height` unitless | `text.line_height` in millimetres. `leading` is the separate trait |
| `color` | `text.color` |
| `padding` in mm | `padding` |
| Rotation `0` / `90` on a text box | `rotation`. `90` reads upward. Any other angle stays off this route |
| `background-color` | `background` on the entry. This is the box fill from [step 1](#step-1-box-background), not `page_size.background` |
| Border width and color | `border` |
| `border-radius` | `border_radius` |
| `opacity` | `opacity` |
| `letter-spacing` | `text.letter_spacing` |
| Scale X, scale Y, skew X, skew Y | `scale_x`, `scale_y`, `skew_x`, `skew_y` |

Removed on this route, because the PDF standard has no feature for them: `white-space`, `text-overflow`, `overflow`, and a `%` width or height. A line is not added.

### Codes and pictures

A code box stores `width` and `height` as the rectangle the writer scales the symbol into. The trait for dots per module is removed, and so is the PNG/SVG switch. A QR from the palette starts square. EAN-13 and QR stay the two palette codes. vCard stays disabled. `code128` and `photo` stay round-trip only.

An uploaded font is drawn on the canvas from the asset object URL. The page entry stores `text.font` as that resource key. The picture has no `@font-face`, no `<link>`, and no `<style>` pointing at the file.

### Bleeds and margins

The toolbar gains four bleed inputs, millimetres, default `0`. They edit `page.page_size.bleeds`. `print.models.ts` gains `page`.

The sheet the user drags on is the trim (`page_size.width` and `height`). The canvas draws the bleed outside that sheet. A block may sit at a negative `left` or `top`, which is a mark in the bleed. Changing the bleed numbers does not move boxes.

The existing margin inputs stay. They still go to `meta.json`. They are separate from bleeds, and an edit does not add them to `posX` or `posY`.

### Check

`npm start` in `ticket-editor/`, `npm run start:editor` in the frontend. Open `http://localhost:4200/editor/1`.

1. The style manager has background, border, corner radius, opacity, letter-spacing, scale, and skew. It has no percent width. Text rotation is only `0` or `90`. The text traits include vertical alignment, Уместить текст, and the leading box.
2. Drop Фамилия. The canvas shows `Петрова`. The font control reads Liberation Serif. The page object has the new text entry before Save. Save. `template.yml` has `template: true`, `preentered: "{{field_1}}"`, `font: text`, `alignment.vertical: top`, `leading: 0`, and millimetre `posX` / `posY` / `width` / `height`. `auto_scale` is absent. `content` is null.
3. Set the variable to `|upper`. The canvas shows `ПЕТРОВА`. Save writes `{{field_1|upper}}`.
4. Set vertical alignment to `middle` and turn Уместить текст on. Save writes `alignment.vertical: middle` and `auto_scale: true`. Reload keeps both.
5. Select that box and drop Имя. The same entry's `preentered` becomes `{{field_1|upper}}<br>{{field_2}}`. One box, two fields.
6. A block with `data-if="field_6"` is visible. Save writes `ifSt: field_6`.
7. Set bleeds to 3 mm on each side. Save writes `page_size.bleeds` of 3. The boxes stay where they were. The margin inputs still do not appear in `template.yml`.
8. `/operator/templates/:id` on a normal `npm start` still shows the background and border controls.

## Step 11. Form backend and pdf-tooling in Compose

[Steps](#steps)

✅ Completed.

`ticket-editor/Dockerfile` builds the Node process. The build context is the repository root, so the image can copy `ticket-editor/`, `proto/irbis/pdf/v1/pdf.proto`, and `tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf`. `npm ci --omit=dev` installs the runtime dependencies, including `yaml`, `jszip`, `@grpc/grpc-js`, and `@grpc/proto-loader`.

The process listens on `BIND_HOST` (default `127.0.0.1` for `npm start` on the host, `0.0.0.0` in Compose) and port `4310`. `DEFAULT_FONT` is the path of the Liberation file inside the image. `PDF_PROTO` is the proto path. `PDF_TOOLING_ADDR` is the service address.

`ticket-editor/docker-compose.yml` has two services:

| Service | Build | Address |
|---|---|---|
| `editor` | `ticket-editor/Dockerfile`, context `..` | Published `4310:4310`. `PDF_TOOLING_ADDR=pdf-tooling:50052`. Bind mount `./data/templates` at `/app/data/templates` |
| `pdf-tooling` | `docker/pdf-tooling/Dockerfile`, context `..` | `expose` `50052` on the compose network. Same image as the root compose file. This file does not publish `50052` |

`editor` depends on `pdf-tooling` being healthy. The pdf-tooling healthcheck is the one already in `docker/pdf-tooling/Dockerfile` (`/usr/local/bin/tcpcheck`). The editor healthcheck is a `fetch` of `http://127.0.0.1:4310/api/print_template/1`.

The root `docker-compose.yml` keeps excel-tooling, pdf-tooling, and google-tables for those tools. This file is the one the editor runs. Starting both at once is fine: they do not share a published port.

### Check

From `ticket-editor/`:

```bash
docker compose up --build -d
curl -sS http://127.0.0.1:4310/api/print_template/1
docker compose ps
```

The GET is the blank 100×100 detail. `editor` and `pdf-tooling` are healthy. Stop is `docker compose down`. Leave the stack up for [step 16](#step-16-preview-endpoint).

## Step 12. List the templates

[Steps](#steps)

✅ Completed.

`GET /api/print_template/list` reads `data/templates`. Each directory whose name is a template id and that has `meta.yml` or `meta.json` is one item. A directory without either file is left out. `pir` is included. The item is the list row the screen needs: `id`, `name`, `source`, `is_default`, `updated_at`, `updated_by_name`, `page_width_mm`, `page_height_mm`. Width and height come from `page_size` in `template.yml` when that file exists, otherwise from the meta fields. The array is ordered by `id`.

`GET /api/print_template/:id` is unchanged. This route does not create, copy, import, export, or delete a template.

### Check

`npm test` in `ticket-editor/`. No Angular app.

1. The seeded template is the only row: id `1`, name `Badge`, 100×100.
2. A second directory `data/templates/2` with its own `meta.yml` and `template.yml` appears as the second row.
3. A directory `data/templates/pir` with `meta.yml` appears. A directory with only `meta.json` appears. A directory without either file does not.
4. `GET /api/print_template/1` is still the blank detail.

## Step 13. Editor app and its route table

[Steps](#steps)

✅ Completed.

Add `ticket-editor/frontend`, an Angular app beside `app.mjs`. It has its own `package.json`. `npm start` serves it on port 4200. The Node process is not this app.

`src/app/app.routes.ts` exports `routes`. `app.config.ts` passes that array to `provideRouter`. That file is the route table a build configuration can replace, the same way the PIR `cloud-public` configuration replaces `app.routes.ts`. This step ships one table, and the default build uses it as written. The paths, in order:

| Path | What it shows |
|---|---|
| `editor/list` | A placeholder whose text is `Макеты`. |
| `editor` | Redirects to `/editor/1`. `pathMatch` is `full`, so it does not swallow `editor/list`. |
| `editor/:id` | A placeholder whose text is `Редактор`. |
| `**` | Redirects to `/editor/list`. |

No route path is `operator`, `setup`, `info`, or a ticket-type id. Login guards, Formly, and the registration screens are not in this app.

### Check

From `ticket-editor/frontend`:

```bash
npm test
npm run build
```

The route spec loads `routes` and expects the paths `editor/list`, `editor`, `editor/:id`, and `**`, in that order. A router harness navigates to `/editor/list`, `/editor/1`, `/editor`, and `/nope`. `/editor/list` shows `Макеты`. `/editor/1` shows `Редактор`. `/editor` ends on `/editor/1`. `/nope` ends on `/editor/list`. `npm run build` finishes. The PIR app is not compiled.

## Step 14. Canvas in the editor app

[Steps](#steps)

✅ Completed.

Replace the `:id` placeholder with the page editor copied from `PIR_REG/expocab-offline-registration/frontend/src/app/operator/templates/`. Copy the layout shell, the template editor, the visual editor, the code editor, the palette, `page-view.ts`, the snippets, the print models and the print API methods the canvas calls, the toast, `styles.scss`, and the Tailwind setup those classes use. Copy the GrapesJS stylesheet asset. `yamlPage` is true. `apiBaseUrl` is `http://127.0.0.1:4310/api`. Formats and the demo guest stay the TypeScript objects in `ticket-editor/src`. The route data sets `localCatalog: true`.

«← Макеты» in this copy points at `/editor/list`. The PIR copy keeps `/operator/templates`. Do not import the Django print contribution, `setupGuard`, or `operatorGuard`.

`ticket-editor/e2e/paths.mjs` sets `frontendRoot` to `ticket-editor/frontend`. Playwright still serves that app on port 4201 with `--configuration` left as the app's default serve, and still uses system Chrome.

The PIR `editor` configuration and `/operator/templates/:id` stay where they are. This plan no longer starts them.

### Check

`npm test` in `ticket-editor/frontend` still passes the route spec, and the `page-view` spec passes there.

`npx playwright test e2e/save-page.spec.mjs` from `ticket-editor/` passes. The dev server it starts is `ticket-editor/frontend`, not the PIR app. `/editor/1` loads the sheet. `/operator/templates/1` is not a route of this app.

## Step 15. Template list

[Steps](#steps)

✅ Completed. `/editor/list` reads `GET /api/print_template/list` and shows each name, sheet size, and updated time. The name and «Редактировать» open `/editor/:id`. `e2e/template-list.spec.mjs` seeds `Hall` and `Visitor`, opens the second, and checks `/setup` and `/editor`.

Replace the list placeholder with the catalog. It calls `GET /api/print_template/list` and nothing else. The list is flat. A row shows the name, the sheet size in millimetres, and the updated time. The name and «Редактировать» go to `/editor/:id`.

Create, copy, import, export, «по умолчанию», and delete stay on `/operator/templates` in the PIR app. This list does not call the ticket-type endpoint, and it does not post a new template.

`/editor` still redirects to `/editor/1`.

### Check

`npm test` in `ticket-editor/` still passes the list route from [step 12](#step-12-list-the-templates).

A Playwright spec next to `e2e/save-page.spec.mjs` seeds templates `1` and `2` with different names, opens `/editor/list`, and sees both names. Clicking the second name lands on `/editor/2` and `#template-name` is that name. `/setup` ends on `/editor/list`. `/editor` ends on `/editor/1`.

## Step 16. Preview endpoint

[Steps](#steps)

✅ Completed. `POST /api/print_template/preview` validates `document`, zips the preview copy with the template's font and image files, and returns the pdf-tooling PDF. The copy keeps text as written, drops an `ifSt` that is false for the demo guest, and fills a barcode (`{{code}}` becomes `2000000000008`, `template` omitted). A text entry missing `alignment` or `padding` gets `left` / `top` and zero padding in that copy only. `npm test` uses a fake client. The compose curl returns `Content-Type: application/pdf` and `%PDF-`.

The caller is the preview contribution in [toolbar-contributions.md](toolbar-contributions.md#example-a-preview-button-of-your-own). `PrintApiService.preview` already posts `POST /api/print_template/preview`. This step is that route on the Node process. The editor component does not grow a preview method.

`PreviewRequest` gains `document`, the page object from `EDITOR_SHEET.document()`. The click handler sends `template_id`, `document`, and `output: 'pdf'`. `document` is the object currently in the browser, so the PDF includes edits that are not saved yet. Assets are the files already stored for `template_id`. The handler checks `document` the same way [step 9](#step-9-save-the-page-json-as-yaml) checks a save, and keeps the preview copy in memory. `template.yml` on disk changes only on PUT. `output` other than `pdf` is 400. The Django cluster keeps posting `html` and `page` to Django, and is not registered on this route.

It then builds a preview copy of that page:

- Text entries stay as written, pipes included.
- An entry whose `ifSt` is false for `demoContext` is omitted from the copy. The stored file still has it. The value used for the test is the raw sample (`field_6` boolean `true` draws). The same table as the first plan's step 7 decides.
- A barcode entry is filled in the copy: `preentered` becomes the code string (`2000000000008` for `{{code}}`), and `template` is omitted. `Render` fills text holes. A barcode left as `template: true` is indexed, and `Render` then looks that id up as a text box and returns an error. The stored page keeps `template: true` and `{{code}}`.

The copy, the font files, and the image files are zipped. Paths inside the zip are the `source_path` and `extracted_path` values (`fonts/LiberationSerif-Regular.ttf`, `files/logo.png`) plus `template.yml`. The handler calls `irbis.pdf.v1.PdfTooling`:

1. `LoadTemplate`. The first message is `TemplateMeta.filename = "template.yml"`. The following messages are the zip bytes.
2. One `ValueRow`. Each name in `TemplateView.fields` is a key. The value is the demo sample for the flat name, with the four pipes applied in the order the first plan stores them (join, then default, then case). `field_1|upper` is sent as the field name `field_1|upper` and the value `ПЕТРОВА`. Lists are joined to one string. Booleans are `true` or `false`. The row contains only names the view returned.
3. `RenderChunks`. The handler concatenates `PdfChunk.data`.
4. `Close` for that `session_id`, including when render fails.

The HTTP response is `200`, `Content-Type: application/pdf`, and the PDF bytes. The contribution opens that blob in a new tab. A page the writer would reject is 400 `{ "detail" }` and does not call the service. A service `INVALID_ARGUMENT` is 400 `{ "detail" }` with the status message. A service the process cannot reach is 502 `{ "detail": "pdf-tooling unavailable" }`.

`createPdfClient(addr)` is the gRPC client. `npm test` passes a fake client and checks the zip and the row without a server. The zip's `template.yml` contains the posted text entry. The row for `{{field_1|upper}}` is `ПЕТРОВА`. A barcode in the zip has the literal code and no `template: true`. An `ifSt` that is false for the fixture is absent from the zip and still present in the stored file.

### Check

Compose from [step 11](#step-11-form-backend-and-pdf-tooling-in-compose) is up. `npm test` passes the fake-client cases.

```bash
curl -sS -D /tmp/preview.headers -o /tmp/badge.pdf \
  -X POST http://127.0.0.1:4310/api/print_template/preview \
  -H 'Content-Type: application/json' \
  -d '{"template_id":1,"output":"pdf","document":{"page_size":{"width":100,"height":100,"units":"mm","bleeds":{"top":0,"right":0,"bottom":0,"left":0}},"resources":{"images":{},"fonts":{"text":{"name":"text","font_family":"Liberation Serif","weight":"normal","italic":false,"embedded":true,"source_path":"fonts/LiberationSerif-Regular.ttf"}}},"contents":[{"id":"p1-t1","posX":10,"posY":20,"width":80,"height":12,"template":true,"text":{"font":"text","font_size":6.35,"leading":0,"line_height":7.62,"preentered":"{{field_1|upper}}","content":null}}]}}'
```

`/tmp/preview.headers` has `Content-Type: application/pdf`. `/tmp/badge.pdf` begins with `%PDF-`.

## Step 17. Preview button opens the PDF

[Steps](#steps)

`/editor/:id` registers `pdfPreviewContribution` from [toolbar-contributions.md](toolbar-contributions.md#example-a-preview-button-of-your-own). The button is `PdfPreviewToolbarComponent`. It injects `EDITOR_SHEET` and `PrintApiService` and posts on its own. The base editor from [implementation-plan.md](implementation-plan.md#step-5-plug-toolbar-actions-into-the-route) does not grow a flag, and the button is not added to `template-editor.component.html`. Как на Zebra, Тест-печать, Экспорт, and the registration-number field stay on the operator route's Django contribution.

`EDITOR_SHEET.document()` returns the page object. The click handler is the one in that example: `template_id`, `document`, `output: 'pdf'`, then `window.open` of the blob. While the request runs the label is `Рендер…`. A failure calls `showError('Не удалось построить превью.')`. The operator route (`npm start` without the editor configuration) is unchanged and still posts `html` and `page` to Django.

### Check

Compose is up. `npm start` in `ticket-editor/frontend`. On `http://localhost:4200/editor/1` the toolbar has Превью PDF, drawn by the contribution, to the left of Save. Click it. The browser opens a PDF tab. After a variable has been dropped and the case is unset, the page shows `Петрова`. With `|upper`, the PDF shows `ПЕТРОВА`. The request is `POST /api/print_template/preview` with `document` and `output: "pdf"`.

`/operator/templates/:id` on a normal `npm start` still shows the Django preview buttons.

## Step 18. Run

[Steps](#steps)

Steps 1–17 are already checked. This is the full click-through.

```bash
cd ticket-editor && docker compose up --build -d
cd ticket-editor/frontend && npm start
```

Open `http://localhost:4200/editor/list`. The seeded template is listed. Open `http://localhost:4200/editor/1`.

1. Drop Фамилия and a QR. Save. `template.yml` has a text entry `{{field_1}}` and a barcode entry `type: qr`, `template: true`, `preentered: "{{code}}"`.
2. Upload a PNG named `logo`, drop it, save. The page has `resources.images.logo.extracted_path` under `files/`, and an image entry whose `resource_name` is `logo`.
3. Превью PDF opens a tab. The page shows `Петрова`, a QR of `2000000000008`, and the logo.
4. Reload. The GET `page` has the three entries. The sheet shows them again. The file is still the YAML page.

## Out of this cut

- Syncing two browsers. The page object is the document a CRDT would merge: one map per entry, order in `contents`. This cut keeps a single browser and a single PUT.
- Zebra preview, test print, ZIP export, operator login.
- A vCard palette item. `has_vcard` stays false. A page that is not text, image, barcode, or photo fails the save.
- Adding `photo` or `code128` to the palette. Entries already on a page round-trip.
- `white-space: nowrap`, ellipsis, percent size, and a `z-index`. The PDF standard has none of these. [Step 10](#step-10-edit-the-page-on-the-canvas) removes those controls.
- Dots per module, and a minimum font size beside `auto_scale`.
- A rotation other than `0` or `90`. The text matrix could hold any angle. This page keeps the parameter it already has.
- Baking toolbar margins into `posX` / `posY` or into bleeds.
