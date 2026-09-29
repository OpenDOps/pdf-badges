# Product design

This is a webview PDF design tool. A user uploads a PDF. The tool turns it into a JSON/YAML document of pages, text boxes, and images. The user moves and scales those elements. An automatic tool can fill text boxes by id. The same document is rendered back to a PDF.

The Rust CLI is the extract half of that loop. It is not the product.

```text
upload PDF
    │
    ▼
extractor (today's rust-reg, extended)
    │
    ▼
JSON / YAML document          ◄──── automatic fill by text-box id
    │
    ├──► React editor: select, move, scale
    │
    ▼
PDF writer
    │
    ▼
PDF
```

## What the user does

1. Upload a PDF in the webview.
2. Receive a document: one record per page, with text boxes and image placements, plus the font and image resources those boxes use.
3. See the page. Select a text box or an image. Drag it to move it. Drag a corner or an edge midpoint to scale it.
4. Leave text as extracted, or let an external tool replace the text of a box addressed by id. Markup inside the string carries font name, size, and style. The box stays one box; style does not split it into more boxes.
5. Export the document as JSON or YAML, and export a PDF built from that document.

Editing text by hand in the webview can come later. The first editor interaction is layout: selection, move, and scale.

## Pieces

| Piece | Doc | Responsibility |
|---|---|---|
| Extractor | [current-design.md](current-design.md), [inventarization-plan.md](inventarization-plan.md) | PDF to document. Next work is text decode, geometry, style, then merging aligned lines into wrapping boxes. |
| Document | [document-schema.md](document-schema.md) | The tree both sides read and write. Text boxes have an id, pre-entered text, and optional replacement text. |
| Editor | [react-editor.md](react-editor.md) | Frontend for the template structure. Renders one page, hit-tests boxes, moves and scales them, writes the document back. |
| PDF writer | [json-to-pdf.md](json-to-pdf.md) | Document to PDF. Box rectangles are the source of truth, not the original content-stream matrices. |

The extractor and the writer should stay in Rust and share the serde types in `data_structures.rs`. The editor is a separate React app. They meet only through the document file (and, when the webview is hosted, a thin upload/render API that runs those two Rust commands).

## Coordinates

The document uses one system so the editor and the writer do not each invent a flip:

- origin at the top-left of the trim box
- x to the right, y down
- units are PDF points (1/72 inch)
- `page_size` is the trim; `bleeds` on each page is the extra past the trim, read from the PDF and written back to it

Today’s files do not do this. Text `posY` is PDF user space (y up). Images carry both a raw `posY` and a flipped `posY_Pdf`. The extractor change that fixes text geometry also switches emission to top-left. Until that lands, the editor must not assume the two box kinds share an origin.

## Text boxes

A text box is a column, not a single PDF text-showing operator.

Lines that sit on the same alignment edge (same left, or same right) and follow one another down the page become one box. The box has horizontal alignment (`left`, `center`, or `right`), vertical alignment (`top`, `middle`, or `bottom`), and padding on each side. Text wraps inside the frame after padding. The box names a font and has leading and line-height. A span can override those, and so can a `<leading>` tag.

Each text box has:

- `id`, stable for the life of the document, so an automatic tool can target it
- `preentered`, the text extracted from the PDF
- `content`, text supplied later; when it is absent the renderer uses `preentered`

## Images

An image placement is a rectangle on the page plus a reference to an image resource (the extracted file). Moving and scaling in the editor update that rectangle. The writer draws the resource into the rectangle. The original `cm` matrix is diagnostic and is not reapplied after the user has moved the box.

## Out of scope for the first loop

- Drawing new shapes, paths, or annotations
- Rebuilding the original content stream operator by operator
- Collaborative editing
- A general PDF renderer (the on-screen page is the document’s boxes, not a second full PDF engine)
