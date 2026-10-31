# Refactor the sheet tools into plugins

[tool-plugins.md](tool-plugins.md) is the shape. This is the order to move the current rail onto it. Each step leaves the sheet behaving as it does now and names the test that proves that.

Do not start a toolbox step that has not been started. Image still appends nothing. Select box still selects nothing. The eyedropper, the image dialog, variables in text, constraints, and typing history stay where [toolbox-plan.md](toolbox-plan.md) left them. The text caret, the bottom-left corner of a new text box, the zoom kept across an edit, and undo selecting the changed entry stay as they are.

Run unit tests from `ticket-editor/frontend` with `npx ng test --include <spec> --watch=false`. Playwright stays off port 4310. `environment.ts` stays `apiBaseUrl: 'http://127.0.0.1:4310/api'` and `yamlPage: true`.

## Steps

| Step | What moves | Test |
|---|---|---|
| [1. Registry](#step-1-registry) | The seven tools as data. Nothing reads it yet | ✅ `tools/registry.spec.ts` |
| [2. Rail](#step-2-rail) | The strip renders the registry | ✅ `tool-rail.component.spec.ts` |
| [3. Session](#step-3-session) | The pointer session takes a gesture and a cursor | ✅ `pointer-session.spec.ts` |
| [4. Shared drag](#step-4-shared-drag) | Rectangle, Image, QR, and Barcode share one drag helper | ✅ `tools/drag-tool.spec.ts` |
| [5. Text](#step-5-text) | Text is a click tool with an edit branch | ✅ `tools/text.tool.spec.ts` |
| [6. Text editing](#step-6-text-editing) | The caret session is what `beginTextEdit` opens | ✅ `tools/text-edit.spec.ts` |
| [7. One apply](#step-7-one-apply) | `onDraw` and the three place handlers call the active tool | ✅ `tools/apply-effect.spec.ts` |
| [8. One cursor](#step-8-one-cursor) | Cursor CSS no longer lists tool ids | ✅ `pointer-session.spec.ts` |

## Step 1. Registry

Add `tools/sheet-tool.ts` and `tools/registry.ts`. Register Select, Select box, Text, Image, Rectangle, QR, and Barcode with the positions, icons, keys, `creates`, cursors, and gestures from [tool-plugins.md](tool-plugins.md#what-a-tool-is). `onGesture` may return `{ kind: 'none' }` for every tool in this step. Export `editorToolIds` and a shortcut map built from `key`.

`tool-rail.component.ts` still owns `TOOLS` and `KEYS`. The session still switches on `EditorTool`.

### Check

`registry.spec.ts`:

- Positions sort to Select, Select box, Text, Image, Rectangle, QR, Barcode.
- Keys are V, M, T, I, R, Q, B. Escape is absent.
- Select and Select box have `creates: false`. The other five have `creates: true`.
- Select is `canvas` and cursor `null`. Select box is `drag` and cursor `null`. Text is `click` and cursor `text`. Image, Rectangle, QR, and Barcode are `drag` and cursor `crosshair`.

✅ Completed. `tools/registry.ts` lists the seven tools. `onGesture` returns `none`. `editorToolIds` and `toolShortcuts` come from that list. The rail and the session do not read it until the next steps.

## Step 2. Rail

`ToolRailComponent` drops `TOOLS` and `KEYS`. It renders `registry` in position order. `choose`, the readonly check, and the keydown listener use the registry. `EditorTool` is the id type of that list. Swatches, the color dialog, and X to swap colors stay in this component and stay off the registry.

### Check

The existing rail spec still passes. The seven labels stay in that order, Select starts pressed, Text click presses Text, Escape returns to Select. Add one assertion: a readonly rail disables Image, Rectangle, QR, Barcode, and Text, and leaves Select and Select box enabled. The color-dialog cases in that spec stay green. They are not tools.

✅ Completed. The strip renders `toolsInOrder()`. Shortcuts come from `toolShortcuts`. A read-only sheet disables the five create tools. The color dialog is unchanged.

## Step 3. Session

`DrawPointerSession.setTool` takes `{ gesture, cursor, onPointerDown }` instead of an `EditorTool`. `gesture: 'canvas'` detaches and clears the cursor. `click` and `drag` attach. Only `drag` draws the band and applies the 2 mm rule. `click` always finishes, with `rect: null` and the snapped pointer-down point. Pointer down calls `onPointerDown` when the tool has one. `'yield'` skips capture and does not finish. `'take'` is the default.

`visual-editor` passes the active tool's `gesture` and `cursor` into the session. `onDraw` is unchanged, so the sheet still appends the same boxes.

### Check

Rewrite the cursor cases in `pointer-session.spec.ts` so they pass a description, not a tool id:

- `{ gesture: 'canvas' }` installs nothing and removes `data-draw-cursor`.
- `{ gesture: 'drag', cursor: 'crosshair' }` sets `data-draw-cursor="crosshair"` and the crosshair rule.
- `{ gesture: 'click', cursor: 'text' }` sets `data-draw-cursor="text"` and the text rule.
- A drag from (1, 1) to (1.2, 8) still finishes with `rect: null`. A drag from (1, 1) to (12, 9) still finishes with a snapped rect.
- A click tool's pointer up from (3, 4) to (3.4, 4.2) finishes once, with `rect: null` and click `(3, 4)` after snap.
- `onPointerDown` returning `yield` does not call finish. The existing band test still passes.

✅ Completed. `setTool` takes a gesture, a cursor, and an optional `onPointerDown`. Canvas detaches. Click finishes with no rectangle and the snapped pointer-down point, and draws no band. Drag still bands and still returns null when a side is under 2 mm. The visual editor passes the active tool's gesture and cursor. An open text caret still yields through `onPointerDown`, so typing is unchanged. `onDraw` is unchanged.

## Step 4. Shared drag

Add `dragTool()` as in [tool-plugins.md](tool-plugins.md#one-drag-for-rectangle-image-qr-and-barcode). Point Rectangle, QR, and Barcode at it. Rectangle's `place` is `appendRect`. QR's `place` is `appendCode(..., 'qr')` and its `adjustRect` is `placedCodeRect`. Barcode's `place` is `appendCode(..., 'ean13')` with no `adjustRect`. Image uses `dragTool` and its `place` is unused: `onGesture` returns `{ kind: 'none' }`. Select box is a `SheetTool` with `gesture: 'drag'` and `onGesture` returning `{ kind: 'none' }`.

Do not call these from `onDraw` yet.

### Check

`drag-tool.spec.ts`, with a page fixture and no canvas:

- Rectangle, Image, QR, and Barcode share `gesture: 'drag'` and `cursor: 'crosshair'`.
- A rectangle gesture with a 10×6 rect returns `append`, and `run` pushes one text entry at that rect.
- QR with `shift: true` stores a square (`lockSquare`). QR with `shift: false` stores the drag. Barcode with `shift: true` stores the drag.
- `onGesture` with `rect: null` returns `none` for all four.
- Image and Select box return `none` for a real rect, and the page contents length does not change.

`page-view.spec.ts` already covers `appendRect` and `appendCode`. It stays green.

✅ Completed. `dragTool()` builds the four crosshair drags. Rectangle appends an empty text entry at the drag. QR with Shift stores a square, and without Shift stores the drag. Barcode stores the drag even with Shift. A missing rectangle returns `none`. Image and Select box return `none` for a real drag, so nothing is appended. `onDraw` does not call these yet.

## Step 5. Text

`text.tool.ts` is a `click` tool, cursor `text`, key T. It implements `onPointerDown` and `onGesture` as in [How typing plugs into the text tool](tool-plugins.md#how-typing-plugs-into-the-text-tool). The spec passes a fake host. The real caret session is step 6. `onDraw` still has the text branch until step 7.

### Check

`text.tool.spec.ts`:

- Empty `entry` and `placeGuard()` false returns `append`. `run` on a page puts the bottom-left corner on the snapped click. The existing `(12.2, 7.4)` case in `page-view.spec.ts` is the size check; this spec only asserts the effect kind and that `run` delegates to that function (one new id, `preentered` `Текст`).
- A text entry returns `edit-text` with that id and the client point, and does not call `appendTextAt`.
- An image or code entry returns `append`.
- `placeGuard()` true and a text entry returns `edit-text`. `placeGuard()` true and no entry returns `none`.
- `onPointerDown` returns `yield` when `editingElement()` contains the target, and `take` after blurring when the target is outside.

✅ Completed. `text.tool.ts` is the click tool. Empty sheet and image or code clicks return `append`. A text entry returns `edit-text`. The place guard blocks an empty append and still allows edit. `onPointerDown` yields inside the box being typed. `onDraw` still has the text branch until step 7.

## Step 6. Text editing

Move the caret session out of the visual editor's private methods and behind the three host methods in [tool-plugins.md](tool-plugins.md#how-typing-plugs-into-the-text-tool). `editingElement()` is today's `inlineEl`. `placeGuard()` is today's `textPlaceGuard`. `beginTextEdit(id, client)` is today's `editPreentered`.

`tools/text-edit.ts` owns that session. The visual editor calls it. The text tool does not touch the DOM. It only returns `edit-text`, and step 7 turns that into `beginTextEdit`. A double-click while Select is active calls `beginTextEdit` too, because Select has no gesture.

The session keeps the behavior already on the sheet:

- The box becomes `contenteditable`. Typing uses the font, size, and weight already on that element. There is no second text window.
- The caret is the glyph at `client`. A click in the empty part of the box goes to the end. The same id only moves the caret.
- Enter inserts a newline at the caret and stays in the box. It does not commit and it does not add a page event.
- Blur commits. Changed text emits one `preenteredEdit` (`<br>` for newlines). Unchanged text puts the previous shown string back and emits nothing. Escape sets cancel, then blur restores and emits nothing.
- Blur arms the place guard for 300 ms. That flag is what step 5's `placeGuard()` reads.
- A pointer down inside the editing element is the text tool's `'yield'`. A pointer down outside blurs, which commits, and then the guard covers that click.
- `beginTextEdit` does not call `showPage`. The commit's `preenteredEdit` reloads through `reloadDocument`, which keeps zoom and pan.
- While the element is `contenteditable`, the canvas key handler returns before page undo and before component delete.

### Check

`text-edit.spec.ts`, with an element in a document and no GrapesJS:

- `begin` sets `contenteditable="true"`, writes the source (`<br>` becomes newlines), and leaves the element's `font-family` as it was.
- A client point on a glyph places the caret there. The same id a second time only moves the caret.
- Enter inserts a newline and the session stays open.
- Blur after a change calls the commit listener once with `linesToBreaks` of the typed string, then `contenteditable` is `false` and `placeGuard()` is true. A second blur does not commit again.
- Blur with the same string does not call the commit listener, and the element shows the string it had before `begin`.
- Escape then blur does not call the commit listener, and the element shows that same opening string.
- `placeGuard()` is false again after 300 ms.

The canvas key handler is still the one in the visual editor. This spec does not load the editor. Step 7's sheet check covers a click that types, a click outside that keeps the text, and Escape that puts the old text back, with the zoom unchanged.

✅ Completed. `tools/text-edit.ts` owns the caret session. The visual editor calls it from `editPreentered` and exposes `beginTextEdit`. Typing stays in the box, in its font. Enter inserts a line. Blur commits once and arms the place guard. Unchanged text and Escape restore the opening string. `onDraw` still opens the session through `editPreentered` until step 7 wires `edit-text`.

## Step 7. One apply

`onDraw` becomes: resolve the entry, call `registry.byId(this.tool).onGesture`, then `applyToolEffect`. Delete the `text` / `rect` / `qr` / `barcode` branches. `textPlace`, `rectPlace`, and `codePlace` can stay as outputs for one commit if a caller still binds them; `TemplateEditorComponent` stops using them.

`applyToolEffect` lives on `TemplateEditorComponent`:

- `none`: return.
- `edit-text`: `host.beginTextEdit`. No history record.
- `append`: `pageHistory.record`, `run(page)`, `showPage(entry.id)`, `markDirty`. This replaces `onTextPlace`, `onRectPlace`, and `onCodePlace`.

The session calls `tool.onPointerDown` instead of `yieldTextEdit`. Delete `yieldTextEdit` and the `tool === 'text'` branch in `onDraw`.

### Check

`page-view.spec.ts` for the stored geometry. Add a small spec for `applyToolEffect` if it is a pure function of `(page, history, effect)`; if it stays a method, cover it by calling the method with a fake page in `template-editor.component.spec.ts`:

- An append effect records one history step, selects the new id, and leaves the previous page on the undo stack.
- An `edit-text` effect does not record history.
- `none` does not change the page.

`e2e/text-tool.spec.mjs`, `e2e/rect-tool.spec.mjs`, and `e2e/code-tool.spec.mjs` are the sheet check after this step, on a port other than 4310. Restore `environment.ts` and delete any Playwright override when the run finishes.

✅ Completed. The canvas emits `toolGesture`. The editor runs the active tool and `applyToolEffect`. An append records history, shows the new id, and marks dirty. An `edit-text` opens the caret and does not record history. `none` leaves the page alone. The old `textPlace` / `rectPlace` / `codePlace` handlers are gone. Pointer down uses the tool's `onPointerDown`. `apply-effect.spec.ts` covers the three results. `page-view.spec.ts` stays green for the stored geometry.

## Step 8. One cursor

Delete `CROSSHAIR_TOOLS`. `updateFrameStyle` sets `data-draw-cursor` from `tool.cursor` only, so a Grapes style reset does not need a tool-id list. Remove the `app-visual-editor[data-tool=…]` cursor blocks in `styles.scss`. The frame stylesheet keeps the two attribute rules it has now, for `crosshair` and `text`.

### Check

`pointer-session.spec.ts` from step 3. On the sheet, Text shows the I-beam over the canvas document, and Rectangle, Image, QR, and Barcode show the crosshair. Select clears the attribute. Zoom stays where it was while a text box is edited and while a box is moved.

✅ Completed. `updateFrameStyle` sets `data-draw-cursor` from `tool.cursor`. The parent `styles.scss` tool-id cursor rules are gone. The frame stylesheet still paints `crosshair` and `text` from that attribute. `pointer-session.spec.ts` still passes.

## After the last step

A new drag tool is `dragTool({ ... })` plus a registry line. A new click tool is a `SheetTool` plus a registry line. Neither one edits `onDraw`, the rail template, or the cursor CSS.
