# Tool plugins refactor audit

Scope: the finished steps of [tool-plugins-refactor.md](tool-plugins-refactor.md), checked against [tool-plugins.md](tool-plugins.md).

The tool registry, the shared drag, the text tool, the caret session, one apply path, and one cursor attribute are in place. The unit specs named in that plan stay green. The issues below are logic gaps, split paths that can disagree, and performance cost that the unit checks do not catch.

Severity:

- **High**: typed text can be lost, or every place pays a full sheet rebuild.
- **Medium**: a wrong box, a wrong caret, a forked host, or a missing sheet check.
- **Low**: cost or leftover surface that does not change today’s gestures.

## Summary

| ID | Severity | Issue |
|---|---|---|
| ✅ [TP-01](#tp-01-tool-change-does-not-finish-an-open-text-edit) | High | Tool change does not finish an open text edit |
| ✅ [TP-02](#tp-02-every-append-rebuilds-the-whole-sheet) | High | Every append rebuilds the whole sheet |
| ✅ [TP-03](#tp-03-entry-resolved-from-pagecontents-not-toolhostpage) | Medium | Entry resolved from `pageContents`, not `ToolHost.page()` |
| ✅ [TP-04](#tp-04-two-select-double-click-openers-one-has-no-caret-point) | Medium | Two Select double-click openers; one has no caret point |
| ✅ [TP-05](#tp-05-split-toolhost-stub-pointerhost-vs-real-toolhost) | Medium | Split `ToolHost`: stub `pointerHost` vs real `toolHost` |
| ✅ [TP-06](#tp-06-cursor-attribute-and-css-written-twice) | Medium | Cursor attribute and CSS written twice |
| ✅ [TP-07](#tp-07-step-7-sheet-e2e-not-re-run-after-toolgesture) | Medium | Step 7 sheet e2e not re-run after `toolGesture` |
| ✅ [TP-08](#tp-08-full-structuredclone-on-every-history-record) | Low | Full `structuredClone` on every history `record` |
| ✅ [TP-09](#tp-09-dispose-drops-an-open-edit-without-commit) | Low | `dispose()` drops an open edit without commit |
| ✅ [TP-10](#tp-10-dead-surface-and-file-drift) | Low | Dead surface and file drift |

## High

### TP-01. Tool change does not finish an open text edit

✅ **Status: fixed.** `TextEditSession.finish()` commits the open box. The visual editor calls it when the tool input changes. Leaving visual mode calls `finishTextEdit()` before the canvas is torn down. `dispose()` finishes first so a destroy does not drop typed text. `text-edit.spec.ts` covers finish and dispose.

**Where:** `tool-rail.component.ts` (`toolChange`); `template-editor.component` (`tool.set`); `visual-editor.component.ts` `ngOnChanges` tool branch; `tools/text-edit.ts` (`commit` only from blur / Escape).

**What is broken:** Switching tools from the rail only updates the pointer session and the cursor. It never blurs or commits `TextEditSession`. Typed text can sit in a live `contenteditable`, then disappear when the next append runs `showPage` and rebuilds the DOM. Keyboard shortcuts are gated on `[contenteditable]`; a rail click is not.

**Fix:** On tool change (and leaving visual mode), commit or cancel the session before attaching the new gesture.

### TP-02. Every append rebuilds the whole sheet

✅ **Status: fixed.** Tool appends call `entryToHtml` + `appendEntry` (`addComponents`) instead of `showPage`. Full reload stays for undo, traits, and color. `keepViewport` cancels a prior restore and reapplies on the next frame only (no stacked 400 ms timer). `page-view.spec.ts` covers `entryToHtml`.

**Where:** `template-editor.component.ts` `onToolGesture` → `showPage`; `visual-editor.component.ts` `reloadDocument` / `keepViewport`.

**What is broken:** Each append runs `pageToHtml`, Grapes `setComponents`, UndoManager clear, and `keepViewport` (immediate, rAF, and a **400 ms** reapply). Zoom stays put, which is correct. The cost per place is large, and the delayed reapply can fight a pan or zoom started right after placing.

**Fix:** Insert the new box incrementally when possible. Cancel prior `keepViewport` timers. Reapply the viewport once after the frame settles.

## Medium

### TP-03. Entry resolved from `pageContents`, not `ToolHost.page()`

✅ **Status: fixed.** `onToolGesture` resolves `entry` with `entryFromTarget(pageDocument, target)` before `onGesture`. The canvas emits `entry: null` and no longer takes a `pageContents` input. `sheet-tool.spec.ts` covers the lookup.

**Where:** `visual-editor.component.ts` `entryFrom` / `onDraw`; `@Input() pageContents`; `template-editor` `pageContents` getter; `tools/text.tool.ts` `onGesture` (missing entry → append).

**What is broken:** The design resolves `entry` from the live page before `onGesture`. The canvas looks up a mirrored `@Input`. The tool runs against `pageDocument`. A missing or stale entry makes Text treat the click as append.

**Fix:** Resolve the entry in `onToolGesture` from `pageDocument.contents` (one source of truth).

### TP-04. Two Select double-click openers; one has no caret point

✅ **Status: fixed.** Removed `component:dblclick`. Select double-click uses only the frame `dblclick` listener with client coordinates. `editPreentered` requires a caret point.

**Where:** `visual-editor.component.ts` `bindPreenteredEdit` (frame `dblclick` with client coordinates); `editor.on('component:dblclick', …)` → `editPreentered(component)` with `{ x: 0, y: 0 }`; `tools/text-edit.ts` same-element branch (only moves the caret).

**What is broken:** The second opener can yank the caret to 0,0 after the first placed it on the clicked glyph.

**Fix:** Keep one path (the frame listener with client coordinates). Drop or gate `component:dblclick`.

### TP-05. Split `ToolHost`: stub `pointerHost` vs real `toolHost`

✅ **Status: fixed.** `PointerHost` is only `editingElement()`. `onPointerDown` takes that type. The canvas stub no longer pretends to be a full `ToolHost`. `ToolHost` extends `PointerHost` for the editor factory.

**Where:** `visual-editor.component.ts` `pointerHost` (`page: () => null`, default style, zero measure); `template-editor.component.ts` `toolHost`; `drawMode` → `onPointerDown`.

**What is broken:** Text’s pointer hook only needs `editingElement` today, so it works. Any future `onPointerDown` that reads `page`, `style`, or `measureLine` will silently misbehave. The design is one narrow host.

**Fix:** One host factory from the editor, or a narrow `PointerHost` type so stubs cannot pretend to be full `ToolHost`.

### TP-06. Cursor attribute and CSS written twice

✅ **Status: fixed.** `updateFrameStyle` no longer writes `data-draw-cursor` or cursor selectors. `DrawPointerSession.setCursor` is the only writer (`style[data-draw-cursor]`, including `.gjs-is__grab`).

**Where:** `pointer-session.ts` `CURSOR_CSS` / `setCursor`; `visual-editor.component.ts` `updateFrameStyle` (attribute + embedded selectors). Parent `styles.scss` tool-id cursor rules are gone.

**What is broken:** Two frame writers can drift after a Grapes style reset.

**Fix:** One writer, one stylesheet snippet (keep `.gjs-is__grab` if still needed).

### TP-07. Step 7 sheet e2e not re-run after `toolGesture`

✅ **Status: fixed.** Re-ran `e2e/text-tool.spec.mjs`, `e2e/rect-tool.spec.mjs`, and `e2e/code-tool.spec.mjs` on port **4321** (`E2E_API_PORT`); `environment.ts` restored to 4310. Playwright config reads that port. `on-tool-gesture.spec.ts` covers resolve → tool → apply. Sheet typing also required: tool-rail ignores shortcuts while a frame `contenteditable` is focused (so `t` does not steal the Text tool mid-edit), and Enter uses `insertLineBreak` so the caret stays after the break.

**Where:** [tool-plugins-refactor.md](tool-plugins-refactor.md) Step 7 Check; `e2e/text-tool.spec.mjs`, `e2e/rect-tool.spec.mjs`, `e2e/code-tool.spec.mjs`; `tools/apply-effect.spec.ts` (unit only).

**What is broken:** Unit green does not cover `onToolGesture` on the live sheet. The plan’s sheet check after the apply wiring was not re-run on a non-4310 port.

**Fix:** Re-run those three e2e specs as written. Optionally add a thin `onToolGesture` harness test.

## Low

### TP-08. Full `structuredClone` on every history `record`

✅ **Status: fixed.** `clonePage` deep-clones each content entry and copies `page_size` / `resources` as shells (replace-on-write). `record` / `undo` / `redo` use it. `page-history.spec.ts` covers isolation from later in-place edits.

**Where:** `page-history.ts` `record` / `undo` / `redo`; `tools/apply-effect.ts` calls `history.record(page)` before `run`.

**What is broken:** Correct for undo. Stacks with full `showPage` as sheets grow.

**Fix:** Structural sharing or clone-on-write for `contents` if undo semantics allow.

### TP-09. `dispose()` drops an open edit without commit

✅ **Status: fixed.** `dispose()` calls `finish()` first (same commit path as tool change). Covered by `text-edit.spec.ts` (“dispose keeps a change before the canvas is torn down”).

**Where:** `tools/text-edit.ts` `dispose`; `visual-editor.component.ts` `ngOnDestroy`.

**What is broken:** Destroy strips `contenteditable` and never emits. Rare in normal use; same class of bug as [TP-01](#tp-01-tool-change-does-not-finish-an-open-text-edit).

**Fix:** Commit or cancel in `dispose`, or call commit first from destroy and tool change.

### TP-10. Dead surface and file drift

✅ **Status: fixed.** `readonlySheet` removed. `adjustRect` stays on `dragTool` / QR (not the session). [tool-plugins.md](tool-plugins.md) file list matches the tree (Select / Select box stay inline in `registry.ts`). Host `data-tool` kept for e2e only; cursor is `data-draw-cursor` in the frame.

**Where:** `ToolHost.readonlySheet`; `SheetTool.adjustRect`; [tool-plugins.md](tool-plugins.md) file list; host `[attr.data-tool]` vs `data-draw-cursor`.

**What is broken:** No gesture break today. The surface and the docs disagree.

**Fix:** Trim or use the unused fields. Optional file split. Keep `data-tool` for tests or assert `data-draw-cursor` instead.

## Not findings

| Topic | Verdict |
|---|---|
| Image / Select box return `none` | Intentional until those [toolbox](toolbox-plan.md) steps start |
| Text `placeGuard` / yield / outside blur → take | Matches [How typing plugs into the text tool](tool-plugins.md#how-typing-plugs-into-the-text-tool) |
| Short drag finishes with `rect: null` then `none` | Second line of defence, as planned |
| `toolById` over seven tools | Negligible |
