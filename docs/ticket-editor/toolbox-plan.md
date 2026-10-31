# Tool rail for the badge canvas

The left panel of `/editor/:id` is a 260px list of survey fields, ticket codes, template files, and **Загрузить файл**. This plan removes that pane. A narrow tool rail takes its place. The operator picks a tool, then clicks or drags on the sheet.

The page object does not change kind. A new box is still one entry in `contents`, appended so it paints above the boxes already there. Save is still the existing PUT. GrapesJS is not given these tools as blocks. The rail lives outside the canvas, and the pointer session runs on the canvas document.

This plan starts after the canvas in [yaml-and-preview-plan.md](yaml-and-preview-plan.md#step-14-canvas-in-the-editor-app). It does not wait for that plan's preview button. Field pipes stay in [implementation-plan.md](implementation-plan.md#step-7-field-pipes-and-ifst).

Nothing from that pane remains beside the rail. Survey fields, **Текстовый блок**, **Коды билета**, **Файлы макета**, and **Загрузить файл** are all gone. A variable is chosen inside the image dialog ([step 10](#step-10-image-from-a-variable)) or while editing text ([step 11](#step-11-variables-in-text)). An image file is chosen in [the source dialog](#step-9-image-source-dialog). Font upload leaves with the pane. A face already stored on the template stays available. Adding a new font is a later step, inserted by [Adding a step](#adding-a-step).

## The rail

The rail is a vertical strip, `3.5rem` wide (56px when the root font is 16px, about 1.5cm on screen). The unit is `rem`, so the strip follows the user's font size. A CSS centimetre is locked to 37.8px and does not.

Each control is an icon button, with a tooltip for the name. The pressed button is the active tool (`aria-pressed`). Icons are Font Awesome. The strip is a dark gray rail: light icons, a gray press for the active tool, and no button border. The tool stays active after a use, so the next gesture draws another box. Escape returns to Select. Keys: `V` Select, `M` Select box, `T` Text, `I` Image, `R` Rectangle, `Q` QR, `B` Barcode, `E` Colorpicker.

The rail is on screen in **Визуально**. In code mode it is not shown. On a read-only template the create tools are disabled. Select, Select box, and Colorpicker stay.

| Tool | Gesture | What is stored |
|---|---|---|
| Select | Click a box, or drag it. Click empty sheet to clear | No new entry. This is the canvas as it works today |
| Select box | Drag, then release | No new entry. Every box fully inside the band becomes the selection. A band with nothing inside clears it. Shift adds to the selection |
| Text | Click | Text entry, 40×10 mm, top-left at the click, `preentered: "Текст"` |
| Image | Drag from one corner to the opposite corner | Image entry of that rectangle, after [the source dialog](#step-9-image-source-dialog) |
| Rectangle | Same corner drag | A text entry with no text, a white fill, and a hairline border. The page has no shape type. Later shapes reuse this frame |
| QR code | Same corner drag | `barcode.type: qr`, `preentered: "{{code}}"`, `template: true`. Shift locks a square |
| Barcode | Same corner drag | `barcode.type: ean13`, `preentered: "{{code}}"`, `template: true` |
| Colorpicker | Click | No new entry. Foreground becomes the text color of the entry under the pointer, or the image pixel under the pointer. [Step 7](#step-7-colorpicker-tool) |

A drag whose width or height is under 2 mm creates nothing. Text does not use that rule: a click is the whole gesture.

Both corners are snapped to 0.5 mm, then normalized, so a drag up or to the left still stores a positive width and height. The conversion is the one the canvas already uses: frame `clientX / (96/25.4)` minus the sheet origin (`bodyOriginMm`). The rubber band is an element in the canvas document, so it zooms and pans with the sheet.

While Select is active, those listeners are off and GrapesJS moves and resizes boxes. Any other tool takes the pointer and does not start a GrapesJS drag. A click on an existing box with a create tool still creates. It does not select.

Every text entry written here includes `alignment.horizontal: left`, `alignment.vertical: top`, and `padding` of 0 on all four sides. The writer requires them. The font is the existing default: face `text`, 12 pt (`4.233` mm), `leading: 0`, `line_height` 1.2 times the size.

## Steps

Each step has a Check. The check uses only what that step added.

Steps 9–14 are the extensions already named. A further extension is a new row inserted immediately before [Run](#step-16-run), with its own heading, a [Steps](#steps) link, and a Check. [Adding a step](#adding-a-step) is the rule. A step that has been started keeps its number.

| Step | What it does | Status |
|---|---|---|
| [1. Draw geometry](#step-1-draw-geometry) | Pure functions: corners, snap, the 2 mm minimum, the square lock, and the inside test. | ✅ Completed |
| [2. Tool rail](#step-2-tool-rail) | The `3.5rem` icon strip replaces the left pane. Select is the default. | ✅ Completed |
| [3. Pointer session](#step-3-pointer-session) | Rubber band on the canvas document. Select leaves GrapesJS in charge. | ✅ Completed |
| [4. Text tool](#step-4-text-tool) | A click appends a 40×10 mm text entry. | ✅ Completed |
| [5. Rectangle tool](#step-5-rectangle-tool) | A corner drag appends an empty text entry with a fill and a border. | ✅ Completed |
| [6. QR and barcode tools](#step-6-qr-and-barcode-tools) | A corner drag appends `qr` or `ean13` with `{{code}}`. | ✅ Completed |
| [7. Colorpicker tool](#step-7-colorpicker-tool) | A click samples an entry or an image pixel and stores that color as the foreground. | ✅ Completed |
| [8. Select box](#step-8-select-box) | The band selects every entry fully inside it. | ✅ Completed |
| [9. Image source dialog](#step-9-image-source-dialog) | After the image drag, a dialog picks an asset or uploads one. | Not started |
| [10. Image from a variable](#step-10-image-from-a-variable) | The same dialog can place a `photo` entry. No file is uploaded. | Not started |
| [11. Variables in text](#step-11-variables-in-text) | Standalone. Insert a variable into the text being edited. | Not started |
| [12. Color pickers](#step-12-color-pickers) | Foreground and background swatches at the bottom of the rail. A click opens a dialog. | ✅ Completed |
| [13. Constraints](#step-13-constraints) | Align selected boxes to each other by writing their positions. | Not started |
| [14. Color picker package](#step-14-color-picker-package) | One source builds a Preact picker or a React picker. React apps do not load Preact. | ✅ Completed |
| [15. Typing history](#step-15-typing-history) | While typing, undo is a character or a block. Leaving the text folds that visit into one event. | Not started |
| [16. Run](#step-16-run) | The rail, each tool, and the extensions, on one sheet. | Not started |

## Checking a step

| What the step added | How to check it, before Run |
|---|---|
| Geometry, hit testing, alignment math | `npm test` in `ticket-editor/frontend`. No browser. |
| The rail, the dialog, or the pickers | The component spec for that control. No canvas server. |
| A box appended from a gesture | The unit test that feeds a rectangle into the page and reads the entry back. |
| The live sheet | [Run](#step-16-run). The dev server is `ticket-editor/frontend` on port 4213. |

## Adding a step

A new idea is a new step. Insert the row in the table above, directly above [Run](#step-16-run). Give Run the next number, in the table and in its heading, in that same edit. Do not renumber a step whose work has started.

The new section follows the same shape as the others. The first line under the heading is a link back to [Steps](#steps). The last part is `### Check`, and the check uses only what that step added. If the idea needs a page field the writer does not have, the step says so and stores the result in a field the writer already accepts.

## Step 1. Draw geometry

[Steps](#steps)

✅ Completed. `draw-tool.ts` snaps both corners to 0.5 mm, rejects a side under 2 mm, locks a square from the anchor, and returns the ids fully inside a band. `draw-tool.spec.ts` covers the checks below.

`ticket-editor/frontend/src/app/operator/templates/draw-tool.ts` holds the math. No Angular and no GrapesJS.

- `normalizeRect(anchor, pointer)` snaps both points to 0.5 mm and returns `{ posX, posY, width, height }` with positive sides. `posX` and `posY` are the lesser coordinates.
- A side shorter than 2 mm returns null.
- `lockSquare(rect)` is used when Shift is held on the QR tool. The side is the larger of width and height. The anchor corner stays put.
- `boxesInside(band, entries)` returns the ids whose rectangles lie fully inside the band. An entry that only crosses an edge is not inside.

`PX_PER_MM` stays `96 / 25.4`, the same constant the canvas uses.

### Check

`draw-tool.spec.ts` next to the module. A drag from `(10, 8)` to `(4, 20)` stores `posX: 4`, `posY: 8`, `width: 6`, `height: 12`. A 1 mm drag returns null. Shift on a 10×4 drag stores a 10×10 square from the anchor. A 10×10 band contains a box at `(2, 2)` size 4×4 and excludes a box at `(8, 8)` size 4×4.

## Step 2. Tool rail

[Steps](#steps)

✅ Completed. The visual sheet shows the seven-icon rail at `3.5rem`. Select starts pressed. Text presses on click, and Escape returns to Select. Code mode hides the rail. The variable list and **Загрузить файл** are gone.

`tool-rail.component.ts` is a radio group of these seven tools, top to bottom: Select, Select box, Text, Image, Rectangle, QR, Barcode. The eyedropper is [step 7](#step-7-colorpicker-tool). Width `3.5rem`. The active tool is a signal on the template editor, default `select`. The rail emits the next tool. Escape and the keys in [The rail](#the-rail) set it.

The rail is the whole left side. `app-variable-palette` is removed from the editor template, along with the upload block under it. The workspace is the rail and the sheet. There is no second column of fields or files.

### Check

The rail spec sees seven buttons in that order, the strip at `3.5rem`, Select pressed on first paint, a click on Text pressing Text, and Escape returning to Select. The editor template spec has no `app-variable-palette` and no **Загрузить файл**.

## Step 3. Pointer session

[Steps](#steps)

✅ Completed. `pointer-session.ts` listens on the canvas document for every tool except Select. A drag paints a dashed band in millimetres and pointer up returns the rectangle from step 1. Rectangle, Image, QR, and Barcode show a crosshair over the sheet. Text shows the typing pointer. `pointer-session.spec.ts` covers the checks below. The session does not append an entry.

The visual editor takes the active tool. For Select it adds no listeners. For every other tool it listens on `editor.Canvas.getDocument()` for pointer down, move, and up, and stops the event so GrapesJS does not start a drag.

Pointer down records the sheet point. Pointer move shows a dashed rectangle from that point to the pointer, in millimetres, inside the canvas document. Pointer up clears the band and returns the rectangle from [step 1](#step-1-draw-geometry), or null when the drag is under 2 mm. The session does not append an entry. Text, Rectangle, QR, barcode, and Image append one. Colorpicker and Select box do not.

### Check

A session spec drives three points and reads the rectangle. No browser. The listeners are installed only when the tool is not Select.

## Step 4. Text tool

[Steps](#steps)

✅ Completed. A Text click on an existing text box edits that box. A click on empty sheet appends an entry at the snapped point with `preentered` `Текст`, sized from the glyphs at the last font, and the tool stays Text. A second click on empty sheet appends another entry. Neither click changes the zoom. ⌘Z removes the newest entry and ⌘⇧Z puts it back. The file changes only on Save. Double-click with Select, or a Text click on a box, types in that box in its own font. The caret is the clicked glyph. There is no second text window. Enter stores `<br>`, and the frame grows by one line height plus the gap between lines unless `auto_scale` is set. `page-view.spec.ts` covers the snap and the size, and `e2e/text-tool.spec.mjs` covers the sheet.

A click with Text active appends one entry at the click:

- `id` from the existing `p1-t*` sequence
- `posX` is the snapped click. `posY` is that click minus the height, so the bottom-left corner is the click
- `width` and `height` from the glyphs of `Текст` at the last font size, on the 0.5 mm grid
- `preentered: "Текст"`, `content: null`
- `alignment` and `padding` as in [The rail](#the-rail)
- `text` from the last face (font, size, and style). The first face is `text` at 12 pt

The canvas reloads and selects that entry. The tool stays Text. Editing types into that entry, in its font, with the caret on the clicked glyph. Inserting a variable into that text is [step 11](#step-11-variables-in-text), not this step.

### Check

A unit test clicks at `(12.2, 7.4)` and reads `posX: 12`, `posY: 2` (the 5.5 mm frame sits above the snapped click), a frame from the glyphs at 12 pt, and `preentered` `Текст`. A second click appends a second entry. A longer string grows the frame. `auto_scale` leaves it. The stored page is unchanged until Save.

## Step 5. Rectangle tool

[Steps](#steps)

✅ Completed. A Rectangle drag from `(5, 5)` to `(25, 15)` appends one empty text entry, 20×10 mm at `(5, 5)`, with fill `#FFFFFF` and border `{ width: 0.2, color: "#000000" }`. A drag under 2 mm appends nothing. The tool stays Rectangle. ⌘Z removes the newest entry and ⌘⇧Z puts it back. The file changes only on Save. `page-view.spec.ts` covers the drag, `e2e/rect-tool.spec.mjs` covers the sheet, and `load_page` accepts the saved text entry.

A corner drag with Rectangle active appends a text entry for the rectangle from [step 3](#step-3-pointer-session). `preentered` is `""`. `background` is `#FFFFFF`. `border` is `{ width: 0.2, color: "#000000" }`. Alignment, padding, and the default font are the same as [step 4](#step-4-text-tool). A drag under 2 mm appends nothing.

The page has no shape entry. This empty text box is the rectangle. A later shape (the rail can grow a small menu on this button) uses the same frame fields. An ellipse waits until the writer has a place for it.

### Check

A drag from `(5, 5)` to `(25, 15)` appends one entry, 20×10 mm at `(5, 5)`, with that fill and border, and no mustache. The YAML a save writes is a text entry. `load_page` accepts it.

## Step 6. QR and barcode tools

[Steps](#steps)

✅ Completed. A 30×12 mm QR drag stores `type: qr` at that size. The same drag with Shift stores 30×30 from the anchor. A barcode drag stores `type: ean13` and `{{code}}`, and Shift leaves the 30×12 rectangle. A drag under 2 mm appends nothing. The tool stays QR or Barcode. Deleting a code removes that entry from the page, so the next drag does not draw it again. Moving or resizing it is the same undo as creating it. ⌘Z removes the newest change and ⌘⇧Z puts it back, and the box that step changed stays selected. The file changes only on Save. `page-view.spec.ts` covers the drag, `e2e/code-tool.spec.mjs` covers the sheet, and `load_page` accepts the saved barcode entries.

A corner drag appends a barcode entry for the same rectangle.

- QR: `barcode.type: qr`, size from the drag. Shift uses `lockSquare`.
- Barcode: `barcode.type: ean13`, size from the drag. Shift does not lock it.
- Both: `template: true`, `preentered: "{{code}}"`, `content: null`.

The canvas draws the demo QR or EAN-13 picture it already uses. The stored page keeps `template: true` and `{{code}}`. Preview fills the code later, in [yaml step 16](yaml-and-preview-plan.md#step-16-preview-endpoint).

### Check

A 30×12 drag with QR stores `type: qr` and those 30×12 mm. The same drag with Shift stores 30×30 from the anchor. A barcode drag stores `type: ean13` and `{{code}}`, and Shift leaves the 30×12 rectangle. A drag under 2 mm appends nothing.

## Step 7. Colorpicker tool

[Steps](#steps)

✅ Completed. The eyedropper sits under Barcode (`E`, `fa-solid fa-eye-dropper`, `creates: false`). A click samples the topmost box: text / rectangle → `text.color` or `#000000`; image / QR / barcode → the painted pixel. Transparent pixels and empty sheet leave foreground alone. The sample writes `ticket-editor:text-style` foreground only; the entry and the page are unchanged. `sample-color.spec.ts` and `colorpicker.tool.spec.ts` cover the checks below.

An eyedropper is added to the rail under Barcode. The icon is `fa-solid fa-eye-dropper`. The key is `E`. The tool stays active after a sample. Escape returns to Select. A click does not drag a band and does not append an entry.

The click writes `foreground` in `ticket-editor:text-style`, the same field [step 12](#step-12-color-pickers) stores. The foreground swatch shows it. The background swatch stays. The clicked entry is not rewritten, and no other selected entry is rewritten. With nothing selected, the next Text or Rectangle entry is born with that foreground, which [step 4](#step-4-text-tool) and [step 5](#step-5-rectangle-tool) already do. The dialog from step 12 does not open.

The color is whatever is under the pointer.

- The topmost entry that contains the point is the hit. An empty sheet changes nothing.
- A text entry, including a rectangle from step 5, contributes `text.color` when that value is `#RRGGBB`, and `#000000` when the field is absent. The click may land on the fill. The sample is still the text color, not `background`.
- An image, a photo, a QR, or a barcode has no text color. The sample is the pixel of the picture painted in that box. The click is mapped from sheet millimetres into that picture's pixels and read as `#RRGGBB`. Alpha is dropped. A fully transparent pixel, or a photo that has no picture, changes nothing.

### Check

A click on a text entry whose `text.color` is `#112233` stores foreground `#112233` and leaves that entry's `text.color` as it was. A text entry with no color stores `#000000`. A click on an image at a pixel that is `#FF6A00` stores that hex. A transparent pixel and a click on empty sheet leave the previous foreground. The background field is unchanged. The page gains no entry.

## Step 8. Select box

[Steps](#steps)

✅ Completed. Select box uses the same band as [step 3](#step-3-pointer-session). On pointer up, `boxesInside` supplies the ids. The canvas selects those components. Shift unions them with the current selection. No id inside the band clears the selection. Nothing is appended. `marquee.tool.ts` returns `select-entries`; `applyToolEffect` does not record history; `visualEditor.selectEntries` drives Grapes multi-select without collapsing via `entrySelect`.

### Check

The inside-test from [step 1](#step-1-draw-geometry) is the contract. `marquee.tool.spec.ts` feeds three entries and a band and expects the two ids that sit fully inside. The third, which crosses the edge, is absent. Shift unions; an empty band clears.

## Step 9. Image source dialog

[Steps](#steps)

Image uses the corner drag from [step 3](#step-3-pointer-session). Pointer up opens a dialog. The rubber band stays visible behind it. The dialog does not append until the operator picks a source.

The dialog offers the image names already in the template manifest, and an upload. The left pane is gone, so this dialog is the only place a file is added. Upload posts `POST /api/print_template/:id/assets` with a name field in the dialog. Image extensions only: `.png`, `.jpg`, `.jpeg`, `.gif`, `.svg`, `.webp`. A font file is refused here.

Choosing a name, or a finished upload, appends an image entry of the dragged rectangle with `image.resource_name` set to that name. Cancel, or a failed upload, appends nothing and removes the band.

A variable as the picture, with no file, is [step 10](#step-10-image-from-a-variable).

### Check

The dialog spec lists two manifest names. Picking `logo` appends an image entry whose `resource_name` is `logo` and whose rectangle is the drag. Cancel appends nothing. An upload spec posts one PNG and appends an entry named from that file.

## Step 10. Image from a variable

[Steps](#steps)

The dialog from [step 9](#step-9-image-source-dialog) gains a third source: a variable. The names are the survey catalog the removed pane used to list. They are shown in this dialog, not in a side list. No file is uploaded and no `resources.images` key is added.

That choice appends a `photo` entry for the same rectangle. `photo.field` is the flat field name (`field_12`, not `form_data.field_12`). The entry is not an image entry. The writer draws whatever bytes the print row supplies for that field.

The editor has no per-guest photo. The canvas shows a labeled placeholder in the rectangle, the field name, rather than a broken image.

### Check

Choosing `field_12` in the dialog appends one `photo` entry with that field and the dragged rectangle, and the manifest is unchanged. The placeholder spec shows the field name inside the box.

## Step 11. Variables in text

[Steps](#steps)

This step is its own topic. [Step 4](#step-4-text-tool) still creates the literal `Текст`. The work here is the control that inserts a variable into the text the operator is editing.

The outcome is fixed. The control is chosen in this step, and only one control is built. It opens from the text being edited, because the field pane is gone. The names are the same survey catalog. A variable becomes one mustache in that entry's `preentered`. `template` is set true. `content` stays null. Existing pipes in the string stay byte for byte. The canvas shows the demo sample for that mustache, which it already does for `{{field_1}}`.

The control inserts at the caret. It does not drop a new box onto the sheet.

### Check

With a text entry whose `preentered` is `Текст`, inserting `field_1` at the end stores `Текст{{field_1}}` and `template: true`. A second insert of `field_2` stores both mustaches. A string that already contains `{{field_1|upper}}` still contains that pipe after the insert.

## Step 12. Color pickers

[Steps](#steps)

✅ Completed. Foreground sits over background at the bottom of the rail, with a swap control. A click opens a dialog with Linear and Radial tabs, R, G, B, and hex. The dialog mounts the bundled Preact build. A chosen `#RRGGBB` is stored in `ticket-editor:text-style` beside the font. `tool-rail.component.spec.ts` sets `#FF6A00` from the hex field and the linear square, keeps it on the radial tab, and reads it back from a new cache. A React editor imports `ticket-editor-color-picker/react` and does not load Preact.

Two swatches sit at the bottom of the rail, pinned under the tools, overlapped like the Photoshop and Illustrator color boxes. The foreground rectangle is in front, at the top left. The background rectangle is behind it, shifted down and to the right. A swap control in the top-right corner exchanges the two colors (the X key does the same). The rail stays `3.5rem`. A click on a swatch opens a dialog for that color. The strip cannot hold the picker.

The dialog has two tabs. **Linear** is a saturation/value square and a hue slider. **Radial** is a color wheel and a value slider. Both tabs edit the same color. The round handle follows the pointer. Under them are three fields, R, G, and B, each an integer from 0 to 255, one hex field, and Apply. The value stored is `#RRGGBB`. The page rejects any other form, including alpha. A hex that is not `#RRGGBB` is not written. While the dialog is open the swatch shows the draft. Apply keeps it. A click outside the dialog, or closing it any other way before Apply, puts back the color the dialog opened with.

The square and the wheel come from the package in [step 14](#step-14-color-picker-package). The Linear tab mounts its box and a hue slider. The Radial tab mounts its wheel and a value slider. The dialog, the tabs, and the R, G, B, and hex fields stay in this editor. This editor imports `ticket-editor-color-picker`, the build with Preact included (`node scripts/build.mjs preact` in `ticket-editor/color-picker`). A React editor imports `ticket-editor-color-picker/react` instead. That build leaves React external and does not load Preact.

A chosen color is written to the current selection and remembered in the same store as the last font. That store is `TextStyleMemory` in `text-style.ts`, key `ticket-editor:text-style`. Two fields join the font, the size, and the style: `foreground` and `background`. Until the operator changes them, foreground is `#000000` and background is `#FFFFFF`.

The pickers write the current selection.

- Foreground on a text entry, including a rectangle from [step 5](#step-5-rectangle-tool), writes `text.color`.
- Background writes `background`.
- Several selected entries all receive the change.
- With nothing selected, the swatches are the colors the next Text or Rectangle entry will be born with. [Step 4](#step-4-text-tool) reads `foreground` into `text.color`. [Step 5](#step-5-rectangle-tool) reads `background` into `background` and `foreground` into `text.color`. The rectangle's border stays `{ width: 0.2, color: "#000000" }` until something else changes it. The border is not a swatch on this rail.

Image, QR, barcode, and photo entries have no text color. Foreground does not write them. Background writes `background`.

### Check

A spec opens the foreground dialog, sets `#FF6A00` from the hex field and from the linear tab, and with a text entry selected reads `text.color`. The background swatch on a rectangle reads `background`. The same choice is still in `ticket-editor:text-style` after reload, beside the font. A value that is not `#RRGGBB` is not written. Switching to the radial tab keeps the same color.

## Step 13. Constraints

[Steps](#steps)

A ninth tool, Constraints, is added under Colorpicker. It does not draw. With fewer than two boxes selected its actions are disabled. With two or more, it offers alignment of the selection relative to itself:

- left edges, horizontal centers, right edges
- top edges, vertical centers, bottom edges
- equal horizontal gaps, and equal vertical gaps, when three or more boxes are selected

The action writes `posX` and `posY` on the entries. The page has no constraint field, so the rule is not stored. Moving a box afterwards does not re-apply it. A later step can store the rule when the page has a place for it, and that step is added by [Adding a step](#adding-a-step).

The reference box is the first selected entry. The others move to it. Equal gaps keep the outermost edges where they are and space the boxes between them.

### Check

Two boxes, 10 mm wide, at `posX` 0 and 30, align-left so both have `posX` 0. Align-center of a 10 mm box at 0 and a 20 mm box at 40 leaves the first at 0 and moves the second to `posX` 5 (centers at 5 and 15 become a shared center). Three boxes distribute horizontally and the first and last `posX` stay put. `template.yml` gains no constraint key.

## Step 14. Color picker package

[Steps](#steps)

✅ Completed. `ticket-editor/color-picker` builds three files from one source. `node scripts/build.mjs react` leaves React external. `node scripts/build.mjs preact` includes Preact. `node scripts/build.mjs preact --external` leaves Preact external. The commands are in `ticket-editor/color-picker/README.md`. `src/bundles.test.mjs` checks the imports and reads `#FF6A00` back from the box and the wheel on each build.

`ticket-editor/color-picker` is its own package. It is not an Angular library and it is not part of `ticket-editor/frontend`. [Step 12](#step-12-color-pickers) mounts the built picker. It does not depend on `@jaames/iro`.

The source is written once. Components import `h`, `Component`, and `Fragment` from `preact`, and the TypeScript JSX transform is the classic one (`jsxFactory: h`). The package draws three controls: a saturation/value box, a hue or value slider, and a color wheel. The value in and out is `#RRGGBB`. Alpha, kelvin, and several colors on one control are out of this package.

`compat/preact-react.ts` is the React stand-in for that import. `h` is React's `createElement`, and it rewrites `ontouchstart` to `onTouchStart`. `Component` extends React's class component and assigns the root element to `this.base`, which the drag math reads. `render` stores a `createRoot` per parent. Each control returns one root element so that ref can attach.

`node scripts/build.mjs` takes `preact` or `react`. The commands are in `ticket-editor/color-picker/README.md`.

- `react` aliases `preact` to the shim and writes `dist/react.js`. `react`, `react-dom`, and `react-dom/client` stay external. The file does not import `preact` and does not contain React.
- `preact` writes `dist/preact.js` with Preact included in the file. The file imports neither `preact` nor `react`.
- `preact --external` writes `dist/preact.external.js`. `preact` stays external. The file does not import `react`.

`package.json` exports `.` as the bundled Preact build, `./preact` as the external Preact build, and `./react` as the React build. `preact`, `react`, and `react-dom` are optional peers. A React app that depends on `ticket-editor/color-picker/react` keeps the single React it already loaded. The controls, the shim, and the build script are written in this package.

### Check

`node scripts/build.mjs react` emits a bundle whose imports name `react` and `react-dom` and do not name `preact`. `node scripts/build.mjs preact` emits a bundle that imports neither. `node scripts/build.mjs preact --external` emits a bundle whose imports name `preact` and do not name `react`. The same `#FF6A00` set through the box and through the wheel reads back as `#FF6A00` from each build.

## Step 15. Typing history

[Steps](#steps)

While a text box is being edited, ⌘Z removes the last character, or the last block of characters when they were typed together. A pause, a word boundary, or a switch between typing and deleting starts a new block. Clicking outside the box, pressing Enter, or moving the box folds that whole visit into one event on the same history as creating a text box, a QR, or a barcode. The next ⌘Z then restores the text from before the visit. The rules are in [typing-history.md](typing-history.md).

### Check

A spec types `ab`, waits, types `c`, and undo restores `ab`, then `a`, then the original string. A fast `hello` is one block. A space ends that block. Clicking outside makes the next undo restore the text from before the visit in one step, and redo puts the whole string back. Moving the box does the same fold. Escape discards the visit and adds no page event.

## Step 16. Run

[Steps](#steps)

Steps 1–14 are already checked. Typing history is [step 15](#step-15-typing-history). This is the click-through on one sheet.

```bash
cd ticket-editor/frontend && npx ng serve --port 4213 --host 127.0.0.1
```

Open `http://127.0.0.1:4213/editor/1`. The rail is `3.5rem` and Select is pressed. The sheet has no field list, no file list, and no **Загрузить файл**.

1. Text. Click the sheet. A 40×10 mm box reads `Текст`. Save. `template.yml` has that entry.
2. Rectangle. Drag a box. The sheet shows a white rectangle with a hairline border.
3. Barcode, then QR. Each drag adds a code. Shift on the QR drag makes a square. The QR picture and the EAN-13 picture both paint.
4. Select box. Drag a band around two of them. Both are selected. A band that only clips a third box leaves that box out.
5. Image. Drag a rectangle. The dialog lists the template files. Pick one, or upload a PNG. The picture fills the rectangle. Cancel on a second drag leaves the page unchanged.
6. In that dialog, pick a variable. The rectangle shows the field name and the entry is a `photo`.
7. Edit the text box and insert a variable. The canvas shows the demo value. Save keeps the mustache.
8. Colorpicker. Click the text box. The foreground swatch becomes that entry's text color, and the entry itself is unchanged. Click a pixel of the image. The foreground swatch becomes that pixel.
9. Select the rectangle. Set the background swatch and the foreground swatch. Save writes `background` and `text.color` as `#RRGGBB`.
10. Select two boxes. Constraints, align left. Their `posX` match. Reload. The positions stick, and the file has no constraint key.
11. Create a text box, a QR, or a barcode. ⌘Z removes that entry. ⌘⇧Z puts it back. The file changes only on Save.
