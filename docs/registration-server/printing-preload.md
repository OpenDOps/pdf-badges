# Printing preload

What is ready when the event opens, and what a print still draws for one visitor. Step 11 of [printing-implementation-plan.md](printing-implementation-plan.md) builds this memory. The two renderers and the CUPS job are [printing.md](printing.md).

`ticket-render` is the old badge: `badge.cfg` and `bg.png`. `struct_to_pdf` is the YAML page, drawn as PDF, PNG, BMP, or ZPL. A category with a page file uses the page. A certificate uses the layout. A print reads the memory from step 11. It does not decode `badge.cfg`, parse a page file, or open a font or an image.

## When the memory is built

`switch_to` and the startup `open_bound` build one map for every `badge/` directory of the event that is open after the call. The key is the category, or that category plus 1000 for a certificate. The previous event's map is dropped after the next map is complete. A switch to the id already open leaves the map. `clear` drops it.

A sync store of `badge.cfg` and `bg.png` for the open event replaces that category's layout entry. A sync store of a page file replaces that category's page entry. The other categories stay. A store for an event that is not open leaves the map unchanged.

A file that fails to decode is kept as that error and left out of the drawable entries. The rest of the event still opens.

## Preload

These pieces are the same for every visitor of a category. They are built at the moments above.

| Piece | Old `badge.cfg` | YAML page |
|---|---|---|
| Document | The layout from step 3, with `layout.json` written beside the cfg | The `Page` and the `TemplateIndex` |
| Fonts | The face the layout names, `.otf` or `.ttf`, parsed once. The file name is [Fonts](printing.md#fonts) | The font bytes and the parsed face. `draw_text` today reads the file for every box |
| Images | `bg.png` decoded once | JPEG bytes kept as stored. BMP decoded to samples once. `paint_contents` today reads and decodes them on every page |

`render_rows` still fills the holes and builds a PDF. It takes the font bytes and the image samples from this memory.

## What is drawn ahead of the visitor

A piece is drawn ahead when it has no field hole, no `ifSt`, and it is not a barcode, a QR code, or a photo. Static text is in that layer. `auto_scale` on a string that is already known is in that layer too. A hole stays out: the line breaks and the scale follow the visitor's text.

On a YAML page the condition is `ifSt`, the same key the old area uses. An absent or empty `ifSt` draws the entry. Any other value is a row key, and the entry is skipped when that value is missing, empty, `false`, or `0`. A barcode entry has type `ean13`, `code128`, or `qr`. A photo entry reads `photo.field` from the row. The fields are [document-schema.md](../pdf-tooling/document-schema.md). They are drawn in steps 8 through 10. An `ifSt`, a barcode, a QR code, and a photo stay out of the static layer: the payload is the visitor's.

### PDF

The YAML page only. Images and static text become one PDF form when the page is prepared. A print fills the holes, lays out those boxes, and appends their operators. The font face stays in memory. The glyph subset for the name is built for that job from the cached face, because the letters depend on the visitor.

### PNG and ZPL

Both renderers. The static layer is built at 203 dpi and at 300 dpi, the two densities the queues use. A print copies that buffer and draws the fields, the barcode, the QR code, and the photo into their boxes.

For ZPL the static layer is already 1 bit. Only the boxes that change are thresholded into it. The job still sends one full `~DG`. The saved work is the decode, the scale, and the full-page threshold.

A long-side cap that asks for another dpi builds that buffer on the first print and keeps it on the entry.

## A page that is only fields

`scripts/badges/tadviser.yaml` has no images. Both boxes are templates with `auto_scale`: `{{surname}}` and `{{name}}` in one, `{{company}}` in the other. The static bitmap of that page is empty. The saving is the preloaded fonts and the template index.

## Memory

A 90 by 130 mm page at 300 dpi is about 1063 by 1535 pixels. An RGBA buffer is about 6.5 MB. The 1-bit ZPL layer is about 0.2 MB. The entry keeps RGBA for a raster draw and 1 bit for a ZPL draw, at 203 and at 300.

## The second page

A transliterated copy and a double print use the same static layer. The dynamic pass runs again with the other field map. The background is not scaled a second time.

## What a print still draws

- Field holes, including every `auto_scale` box whose string comes from the visitor.
- An area whose condition depends on the visitor.
- A barcode, a QR code, and a photo.
- The PDF operators for those boxes, and the glyph subset for that name.
- The `^XA` wrapper and the seven Zebra settings, which are the printer's, added when the job is built.
