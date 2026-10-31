# Pluggable sheet tools

The tool rail, the canvas pointer, and the code that appends a box each keep their own list of tools. A new tool means a new `EditorTool` member, a button in `TOOLS`, a key in `KEYS`, a cursor branch in three places, and another `if (this.tool === …)` in `onDraw`. Rectangle, Image, QR, and Barcode already do the same gesture and then diverge only in what they store.

This note describes one tool object that owns its button, its cursor, and its gesture. The pointer session stays one implementation. The editor applies the result. [tool-plugins-refactor.md](tool-plugins-refactor.md) is the order to get there. This is not the print toolbar in [toolbar-contributions.md](toolbar-contributions.md): those are Angular components on a route. A sheet tool is a plain object.

## What a tool is

A tool is a `SheetTool`. The rail sorts by `position` and draws one radio button per tool. The pointer session reads `cursor` and `gesture` and never switches on an id.

```ts
type ToolCursor = 'crosshair' | 'text' | null;

type ToolGesture = 'canvas' | 'click' | 'drag';

interface SheetTool {
  id: string;
  label: string;
  /** Font Awesome classes, as the rail draws them today. */
  icon: string;
  /** Top of the rail is the smaller number. */
  position: number;
  /** Bare key, no modifier. Escape stays Select and is not a tool key. */
  key?: string;
  /** A read-only sheet disables these buttons. Select and Select box are false. */
  creates: boolean;
  cursor: ToolCursor;
  gesture: ToolGesture;
  /**
   * Drag only. Shift-QR locks a square. The others return the rect unchanged.
   * Called after the 2 mm check, so a short drag never arrives.
   */
  adjustRect?(rect: DrawnRect, shift: boolean): DrawnRect;
  /**
   * The pointer went up. A click tool gets `rect: null`. A drag tool gets the
   * snapped rect, or is not called when a side is under 2 mm.
   * `entry` is the page entry under the pointer, when the element has a yaml id.
   */
  onGesture(input: GestureInput, host: ToolHost): ToolEffect;
  /**
   * Pointer down, before the session captures. `'yield'` leaves the event to
   * the browser. `'take'` is the default when a tool omits this.
   * Text returns `'yield'` while the caret is inside the box being typed.
   */
  onPointerDown?(event: PointerEvent, host: ToolHost): 'yield' | 'take';
}
```

`EditorTool` becomes the id union of the registered tools, derived from the list. The rail, the visual editor, and the pointer session stop declaring the seven names themselves.

| Tool | `position` | `gesture` | `cursor` | `creates` | Key |
|---|---|---|---|---|---|
| Select | 10 | `canvas` | `null` | no | V |
| Select box | 20 | `drag` | `null` | no | M |
| Text | 30 | `click` | `text` | yes | T |
| Image | 40 | `drag` | `crosshair` | yes | I |
| Rectangle | 50 | `drag` | `crosshair` | yes | R |
| QR | 60 | `drag` | `crosshair` | yes | Q |
| Barcode | 70 | `drag` | `crosshair` | yes | B |
| Colorpicker | 80 | `click` | `crosshair` | no | E |

Gaps of ten leave room for a later tool without renumbering. Colorpicker is the eyedropper in [toolbox-plan.md](toolbox-plan.md#step-7-colorpicker-tool).

## Two gestures, one session

`DrawPointerSession` already listens on the canvas document for every tool except Select, draws the rubber band, snaps to 0.5 mm, and drops a drag under 2 mm. That stays the only pointer code. A tool does not add its own `mousedown` listener.

`gesture` tells the session which half of that to run:

| `gesture` | Listeners | Rubber band | Pointer up |
|---|---|---|---|
| `canvas` | off, GrapesJS moves and resizes | no | the tool is not called |
| `click` | on, capture, `preventDefault` | no | always. `rect` is null. `click` is the snapped pointer-down point |
| `drag` | on, same capture | yes, from pointer down to pointer move | only when both sides are at least 2 mm. `adjustRect` runs first |

A click tool that moves a few pixels is still a click. Text works that way today: the session may return a null rect, and the text branch uses the anchor point.

```ts
interface GestureInput {
  click: SheetPoint;
  rect: DrawnRect | null;
  shift: boolean;
  /** Frame client coordinates. Text places the caret with these. */
  client: { x: number; y: number };
  target: Element | null;
  entry: PageEntry | null;
}
```

The session builds `click`, `rect`, `shift`, `client`, and `target`, as `onPointerUp` does now. The host resolves `entry` from `data-yaml-id` before `onGesture`. The band, the 2 mm rule, snap, and pointer capture stay in the session. `normalizeRect` and `lockSquare` stay in `draw-tool.ts`.

## Clicking the same kind of object

`onGesture` always receives `entry`. The tool decides whether that entry changes the gesture.

Text is the tool that uses it. A `tpl-text` or `tpl-var` under the pointer returns an edit effect. Any other target, including empty sheet, returns an append effect. The bottom-left corner of a new text box stays the snapped click (`appendTextAt`). The click that closes an edit is the text tool's place guard, described with the rest of typing below.

Rectangle, Image, QR, and Barcode do not read `entry`. A drag that ends on an existing box still appends, which is the rule in [toolbox-plan.md](toolbox-plan.md#the-rail). A later tool can branch on `entry` without a new session.

## One drag for rectangle, image, QR, and barcode

Those four tools share the drag. They differ in `adjustRect` and in the entry they append. A helper builds the `SheetTool` so the four files do not each repeat the band, the minimum size, or the history step.

```ts
function dragTool(spec: {
  id: string;
  label: string;
  icon: string;
  position: number;
  key: string;
  adjustRect?(rect: DrawnRect, shift: boolean): DrawnRect;
  /** Called with a rect that already passed the 2 mm check and adjustRect. */
  place(page: PageDocument, rect: DrawnRect, host: ToolHost): PageEntry;
}): SheetTool
```

`onGesture` for that helper is:

1. If `input.rect` is null, return `{ kind: 'none' }`. The session already skips the call for a short drag; this is the second line of defence.
2. `const rect = spec.adjustRect?.(input.rect, input.shift) ?? input.rect`.
3. Return `{ kind: 'append', run: (page) => spec.place(page, rect, host) }`.

QR passes `adjustRect` that calls `lockSquare` when Shift is held. Barcode passes nothing, so Shift still stores the drag. Rectangle and Image pass nothing. Image's `place` returns no entry until [the source dialog](toolbox-plan.md#step-9-image-source-dialog) exists: the effect is `{ kind: 'none' }`, matching `onDraw` today, which has no image branch. Select box uses `gesture: 'drag'` and also returns `{ kind: 'none' }` until [step 8](toolbox-plan.md#step-8-select-box) reads the band. The helper is only for tools that append a box from a rectangle.

## What the tool returns

A tool does not record history, call `showPage`, or mark the sheet dirty. It returns an effect. The editor applies it, which is the body of `onTextPlace`, `onRectPlace`, and `onCodePlace` today, written once.

```ts
type ToolEffect =
  | { kind: 'none' }
  | { kind: 'edit-text'; id: string; client: { x: number; y: number } }
  | { kind: 'append'; run(page: PageDocument): PageEntry };
```

`append.run` is `appendTextAt`, `appendRect`, or `appendCode`. Those functions already push onto `page.contents`. The editor records history, runs `run`, selects the new id, and marks dirty. `edit-text` does not record a page event. Applying it calls `host.beginTextEdit`, which is the only way a tool starts typing.

`ToolHost` is the narrow door into the editor. Tools do not import `VisualEditorComponent` or `TemplateEditorComponent`.

```ts
interface ToolHost {
  page(): PageDocument | null;
  style(): TextStyleMemory;
  measureLine(font: string, sizeMm: number, bold: boolean, italic: boolean): (line: string) => number;
  /** The box currently being typed, or null. */
  editingElement(): HTMLElement | null;
  /** True for the click that just closed an edit. */
  placeGuard(): boolean;
  /** Open the caret in that box. The text tool is the caller. */
  beginTextEdit(id: string, client: { x: number; y: number }): void;
}
```

## How typing plugs into the text tool

The caret, Enter, blur, and Escape stay one session on the host (`editPreentered` today). The text tool is what decides when that session runs. The visual editor does not branch on `tool === 'text'`. The session calls the active tool, and only the text tool implements these two hooks.

Pointer down, before capture:

```ts
onPointerDown(event, host) {
  const editing = host.editingElement();
  if (!editing) return 'take';
  if (event.target instanceof Node && editing.contains(event.target)) return 'yield';
  editing.blur();
  return 'take';
}
```

`'yield'` means the session does not capture and does not `preventDefault`, so the browser puts the caret on the glyph. A pointer down outside the box blurs it. Blur is the commit: changed text emits one page event, unchanged text restores the previous string, Escape has already set the cancel flag so blur restores and emits nothing. Blur then arms the place guard for 300 ms.

Pointer up:

```ts
onGesture(input, host) {
  const text = input.entry && (input.entry is tpl-text or tpl-var);
  if (host.placeGuard()) return text ? edit(input) : { kind: 'none' };
  if (text) return edit(input);
  return { kind: 'append', run: (page) => appendTextAt(page, input.click, …) };
}

function edit(input): ToolEffect {
  return { kind: 'edit-text', id: input.entry.id, client: input.client };
}
```

The guard is the text tool reading `host.placeGuard()`. A click on empty sheet in that window returns `none`, so nothing is appended. A click on another text box returns `edit-text`, so that box opens. After 300 ms `placeGuard()` is false and an empty click appends again.

`applyToolEffect` sees `edit-text` and calls `host.beginTextEdit(id, client)`. That is the contenteditable session: the box's own font, the caret at `client`, Enter inserts a newline and stays, blur commits, Escape restores. Same-id `beginTextEdit` only moves the caret. A double-click while Select is active calls `beginTextEdit` too. Select has no gesture, so that path is the canvas double-click listener, not `onGesture`.

No other tool implements `onPointerDown` or returns `edit-text`. Rectangle, Image, QR, and Barcode never see the caret or the guard.

## Files

```
ticket-editor/frontend/src/app/operator/templates/tools/
  sheet-tool.ts       SheetTool, GestureInput, ToolEffect, ToolHost, PointerHost
  drag-tool.ts        dragTool()
  registry.ts         the tools (Select and Select box inline), shortcut map
  text.tool.ts
  text-edit.ts        caret session behind beginTextEdit
  image.tool.ts
  rect.tool.ts
  qr.tool.ts
  barcode.tool.ts
  colorpicker.tool.ts
  sample-color.ts     hit test and pixel → hex for the eyedropper
  apply-effect.ts
  registry.spec.ts
  drag-tool.spec.ts
  text.tool.spec.ts
  text-edit.spec.ts
  colorpicker.tool.spec.ts
  sample-color.spec.ts
  sheet-tool.spec.ts
  on-tool-gesture.spec.ts
  apply-effect.spec.ts
```

`draw-tool.ts` keeps snap, `normalizeRect`, `lockSquare`, and `placedCodeRect`. `pointer-session.ts` keeps the listeners and the band, and takes the active `SheetTool` instead of an `EditorTool`. `tool-rail.component.ts` loops `registry` and keeps the swatches. The swatches are not tools.

Cursor is one attribute, `data-draw-cursor`, set from `tool.cursor` on the canvas document. The frame stylesheet paints `crosshair` and `text` from that attribute (including `.gjs-is__grab`). Parent `styles.scss` has no tool-id cursor rules. The host still sets `data-tool` for e2e assertions only; it does not drive the cursor.

## What stays outside a tool

The text tool decides when to type and when the closing click must not append. The host still owns the DOM session those hooks call: contenteditable, caret placement, Enter, blur, Escape, and the 300 ms flag behind `placeGuard()` and `editingElement()`.

- Viewport. A tool effect must not fit the sheet. `reloadDocument` already keeps zoom and pan.
- Undo selects the entry the effect changed. That stays in `onHistoryKey`.
- Foreground and background swatches, and the color dialog.
- GrapesJS as the Select tool. `gesture: 'canvas'` means the session detaches.

Adding a tool is a new file, one `dragTool(...)` or one `SheetTool`, and a line in `registry.ts`. The rail, the session, and `applyToolEffect` do not change.
