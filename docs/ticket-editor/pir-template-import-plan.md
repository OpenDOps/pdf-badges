# Import one PIR badge and open it in the editor

A test of the HTML-to-page path. Download one visual PIR print template as a ZIP, read its HTML and CSS, write `template.yml`, and open that page in the ticket editor. The canvas should show the same boxes.

The source is the cloud ticket-template archive. A node token downloads it from `https://prd.api.pir.ru/api/v4/offline_registration/ticket_type/1548/ticket_template`. The bytes are a ZIP: `template.html`, `manifest.json`, and `files/`. Styles live in `template.html` (a `<style>` block and `style` on each box). A `.css` file named by the manifest is part of the same style. The archive shape is section 7 of `PIR_REG/expocab-offline-registration/docs/TZ_print_templates.md`. The PDF analyzer is not this path. It describes a drawn PDF. This plan reads the template before it is drawn, so the mustaches stay.

Field rules are [pdf-yaml-mapping.md](pdf-yaml-mapping.md). The editor already draws a page through `pageToHtml` in `page-view.ts`. This plan adds the opposite direction and one fixture that proves it.

This plan does not change Save, preview, or the operator route. It does not import `{% if %}` or `{% for %}`.

## Steps

Each step has a Check. The check uses only what that step added.

| Step | What it does | Status |
|---|---|---|
| [1. Download the ZIP](#step-1-download-the-zip) | One visual badge archive, HTML plus style, on disk. | ✅ Completed |
| [2. Record the fixture](#step-2-record-the-fixture) | Unpack it. The style and the boxes are the expected input. | ✅ Completed |
| [3. HTML and CSS to YAML](#step-3-html-and-css-to-yaml) | One script writes `template.yml`, `meta.yml`, and the files. | ✅ Completed |
| [4. The writer accepts the page](#step-4-the-writer-accepts-the-page) | `load_page` reads the YAML. Dropped CSS is listed. | ✅ Completed |
| [5. The editor serves that page](#step-5-the-editor-serves-that-page) | GET returns the imported object. | ✅ Completed |
| [6. The canvas shows the boxes](#step-6-the-canvas-shows-the-boxes) | `/editor/<id>` matches the fixture positions and text. | ✅ Completed |

## Checking a step

| What the step added | How to check it |
|---|---|
| The ZIP and the unpacked HTML | The files exist. No server. |
| The converter | `npm test` in `ticket-editor/` on a small HTML fixture. No browser. |
| The imported page | `cargo test --test struct_to_pdf` is unchanged. A one-off `load_page` of the imported YAML is the check in step 4. |
| GET | `npm start` in `ticket-editor/`, then curl. No Angular app. |
| The canvas | The ticket editor on port 4213, then the checklist in step 6. |

## Step 1. Download the ZIP

[Steps](#steps)

✅ **Status: done.** `ticket-editor/fixtures/pir/source.zip` is the ticket type 1548 archive, 402047 bytes. It unpacks to `template.html`, `manifest.json`, and one JPEG.

Download the cloud badge for «Билет посетителя ПИР ЭКСПО 2026». That ticket type is cloud id `1548`. Both registration nodes carry only this ticket type. The call is the one the node makes in `get_ticket_template_archive` (`PIR_REG/expocab-offline-registration/backend/registration/sync/client.py`): `GET ticket_type/<id>/ticket_template` with the node token.

From the repository root:

```bash
mkdir -p ticket-editor/fixtures/pir
curl -fsS -o ticket-editor/fixtures/pir/source.zip \
  -H "Authorization: Token 443dd37a88a587a06bea1c7e7f8420f7b860545d" \
  https://prd.api.pir.ru/api/v4/offline_registration/ticket_type/1548/ticket_template
```

| | |
|---|---|
| Cloud | `https://prd.api.pir.ru` |
| API prefix | `https://prd.api.pir.ru/api/v4/offline_registration/` |
| Ticket type | `1548`, survey `7`, «Билет посетителя ПИР ЭКСПО 2026» |
| Header | `Authorization: Token <node token>` |
| HALL13 token | `443dd37a88a587a06bea1c7e7f8420f7b860545d` |
| HALL14 token | `f90a6242ea1780fe2d9639bc232bf842a222cdae` |

The curl above uses the HALL13 token. HALL14 is the same ticket type, so its token returns the same archive. The tokens are the node tokens from the stand guide [Установка узла оффлайн-регистрации на стойке](https://cpoppe-mdviewer.containerapps.ru/d/jqlIM87g9PASqu5ioDQc5MbNpzvkOYUugXQ_-_UkztE/). A `401` or `403` means the token is wrong or the node is disabled. A `404` means ticket type `1548` has no cloud template.

The response is the ZIP bytes, not JSON. A cloud archive has no `format_version` and no `page`. The page is then A4, portrait, with 2 mm margins on every side. Inside the ZIP:

```text
template.html
manifest.json
files/…
```

`editor.json` may be present. The converter does not read it. `template.html` holds the sheet style and the box styles. A `.css` file is included when `manifest.json` names one.

The stand guide also names `ticket_visitor_pirexpo2026_bw.zip`, the black-and-white A4 badge imported on the node by hand. That file is not this request. The token downloads the cloud template only.

The body this test can convert is absolutely positioned boxes (`tpl-text`, `tpl-var`, `tpl-code`, `tpl-file`, or a plain `div` with `position: absolute`), with no `{% %}` except `{% load static %}`, no `<script>`, and no `<link>`. Do not commit the ZIP if `template.html` contains a filled-in guest. A template that still has `{{ }}` is the one to keep.

### Check

`source.zip` is a zip file and unpacks with `template.html` and `manifest.json`. `template.html` contains a `<style>` block or inline `style` attributes, and at least one `mm` length.

## Step 2. Record the fixture

[Steps](#steps)

✅ **Status: done.** Unpacked to `ticket-editor/fixtures/pir/unpacked/`. `expected.md` lists the four absolute boxes and the full-page JPEG. `sample.html` is the 90×130 one-box badge.

Unpack `source.zip` to `ticket-editor/fixtures/pir/unpacked/`. Add `ticket-editor/fixtures/pir/expected.md` with the facts the later checks compare:

| Fact | Where it comes from |
|---|---|
| Sheet width and height, mm | `body` in the style, or `manifest.json` `page.width_mm` / `page.height_mm`. A named format uses the millimetre sides from `formats()`. Landscape swaps them. A cloud archive with no `page` is 210×297. |
| Margins, mm | `page.margins_mm`, or 2 mm on every side when the archive has no `page` |
| Box count | Body children that are absolutely positioned |
| Each box | Class or marker, `left`, `top`, `width`, `height`, the text or `src`, the background and border if present |
| Files | Keys in `manifest.files` and the paths they name |
| Font files | `.ttf` or `.otf` under `files/`. A `.woff` or `.woff2` is listed and is not embedded |

Also add a small HTML file, `ticket-editor/fixtures/pir/sample.html`, copied from the shape in [pdf-yaml-mapping.md](pdf-yaml-mapping.md): a 90×130 mm body, one centered text box at `13mm` / `49.5mm`, size `64mm` × `37.5mm`, `font-size: 24pt`, `line-height: 1.2`, `color: #FF6A00`, text `{{company}}<br>{{surname}}<br>{{name}}`. The unit test in the next step uses this file so the check does not depend on `source.zip`.

### Check

`expected.md` lists every absolute box in the unpacked HTML. `sample.html` is the one-box badge above.

## Step 3. HTML and CSS to YAML

[Steps](#steps)

✅ **Status: done.** `ticket-editor/scripts/html-to-page.mjs` writes the page. `html-to-page converts sample.html into one text box` and `html-to-page places the PIR sheet image first at the page size` cover both inputs. The PIR run writes the sheet JPEG as the first box, then the four positioned boxes.

Add `ticket-editor/scripts/html-to-page.mjs`. It reads an unpacked directory, or `sample.html` plus an optional manifest, and writes:

```text
ticket-editor/data/templates/<id>/
  template.yml
  meta.yml
  files/<name>.<ext>
  fonts/<file>.ttf
```

Use template id `pir` for `source.zip`. The script prints a short report: boxes written, files copied, and CSS declarations skipped.

The page is one object `load_page` accepts. No `pages` key and no `metadata` key. `units` is `mm`. Every bleed is `0`.

Sheet size is the body width and height when those are millimetres. Otherwise it is the manifest page. Margins are stored in `meta.yml` and are not applied to `posX`, `posY`, `width`, or `height`. A box at `left: 110mm` and `width: 100mm` stays `posX: 110` and `width: 100`.

Walk body children in order. Each absolutely positioned box is one `contents` entry. Assign `p1-t<n>`, `p1-i<n>`, or `p1-b<n>`.

An `img` of `{{ files.name }}` with no `position: absolute`, whose width and height are the body (or omitted), is the sheet image. It is written first, before those boxes, so they paint on top of it. `posX` and `posY` are `0`. `width` and `height` are the page size.

| HTML | YAML |
|---|---|
| `left`, `top`, `width`, `height` in `mm` | `posX`, `posY`, `width`, `height` |
| `tpl-text` or `data-text` | Text entry. `preentered` is the text. `content: null` |
| `tpl-var` or `data-var` | Text entry, `template: true`. One mustache: strip `form_data.`, keep `\|upper`, `\|title`, `\|join`, and `\|default`. `data-prefix` and `data-suffix` are characters around it |
| Several `{{ }}` in one box | One `preentered` string, `template: true` |
| `tpl-code` `barcode` | `barcode.type: ean13`, `template: true`, `preentered: "{{code}}"` |
| `tpl-code` `qrcode` | `barcode.type: qr`, same payload |
| `tpl-code` `vcard` | `barcode.type: qr`. `preentered` is `{{vcard}}`. The page has no vCard builder |
| `tpl-file` or `{{ files.name }}` | `image.resource_name`. Copy the manifest file to `files/` and add `resources.images` |
| A plain absolute `div` with a background or border and no text | Text entry, `preentered: ""`, plus the frame fields |
| `font-size` in `pt` | `text.font_size` in mm: `mm = pt × 25.4 / 72` |
| Unitless `line-height` | `text.line_height = font_size × ratio`, `leading: 0` |
| `color` as `#RRGGBB` or `rgb()` | `text.color`. Black omits `color` |
| `font-weight: bold` or `600` and up, italic, underline | `text.font_style` |
| `text-align` | `alignment.horizontal` |
| `data-valign` | `alignment.vertical`. Absent writes `top` |
| `letter-spacing` in `mm` | `text.letter_spacing` |
| `background` or `background-color` | `background` |
| `border: <n>mm solid #RRGGBB`, or `border-width` plus `border-color` | `border.width`, `border.color` |
| `border-radius` in `mm` | `border_radius` |
| `padding` in `mm` | `padding` on the four sides |
| `opacity` | `opacity` when it is not `1` |
| `data-fit` | `auto_scale: true`. The minimum size is omitted |
| `.ttf` or `.otf` in the ZIP | `resources.fonts`, `embedded: true`, `source_path` under `fonts/`. `text.font` is that key |
| No font file | `text.font: text`, the Liberation Serif face the editor already seeds |

`font-family` alone does not embed a face. A `.woff` or `.woff2` is copied into `files/` and listed in the report. The text box still uses `text`.

These declarations are skipped and named in the report. They are not page fields: `white-space`, `text-overflow`, `overflow`, a length in `%`, `z-index`, `vertical-align`, and `transform`. `data-dots` and `data-variant` are skipped. A length in `px` converts first: `mm = px × 25.4 / 96`.

`meta.yml` holds `id`, `name` from the manifest, `source: local`, the four margins, and `manifest`. `html` is omitted. `editor_state` is omitted. `ticket_type`, `ticket_type_name`, `page_format`, and `orientation` are omitted.

### Check

From `ticket-editor/`:

```bash
node scripts/html-to-page.mjs fixtures/pir/sample.html --out data/templates/sample
npm test
```

`npm test` runs the same conversion on `sample.html` and asserts the page: `90` × `130`, bleeds `0`, one text entry, `posX: 13`, `posY: 49.5`, `width: 64`, `height: 37.5`, `font_size` about `8.467`, `color: "#FF6A00"`, `preentered` containing `{{company}}`, `{{surname}}`, and `{{name}}`, `template: true`, `content: null`.

Running the script on `fixtures/pir/unpacked` writes `data/templates/pir/template.yml`. The report's box count equals the count in `expected.md`.

## Step 4. The writer accepts the page

[Steps](#steps)

✅ **Status: done.** `imported_html_pages_load` calls `load_page` on both converted files. The PIR page starts with the sheet image. `expected.md` lists the skipped `display` lines.

Run the imported YAML through `load_page`. A page it rejects is a converter bug, not a preview bug. Fix the script and run step 3 again.

The report from step 3 is the list of CSS that the canvas will not show. `expected.md` gains a "Skipped" section copied from that report. A skipped `nowrap` or `data-dots` is expected. A skipped `left` or `font-size` is a failure.

### Check

From the repository root, a small program or a test calls `load_page` on `ticket-editor/data/templates/pir/template.yml` and on `data/templates/sample/template.yml`. Both return a page. `cargo test --test struct_to_pdf` stays green.

## Step 5. The editor serves that page

[Steps](#steps)

✅ **Status: done.** A template id may be `pir` as well as a number. `GET /api/print_template/pir serves the imported page and its sheet image` checks the page and the JPEG.

`ticket-editor/` already reads `template.yml` and `meta.yml`. No new route. Start the process with the data directory that step 3 wrote.

Seed `fonts/LiberationSerif-Regular.ttf` into `data/templates/pir/fonts/` when the imported page uses the `text` font. The blank-template seed already knows this file: `tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf`.

### Check

```bash
cd ticket-editor
npm start
curl -s http://127.0.0.1:4310/api/print_template/pir
```

The JSON has `page.page_size` equal to the sheet in `expected.md`, `page.contents` length equal to the absolute boxes plus the sheet image, and `html` null. The first entry is that image at `0, 0` with the page width and height. `editor_state` is null. One text entry's `preentered` still contains `{{`. An image entry's `resource_name` is a key in `manifest`, and `GET /api/print_template/pir/assets/<name>` returns the file.

## Step 6. The canvas shows the boxes

[Steps](#steps)

✅ **Status: done.** The ticket editor at `http://127.0.0.1:4213/editor/pir` loads the imported page. Visual mode draws the 210×297 mm sheet, the JPEG at `0, 0`, and the four boxes at the HTML positions. The first text box is `110mm` from the left and `100mm` from the top. The barcode and the QR demo images both paint. Code mode shows the same object as JSON.

Start the ticket editor against that process. `environment.ts` sets `yamlPage: true` and `apiBaseUrl` to `http://127.0.0.1:4310/api`.

```bash
cd ticket-editor/frontend
npx ng serve --port 4213 --host 127.0.0.1
```

Open `http://127.0.0.1:4213/editor/pir`.

The screen loads `page` from GET and draws it with `pageToHtml`. Visual mode is the view to check. Code mode shows the same object as JSON.

Compare the canvas with `expected.md`:

| On the canvas | Matches |
|---|---|
| Body size | Sheet width and height |
| Block count | The sheet image, then the absolute boxes |
| `data-yaml-id` | The entry `id` |
| `left`, `top`, `width`, `height` | `posX`, `posY`, `width`, `height` |
| A one-mustache box | The flat mustache, such as `{{field_1}}` |
| A multi-mustache box | The demo guest for those keys, because `pageToHtml` substitutes when a box has more than one `{{ }}` |
| `font-size` | Points, `pt = mm × 72 / 25.4` |
| Color, background, border, radius, padding, opacity | The frame and text fields |
| A code | `tpl-code` with `data-code="barcode"` or `data-code="qrcode"`, and the demo image |
| A file | `tpl-file` and the asset bytes |
| Font | The family from `resources.fonts`. A box with no uploaded face shows Liberation Serif |

A declaration listed under Skipped in `expected.md` is absent on the canvas. That is the expected difference. `display: inline-block` is the skipped line, so the name boxes are ordinary text blocks.

The open screen is the check. `save-page.spec.mjs` covers the same canvas on the blank template. This page was compared on `/editor/pir` directly.

### Check

On `http://127.0.0.1:4213/editor/pir`, the sheet is 210×297 mm and the canvas has the sheet image plus the four boxes. The first text box is at `left: 110mm` and `top: 100mm`. Code mode shows that `posX` and the `preentered` `{{field_108}} {{field_107}}`.
