# Document schema

JSON and YAML for one design document. The extractor writes it, the editor moves boxes in it, an automatic tool fills text by id, and the PDF writer reads it. Both encodings are the same serde tree (`PdfAnalysis` in `src/modules/data_structures.rs`).

Two shapes are specified:

- **Current** is what `rust-reg` emits today.
- **Target** is the tree the editor, the fill tool, and the writer share. It adds ids, pre-entered text, replacement text, inline markup, and one coordinate system. Field names below are the ones to implement; the current names stay where they already match.

## Current tree

```text
pages[]
  page_number          1-based
  page_id              debug string of the lopdf object id, e.g. "(2, 0)"
  page_size
    width, height      points, from MediaBox only
    units              "points" or "points (UserUnit: …)"
    media_box          [llx, lly, urx, ury] in PDF user space, or null
                       no bleeds field; TrimBox and BleedBox are not read
  resources
    images             map keyed by XObject name
    fonts              map keyed by font resource name
  contents[]           untagged text object or image object
metadata
  page_count, title, author, creator, producer, creation_date, modification_date
```

A content object is text when it has a `text` field, and an image when it has an `image` field.

### Current text object

```yaml
posX: 566.9219          # last Tm x, reused for every line in the stream
posY: 430.0             # PDF text-matrix Y, origin at the bottom of the page
width: 278.40002        # character_count * font_size * 0.6
height: 16.0            # font size
text:
  resource_name: F6
  font_size: 16.0
  text_lines:
    - y_position: 430.0
      x_position: 566.9219
      text: Сергей Курдюков
      font_size: 16.0
      line_height: 19.0
  line_height: 19.0
  line_height_ratio: 1.1875
  font_properties:
    is_italic: false
    is_bold: false
    is_underlined: false
    font_family: null
  alignment: Right       # Left | Center | Right | Justified
```

There is one object per extracted line. There is no id. Style flags are present and always default.

### Current image object

```yaml
posX: 11.102654
posY: 832.66907          # raw matrix translation (PDF space, y up)
posY_Pdf: 10.2109375     # pageHeight - posY
width: 580.876
height: 821.56647
image:
  resource_name: X4
  transformation_matrix: [580.876, 0.0, 0.0, -821.56647, 11.102654, 832.66907]
```

### Current resources

Image resource: `name`, `width`, `height` (pixels), `bits_per_component`, `color_space`, `filter`, `file_format`, `extracted_path`, `file_size_bytes`, `description`.

Font resource: `name`, `base_font`, `font_type`, `encoding`, `to_unicode` (raw CMap text or null), `extracted_path`, `description`.

`contents` order is the order the extractor emitted (all text boxes, then all image boxes), not guaranteed paint order.

## Target tree

Same top level: `metadata`, `pages[]`, per page `page_size`, `resources`, `contents`.

Coordinate system for every box:

- origin at the top-left of the trim box
- x right, y down
- unit is the point
- `page_size.width` and `page_size.height` are the trim size
- `page_size.bleeds` is how far the bleed extends past the trim on each side

### Page size and bleeds

```yaml
page_size:
  width: 595.28             # trim width, points
  height: 841.89            # trim height, points
  units: points
  bleeds:
    top: 8.5
    right: 8.5
    bottom: 8.5
    left: 8.5
```

`bleeds` is required on every page. Each side is a length in `page_size.units`, `>= 0`. `units` is `points` or `mm`. Asymmetric sides are valid. Zero on every side means the PDF had no bleed. The writer converts millimetres with `pt = mm * 72 / 25.4` before writing the PDF boxes. The extractor writes `points`, because PDF boxes are already in points.

The extractor reads the page boxes, walking parent `Pages` nodes when a box is missing on the page, then applies `UserUnit` (default 1):

| PDF box | If absent |
|---|---|
| `MediaBox` | required; stop if it cannot be resolved |
| `CropBox` | `MediaBox` |
| `BleedBox` | `CropBox` |
| `TrimBox` | `CropBox` |

With trim `[llx, lly, urx, ury]` and bleed `[bllx, blly, burx, bury]` in PDF user space (y up):

```text
width          = urx - llx
height         = ury - lly
bleeds.left    = llx - bllx
bleeds.bottom  = lly - blly
bleeds.right   = burx - urx
bleeds.top     = bury - ury
```

A file that only has a `MediaBox` (the current `print.pdf` case) resolves trim and bleed to that same rectangle, so every bleed side is `0` and `page_size` matches today’s media size.

Content coordinates are relative to the trim’s top-left, not the media box. A mark in the left bleed has a negative `posX`. Changing `bleeds` does not move existing boxes.

The writer maps the field back onto the page dictionary. In PDF user space with the origin at the media lower-left:

```text
MediaBox = BleedBox = [0, 0, left + width + right, bottom + height + top]
TrimBox            = [left, bottom, left + width, bottom + height]
```

There is no separate slug area: the media box is the bleed box. `CropBox` is written equal to `MediaBox`.

`media_box` is not part of the target page. The four bleed sides and the trim size are enough to rebuild the PDF boxes.

`contents` is paint order: later entries draw above earlier ones.

### Identity

Every text box and every image placement has `id`. The extractor assigns `p{page}-t{n}` and `p{page}-i{n}` in reading order. The editor preserves ids when the user moves or scales a box. New boxes created in the editor get a new id; ids are not reused.

The automatic tool addresses a text box only by `id`. It does not match on coordinates or on the extracted string.

### Target text box

```yaml
id: p1-t1
posX: 10
posY: 30
width: 70
height: 50
alignment:
  horizontal: center        # left | center | right
  vertical: middle          # top | middle | bottom
padding:
  top: 0
  right: 0
  bottom: 0
  left: 0
auto_scale: true            # card: shrink font sizes so the long text fits this rectangle
template: true              # scan this box for fields; absent means false
delimiters:                 # only with template: true; absent means {{ and }}
  open: "{{"
  close: "}}"
text:
  font: font1               # key in resources.fonts
  font_size: 5              # in page_size.units
  leading: 0.5              # extra gap after each line, same unit
  line_height: 6            # line box height, same unit
  font_style:
    weight: normal          # normal | bold
    italic: false
    underline: false
  preentered: '<span font="font1">{{name}}<br>{{surname}}</span><br><br><span font="font2"><bold>{{company name}}</bold></span>'
  content: null             # replacement; null or absent means render preentered
```

Lengths on the page (`posX`, `posY`, `width`, `height`, `padding`, `font_size`, `leading`, `line_height`) use `page_size.units`: `points` or `mm`. The writer converts `mm` to points with `pt = mm * 72 / 25.4` before building the PDF.

`alignment`, `padding`, `auto_scale`, `template`, and `delimiters` sit on the box, next to the rectangle. They are not inside `text`. The current single value `text.alignment` (`Left`, `Center`, `Right`, `Justified`) is replaced by the two-axis object.

| Field | Meaning |
|---|---|
| `posX`, `posY` | Top-left of the frame |
| `width`, `height` | Frame size. The editor can change both. The writer clips to the frame. |
| `alignment.horizontal` | `left`, `center`, or `right`. Each line sits on that side of the content box. `center` places the line in the middle of the content width. |
| `alignment.vertical` | `top`, `middle`, or `bottom`. The whole text block sits on that side of the content box. `middle` centers the block in the content height. |
| `padding` | Inset on each side, in page units. All four are required. Values are `>= 0`. |
| `font` | Default font. A key in `resources.fonts`, not a family name. |
| `font_size` | Default size, in page units. |
| `line_height` | Height of one line box, in page units. |
| `leading` | Extra space after each line, in page units. The distance from one baseline to the next is `line_height + leading`. |
| `font_style` | Default weight, italic, and underline for text that is not inside a styled tag. |
| `preentered` | Text extracted from the PDF, or the authored string. Kept so a fill can be reverted. |
| `content` | Text written by the automatic tool or a later editor. Render `content` when it is a string, including an empty string. Render `preentered` when `content` is null or absent. |
| `auto_scale` | Optional, on the rectangle, not inside `text`. Absent or `false` leaves every font size as authored. `true` asks the writer to shrink font sizes so the text bounds fit this box. The card box sets it `true` and uses long paragraphs so the authored size overflows. The scale rules are [Fit text to the rectangle](json-to-pdf.md#fit-text-to-the-rectangle). |
| `template` | Optional. Absent or `false` copies the box as written. `true` scans the string the writer paints (`content` when it is a string, otherwise `preentered`) for fields wrapped by `delimiters`. Only a text box may set it. |
| `delimiters` | Optional. Used when `template` is true. `open` and `close` are the strings that wrap a field name. Absent means `{{` and `}}`. When the key is present, both sides are required non-empty strings. They are matched as written. One box has one pair. Two boxes on a page may use different pairs. A `delimiters` key without `template: true`, or on an image placement, is invalid. |

The content box is the frame inset by padding:

```text
contentX      = posX + padding.left
contentY      = posY + padding.top
contentWidth  = width  - padding.left - padding.right
contentHeight = height - padding.top  - padding.bottom
```

Wrap width is `contentWidth`. `horizontal: left` starts each line at `contentX`. `horizontal: right` ends each line at `contentX + contentWidth`. `horizontal: center` sets the line x to `contentX + (contentWidth - lineWidth) / 2`. `vertical: top` places the first line at `contentY`. `vertical: bottom` places the last line so the block ends at `contentY + contentHeight`. `vertical: middle` sets the block’s top to `contentY + (contentHeight - blockHeight) / 2`.

If `padding.left + padding.right` is greater than `width`, or `padding.top + padding.bottom` is greater than `height`, the document is invalid.

The extractor sets `horizontal` from the shared edge (`left` or `right`), `vertical` to `top`, and every padding side to `0`. The editor and the automatic tool may change them. A taller frame with `vertical: bottom` keeps the text on the bottom when the string is shorter than the frame.

`text_lines` is not part of the target box. Per-line positions are an extractor intermediate. After unification, the box and the string are the model. A multi-line extract becomes either spaces (soft wrap) or `<br>` (hard break) inside the one string. Other whitespace wraps to `contentWidth`.

`font` is the default face. A span's `font` attribute overrides it for the text inside that span. `font_family` on the resource is how that face is described; the box does not store the family itself.

### Inline markup

`preentered` and `content` are strings in a small HTML-like subset. The subset is the same in both fields. Tags mark runs inside the box. They do not create boxes.

A run inherits `font`, `font_size`, `leading`, `line_height`, and `font_style` from the box, then from the surrounding span. The baseline step for a line is `line_height + leading` of the run that ends the line.

Allowed:

| Form | Effect |
|---|---|
| plain text | Current font, size, leading, line-height, and style |
| `<br>` | Forced line break. `<br><br>` is a break, an empty line, and another break. |
| `<b>…</b>`, `<bold>…</bold>` | Bold. Same effect. |
| `<i>…</i>` | Italic |
| `<u>…</u>` | Underline |
| `<span font="font1" leading="0.5" line-height="6">…</span>` | Overrides font and, when present, leading and line-height, for the text inside the span. `font` is a key in `resources.fonts`, written as `font="font1"`. `leading` and `line-height` are numbers in page units. Any of the three attributes may be omitted. `</span>` only closes the span. |
| `<leading>1</leading>` | Sets `leading` for the following text until the next `<leading>` or the end of the current span. Does not change font or line-height. The body is a number in page units. |
| `<span style="…">…</span>` | Restricted style, below. May be combined with `font`, `leading`, and `line-height`. |

Tags may nest. Unknown tags are rejected by the writer (it does not strip them silently). `<`, `>`, and `&` in text are escaped as `&lt;`, `&gt;`, `&amp;`. `{{name}}` is literal text, not a substitution.

`span` style is a semicolon-separated list. Only these properties are valid:

| Property | Values |
|---|---|
| `font-size` | a number in page units |
| `font-weight` | `normal` or `bold` |
| `font-style` | `normal` or `italic` |
| `text-decoration` | `none` or `underline` |

The v0 card string is one box with two faces and a bold company line:

```text
<span font="font1">{{name}}<br>{{surname}}</span><br><br><span font="font2"><bold>{{company name}}</bold></span>
```

`{{name}}` and `{{surname}}` are inside the `font1` span. `{{company name}}` is inside the `font2` span, and `bold` marks that run bold. Two breaks between the spans make one empty line. That line uses the box font.

There is no CommonMark (`**bold**` is literal asterisks).

### Markup tags and styles

This is the whole set the writer accepts. The writer parses these tags when it draws a text box.

Box styles are fields on the text box, not tags. A run inherits them, then a span or a leading tag can override the ones named below.

```text
Box styles
  font                  resource key in resources.fonts
  font_size             number, page units
  leading               extra gap after a line, page units
  line_height           line box height, page units
  font_style.weight     normal | bold
  font_style.italic     true | false
  font_style.underline  true | false

Layout fields, not markup
  alignment.horizontal  left | center | right
  alignment.vertical    top | middle | bottom
  padding               top, right, bottom, left

Tags
  br
      Forced line break. Two br tags in a row are a break, an empty line, and a break.
  b, bold
      Bold until the matching closer. Same effect. Does not change the font key.
  i
      Italic until the matching closer.
  u
      Underline until the matching closer.
  span font="font1" leading="0.5" line-height="6"
      Overrides font, and leading or line-height when that attribute is present,
      for the text inside the span. A missing attribute inherits.
      font is a key in resources.fonts, written as font="font1".
      leading and line-height are numbers in page units.
      The closer only ends the span. A font key on the closer, or a bare token
      such as font1 with no attribute name, is an error.
  leading
      Body is a number in page units. Sets leading until the next leading tag
      or the end of the current span. Does not change font or line-height.
  span style="..."
      Restricted style, below. May be combined with font, leading, and line-height.

span style properties (semicolon-separated; nothing else is valid)
  font-size         number, page units
  font-weight       normal | bold
  font-style        normal | italic
  text-decoration   none | underline

Text
  {{name}} stays those characters. It is not substituted.
  < > & in text are written as the entities lt, gt, and amp.
  Any other tag, or a leading body that is not a number, is an error naming the box id.
```

### Target image placement

```yaml
id: p1-i1
posX: 11.1
posY: 10.2
width: 580.9
height: 821.6
image:
  resource_name: X4
```

The rectangle is the source of truth after any edit. `transformation_matrix` is optional and diagnostic. The writer ignores it.

Moving a placement changes `posX` and `posY`. Scaling changes `width` and `height`, and the opposite edges when the dragged handle is not the bottom-right.

### Resources (target)

Unchanged role: the page owns the bytes and the font identities; placements and text boxes point at them by name.

Image `extracted_path` is relative to the document file’s directory (the `-o` folder), not a hardcoded `extracted/` at the cwd.

Font resources gain structured fields used by markup resolution:

```yaml
F6:
  name: F6
  base_font: AAAAAA+LiberationSerif-Bold
  font_family: LiberationSerif
  font_type: Type0
  encoding: Identity-H
  weight: bold
  italic: false
  embedded: true
  source_path: fonts/LiberationSerif-bold.ttf
```

`source_path` is the embedded font program, relative to the analysis file. The extractor writes that file from `FontFile2`, `FontFile3`, or `FontFile`. `embedded: false` and `source_path: null` mean the PDF did not contain a font file. The writer maps `font_name` plus weight and italic to a resource, then embeds `source_path`. Subset prefixes are not part of `font_family`. The raw `to_unicode` CMap is not part of the target file.

## Who writes which fields

| Actor | Writes |
|---|---|
| Extractor | Full document. `preentered` set, `content` null, ids assigned, `alignment.horizontal` from the shared edge, `alignment.vertical: top`, padding all `0`, `page_size.bleeds` from the PDF trim and bleed boxes, markup only where a run differs from the box default. Leaves `template` and `delimiters` absent. |
| Editor | `posX`, `posY`, `width`, `height` of text boxes and image placements. May also change `alignment`, `padding`, `page_size.bleeds`, and, on a text box, `template` and `delimiters`. Does not rewrite `preentered`. |
| Automatic tool | `content` on a text box chosen by `id`. May use the same markup. May change `alignment` and `padding`. |
| Writer | Nothing. Reads the tree and produces a PDF. |

## Version

Add `schema_version: 1` at the root when the target fields ship. The current files have no version. A reader treats a missing `schema_version` as the current tree (bottom-left text Y, no ids, per-line text boxes).
