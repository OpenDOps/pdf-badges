# React editor

Frontend for setting up the PDF template structure. It edits the page document from [document-schema.md](document-schema.md): the rectangles, the text they carry, and the layout the writer fills. It loads JSON or YAML, draws one page, and lets the user select text boxes and image placements, move them, and scale them by corners and edge midpoints.

It does not parse PDF and it does not embed fonts. Upload calls the Rust extractor. Export PDF calls the writer in [json-to-pdf.md](json-to-pdf.md).

## Stack

- React, TypeScript, Vite
- The document in memory is the target schema (`schema_version: 1`)
- YAML via the same field names as JSON (`yaml` package). The file on disk may be either encoding.
- No PDF engine in the browser for the first version. The page on screen is the box list.

A missing `schema_version` is refused with a message that the file is the current extractor tree and needs the text-unification work first. The editor does not try to flip coordinates itself.

## Layout

```text
┌──────────────────────────────────────────────┐
│ toolbar: open, save, page, zoom, export PDF  │
├─────────────┬────────────────────────────────┤
│ box list    │  page surface                  │
│ by id       │  images and text boxes         │
│             │  selection and handles         │
└─────────────┴────────────────────────────────┘
```

The page surface is a div whose size is the trim (`page_size.width` and `height`) in points times the zoom (1 zoom unit = 1 CSS pixel per point, then multiplied by the zoom factor). `bleeds` is drawn outside that rectangle and is not part of the trim size. A box may sit in the bleed. Save writes `bleeds` back unchanged unless the user edits it. Boxes are absolutely positioned children. Images are `<img>` elements pointed at the resource `extracted_path`. Text is rendered with CSS using the box default font and `white-space: pre-wrap`, plus a small renderer for the markup subset so `<b>`, `<i>`, `<u>`, `<font>`, and `<span style>` match the writer. The list shows ids and a one-line preview of `content` or `preentered`.

Paint order follows `contents` array order (later on top). The selected box gets a higher z-index only while selected.

## Selection

Hit test in reverse paint order. A pointer-down on a box selects it and starts a move, unless the pointer is on a handle. Pointer-down on empty page clears the selection. The list selection and the canvas selection are the same `selectedId`.

One selection in the first version. Multi-select can wait.

Handles, in page points, drawn outside the box’s content so they stay clickable:

| Handle | Position |
|---|---|
| `nw` `ne` `se` `sw` | four corners |
| `n` `s` `e` `w` | four edge midpoints |

Handle size is about 8 px in screen space (converted back to points using zoom) so hit targets stay usable when zoomed out.

## Move

Dragging the box body (not a handle) adds the pointer delta, converted from screen pixels to points, to `posX` and `posY`. The drag stores the pointer start and the box start, and applies `current - start` so updates do not accumulate error.

The box may be dragged partly off the page. The writer will clip to the page; the editor shows the overflow.

## Scale

Dragging a handle changes the rectangle. Anchor is the opposite corner or the opposite edge. Deltas are in points.

| Handle | Changes |
|---|---|
| `e` / `w` | `width`, and `posX` when dragging `w` |
| `n` / `s` | `height`, and `posY` when dragging `n` |
| corners | `width` and `height`, and `posX` / `posY` when the dragged corner is on the top or left |

Minimum width and height: 8 pt. A drag that would cross the anchor stops at the minimum instead of flipping the box.

Images and text use the same rectangle math.

- **Image.** The rectangle is the drawn size. Default drag does not lock aspect ratio. Holding Shift locks to the aspect ratio at drag start.
- **Text.** `width` and `height` are the frame. Preview text wraps to the content width (`width` minus `padding.left` and `padding.right`) and sits using `alignment.horizontal` (`left` or `right`) and `alignment.vertical` (`top` or `bottom`). `e` and `w` reflow the preview. Corner drags change the frame the same way. They do not change `font_size` in the first version, so style and the automatic tool’s text stay stable while the user is only laying out columns. A later mode can map a uniform corner scale onto `font_size` if layout-only scale is not enough.

`preentered` and `content` are not modified by move or scale.

## State

```text
document     the schema tree (source of truth)
pageIndex    which page is shown
zoom         number
selectedId   string or null
drag         null or { id, handle, startPointer, startRect }
```

Updates during a drag write into `document` (or into a draft rect committed on pointer-up). Undo is a stack of document snapshots, capped, taken on pointer-up and on fill, not on every move event.

Save writes pretty JSON or YAML with the same tree. Ids are unchanged.

## Upload and export

The webview is a client of two Rust commands:

| Action | Call | Result |
|---|---|---|
| Open PDF | extractor on the upload | a `schema_version: 1` document plus resource files |
| Export PDF | writer on the current document | PDF bytes |

Until those are served over HTTP, both can be files the user drops: a `.yaml` beside its image folder. The editor’s contract is the document, not the transport.

## Markup on screen

The preview renderer implements the same subset as [document-schema.md](document-schema.md). It displays `content` when that field is a string, otherwise `preentered`. It does not offer a rich-text caret in the first version. Filling text is the automatic tool’s job, by id, outside the pointer interactions above.

## First slice

1. Load a target-schema YAML, draw page size, images, and text boxes.
2. Select from the canvas or the list.
3. Move, and scale from the eight handles, with the rules above.
4. Save YAML.

Zoom, undo, PDF upload, and PDF export sit on that slice once move and scale round-trip through the file.
