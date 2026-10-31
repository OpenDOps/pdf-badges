# Ticket editor

The badge editor in `PIR_REG/expocab-offline-registration`. Spec: `PIR_REG/expocab-offline-registration/docs/TZ_print_templates.md`, section 6. Source is TypeScript.

| File | Role |
|---|---|
| `frontend/src/app/operator/templates/template-editor.component.ts` | Screen: load, save, page size, mode switch |
| `frontend/src/app/operator/templates/template-editor.component.html` | Toolbar, palette column, canvas, properties |
| `frontend/src/app/operator/templates/visual-editor.component.ts` | GrapesJS canvas |
| `frontend/src/app/operator/templates/code-editor.component.ts` | CodeMirror HTML |
| `frontend/src/app/operator/templates/template-snippets.ts` | Expressions, block HTML, document split/join |
| `frontend/src/app/operator/templates/variable-palette.component.ts` | Left-hand palette |
| `frontend/src/app/core/models/print.models.ts` | JSON shapes |
| `frontend/src/app/core/api/print-api.service.ts` | HTTP |
| `backend/registration/printing/page_formats.py` | `formats()` catalog and page rules |
| `backend/registration/printing/demo.py` | `demoContext()` |
| `backend/registration/printing/template_files.py` | Asset storage |
| `backend/registration/printing/printer.py` | Real print context the demo imitates |
| `backend/registration/models.py` `PrintTemplate` | Stored row |

Route today: `/operator/templates/:id`, behind operator login, full-screen shell (`EditorLayoutComponent`) with no registration header. Cloud templates (`source: "cloud"`) open read-only.

## Design

One template is one HTML document plus a page-size record plus a file manifest. Both edit modes write the same HTML. GrapesJS project data (`editor_state`) is stored so the visual mode can reopen without rebuilding from HTML. The printer reads `html` only. `data-var`, `data-code`, and `data-file` are editor markers. The Django renderer treats them as ordinary attributes.

```text
┌ name · ticket type · [Visual | Code] · sheet size · margins · zoom ·
│ [PDF preview] [Zebra preview] [test print] [export] [Save]
├──────────────────┬──────────────────────────────┬─────────────────────┐
│ Palette          │ Sheet, mm, rulers, 5 mm grid │ GrapesJS traits and │
│ variables        │ absolutely positioned boxes  │ style manager       │
│ ticket codes     │                              │ (visual mode only)  │
│ files + upload   │                              │                     │
└──────────────────┴──────────────────────────────┴─────────────────────┘
```

Visual mode is the default the user switches to. The component starts in code mode and flips to visual when the HTML has no `{% %}` (except `{% load static %}`), no `<script>`, and no `<link>`. Those three block the canvas because GrapesJS would rewrite them. Code mode can still edit them.

The canvas is GrapesJS with `dragMode: 'absolute'`, storage turned off, Russian locale. Drag and resize write CSS in millimetres, snapped to 0.5 mm. 1 mm = 96/25.4 CSS px, the same ratio Playwright uses, so a position on the canvas is the printed position.

Codes snap further. An EAN-13 is 117 modules wide including quiet zones. A QR width is `qr_modules × dots per module`. Allowed densities are 2–5 dots per module for EAN-13 and 3–6 for QR, at the printer dpi (203 when no Zebra is selected). A QR stays square. A barcode's height is free.

Variable and text blocks grow their font when the box height changes. Width-only resize leaves the font and changes the wrap width.

`fit_text.js` is injected into the canvas and into the print page. It is not stored in the template. An element with `data-fit="min:8pt"` (also `min:10pt`, `min:6pt`) steps `font-size` down by 0.5 pt until the text fits or the minimum is hit.

## Page structure

The printed page is the `<body>`. `joinDocument()` wraps whatever the canvas exports:

```html
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
body {
  margin: 0;
  position: relative;
  width: 100mm;
  height: 100mm;
  overflow: hidden;
  background-color: #ffffff;
}
</style>
</head>
<body>
  <!-- absolutely positioned blocks -->
</body>
</html>
```

`splitDocument()` pulls every `<style>` into the GrapesJS CSS and the body inner HTML into the canvas. `body` width and height are the sheet. Overflow is clipped. A grey desk and a 5 mm grid are canvas chrome and are not exported.

Page size is also stored on the row, and that row wins at print time. Playwright injects `@page { size: Wmm Hmm; margin: … }` from the row (`PageSpec.page_css`). The body rule is what the canvas edits.

A block is a child of `body` with `position: absolute` and `left` / `top` / `width` / `height` in mm. Origin is the top-left of the sheet. Y grows downward. A block may sit at a negative `left` or `top` (the style manager allows −500 mm). The dashed margin rectangle is a guide. It is not a clip, and it is not a bleed.

Four block kinds are recognised on the way back in:

| Class | Marker | Exported as |
|---|---|---|
| `tpl-text` | `data-text` | A `div` of literal text. Double-click edits it |
| `tpl-var` | `data-var` | A `div` whose text is one Django expression |
| `tpl-code` | `data-code` | `<img src="{{ barcode_svg }}">` (or qr / vcard, svg or png) |
| `tpl-file` | `data-file` | `<img src="{{ files.<name> }}">` |

Anything else (a plain `div` with a border, a hand-written `<img>`) round-trips as HTML. The style manager can set background, border, radius, padding, and opacity on any block, so a rectangle is a styled box. The palette itself only inserts the text block, the variables, the three codes, and the files.

Example, a name and a QR on a 100×100 mm badge:

```html
<div class="tpl-var" data-var="form_data.field_1"
     style="position:absolute;left:10mm;top:20mm;width:80mm;height:12mm;font-size:18pt;text-align:center;white-space:nowrap">
  {{ form_data.field_1 }}
</div>
<img class="tpl-code" data-code="qrcode" data-variant="svg" data-dots="4"
     src="{{ qrcode_svg }}"
     style="position:absolute;left:35mm;top:60mm;width:14.5mm;height:14.5mm">
```

On the canvas the variable shows `Петрова` (from `demoContext`). The saved file keeps the braces.

## Fields on a template

Stored on `PrintTemplate` and returned by `GET print_template/:id`.

| Field | Use |
|---|---|
| `id` | Route and asset URLs |
| `name` | Toolbar text. Saved with the HTML |
| `ticket_type`, `ticket_type_name` | Which survey the palette was built from. Shown, not edited |
| `source` | `local` editable, `cloud` read-only |
| `is_default` | The template used when a print does not name one |
| `html` | The document above. This is what prints |
| `editor_state` | GrapesJS project JSON. Null in code mode |
| `manifest` | `{ "logo": "files/logo.png" }`. Keys are the names in `{{ files.logo }}` |
| `page_format` | `A4`, `A5`, `A6`, `Letter`, `Legal`, or `custom`. `roll` is reserved and unused |
| `page_width_mm`, `page_height_mm` | Used when `page_format` is `custom`. Ignored for a named format |
| `orientation` | `portrait` or `landscape`. Swaps the named format's sides. A custom size is already the physical size, so orientation does not swap it |
| `margin_top_mm`, `margin_right_mm`, `margin_bottom_mm`, `margin_left_mm` | `@page` margin at print, millimetres, default 0 |
| `updated_at` | Optimistic lock. The save sends the value it loaded. Django returns 409 if another save landed first |

Save body (`PrintTemplateWrite`): `name`, `html`, `editor_state`, the page fields, and `updated_at`.

### Variable block

`data-var` is a Django path without braces.

| Attribute | Becomes |
|---|---|
| `data-var` | `form_data.field_12`, `ticket.number`, `ticket.ticket_code.code`, `ticket.ticket_type.name` |
| `data-prefix`, `data-suffix` | Literal text around the expression |
| `data-case` | `upper` → `\|upper`, `title` → `\|title` |
| `data-list` plus `data-join` | `\|join:"…"` for multi-select answers |
| `data-default` | `\|default:"…"` when the value is empty |
| `data-fit` | `min:10pt`, `min:8pt`, or `min:6pt`. Handled by `fit_text.js`, not by Django |

Style manager, stored as CSS on the same element: `left`, `top`, `width`, `height` (mm), `transform`, `font-family`, `font-size` (pt, mm, or px), `font-weight`, `font-style`, `text-align`, `line-height`, `letter-spacing`, `color`, `white-space`, `text-overflow`, `overflow`, `background-color`, `border`, `border-radius`, `padding`, `opacity`.

### Code block

| Attribute | Meaning |
|---|---|
| `data-code` | `barcode` (EAN-13), `qrcode`, or `vcard` |
| `data-variant` | `svg` (default) or `png` |
| `data-dots` | Printer dots per module. Sets the width |

`src` is one of `{{ barcode_svg }}`, `{{ barcode }}`, `{{ qrcode_svg }}`, `{{ qrcode }}`, `{{ vcard_qrcode_svg }}`, `{{ vcard_qrcode }}`. Those context keys are data URIs produced at print time from the ticket code. The caption under a code is a separate text or variable block (`ticket.ticket_code.code`), not part of the image.

### File block

`data-file="logo"` exports `src="{{ files.logo }}"`. The file bytes live beside the template, named in `manifest`.

## `formats()`

`GET /api/print_template/formats`. No arguments. Built by `formats_catalog()` in `page_formats.py`. It is the dropdown data, not the saved page.

| Key | Meaning |
|---|---|
| `standard[]` | Named office sizes in portrait millimetres: A4 210×297, A5 148×210, A6 105×148, Letter 215.9×279.4, Legal 215.9×355.6. Choosing one stores `page_format` and ignores any custom width |
| `thermal_presets[]` | Suggestions for a custom sheet: 100×100 (the Zebra badge), 100×150, 100×70, 58×40. Choosing one stores `custom` plus those sides |
| `custom.min_mm`, `custom.max_mm` | 10 and 1000. Both sides of a custom sheet must sit in this range |
| `orientations` | `portrait`, `landscape` |
| `default_new_template` | `custom` 100×100 |

Margins are not in this payload. They default to 0 and are edited in the toolbar. A margin that eats the whole sheet is rejected (`left + right >= width`, or the same for top and bottom).

Cloud templates are created as A4 with 2 mm margins so older output stays the same.

## `demoContext()`

`GET /api/print_template/demo_context?ticket_type=<id>`. Built by `demo_payload()` in `demo.py` from that ticket type's Formly schema. The editor calls it after the template loads, and also loads the schema itself for the “Фамилия Имя” composites.

The shape matches a real print context (`printer.build_context`) with invented answers, so the canvas, a PDF preview, and a test print show the same guest.

| Key | Meaning |
|---|---|
| `fields[]` | One leaf question: `key` (`field_<id>`), `label`, `kind`, `section`, `sample`. This is the palette. `kind` is `text`, `textarea`, `choice`, `multichoice`, `boolean`, `phone`, `email`, or `date`. `multichoice` samples are a list and the expression gets `\|join` |
| `ticket.number` | `A-00042`. Palette item `ticket.number` |
| `ticket.code` | EAN-13 of `200000000000`, including the check digit (`2000000000008`). Palette item `ticket.ticket_code.code` |
| `ticket.ticket_type_name` | The cached ticket type name |
| `form_data` | `{ field_<id>: sample }`. The canvas looks up `data-var="form_data.field_1"` here |
| `has_vcard` | True when the ticket type has a `vcard_template`. The vCard palette item is disabled when this is false |
| `codes` | Data URIs for `barcode`, `qrcode`, `vcard_qrcode` and the `_svg` variants. The canvas puts these on `<img>` while the file keeps `{{ qrcode_svg }}` |
| `geometry.ean13_modules` | 117 |
| `geometry.qr_modules` | Module count of the demo QR, including the quiet border |

Samples are guessed from the Russian label (фамилия → Петрова, имя → Анна, компания → ООО «Ромашка-Экспо», and similar). A choice field uses its first option.

Print-time context also has `files` (manifest names → `file://` URIs inside the template asset directory), `event_data` (empty), and `APP_URL` (empty). `demoContext()` does not send `files`. The editor loads those with `getAsset`.

## Assets

A template file is a picture or a font that belongs to the layout, not to a guest. The registration answers do not supply it.

| Call | What it does |
|---|---|
| `uploadAsset(id, name, file)` | `POST print_template/<id>/assets`, multipart `name` and `file`. Stores `files/<name><ext>` and sets `manifest[name]`. Reply includes the new manifest. The left panel lists the name and the palette can insert it |
| `getAsset(id, name)` | `GET print_template/<id>/assets/<name>`. Raw bytes. The editor makes an object URL so the canvas can show `{{ files.name }}` without rendering Django |
| `deleteAsset(id, name)` | `DELETE` the same path. Drops the file and the manifest key. Reply is the new manifest |

Allowed extensions: `.png`, `.jpg`, `.jpeg`, `.gif`, `.svg`, `.webp`, `.ttf`, `.otf`, `.woff`, `.woff2`, `.css`. Name is letters, digits, `.`, `-`, `_`, up to 100 characters. Max 10 MB. The HTML never contains the disk path. It contains `{{ files.<name> }}`, and the renderer resolves that from the manifest at print time.

Uploading a `.ttf` does not by itself change `font-family`. The template CSS has to add an `@font-face` that points at `{{ files.that_font }}`. Code mode can write that. Visual mode blocks a document that contains `<link>`.
