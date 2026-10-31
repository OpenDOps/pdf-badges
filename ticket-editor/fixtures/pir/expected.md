# Ticket type 1548, `ticket_visitor_pirexpo2026`

Unpacked from `source.zip`. Cloud archive: no `format_version`, no `page`, no `editor.json`.

| Fact | Value |
|---|---|
| Sheet | 210 × 297 mm, from `body` style `width: 210mm; height:297mm` |
| Format | A4 portrait. The archive has no page record, so the margins are 2 mm on every side |
| Name | `ticket_visitor_pirexpo2026` |
| Absolute boxes | 4 |
| Font files | none. No `.ttf`, `.otf`, `.woff`, or `.woff2` |
| Django tags | `{% load static %}` only |

`template.html` has no `<style>` block. Every length is an inline `style`. There are no `tpl-text`, `tpl-var`, `tpl-code`, or `tpl-file` classes.

## Files

| Manifest key | Path |
|---|---|
| `ticket_visitor_pirexpo2026` | `files/56_a4e54ef7-ce30-45b3-8352-4e92aa7c6ab7.jpg` |

## Absolute boxes

Body order. None of these set `height` or `font-size`.

| # | Element | left | top | width | height | Text or src | Other style |
|---|---|---|---|---|---|---|---|
| 1 | `b` | 110mm | 100mm | 100mm | — | `{{ form_data.field_108 }} {{ form_data.field_107 }}` | `display: inline-block`, `text-align: center` |
| 2 | `b` | 110mm | 107mm | 100mm | — | `{{ form_data.field_118 }}` | `display: inline-block`, `text-align: center` |
| 3 | `img` | 140mm | 122mm | 40mm | — | `{{ barcode }}` | — |
| 4 | `img` | 10mm | 5mm | 31mm | — | `{{ vcard_qrcode }}` | — |

`src` values are unquoted.

## Sheet image

One image is not absolutely positioned. Its size is the body, 210×297 mm, and its `src` is `{{ files.ticket_visitor_pirexpo2026 }}`. It is the sheet artwork. The converter writes it as the first `contents` entry, `p1-i1`, at `0, 0`, with the page width and height. The four boxes keep their HTML `left`, `top`, and `width` and paint on top of it. Margins stay in `meta.yml` and do not move or resize a box.

## Skipped

Copied from the converter report. `display` is not a page field.

| Report line | Why it is kept |
|---|---|
| `b display` | `display: inline-block` on both name lines |

`font-size` and `height` were missing and filled (12 pt, and heights 7, 5.08, 15, and 31). They are not skips. A skipped `left` or `font-size` would be a converter failure. None were skipped.
