import { appendTextAt, ptToMm, type PageEntry } from '../page-view';
import type { GestureInput, PointerHost, SheetTool, ToolEffect, ToolHost } from './sheet-tool';

/** A text or rectangle box. Image, QR, and barcode stay out. */
export function isEditableText(entry: PageEntry | null | undefined): boolean {
  return !!entry?.text;
}

function edit(input: GestureInput): ToolEffect {
  const entry = input.entry;
  if (!entry) return { kind: 'none' };
  return { kind: 'edit-text', id: entry.id, client: input.client };
}

export const textTool: SheetTool & { id: 'text' } = {
  id: 'text',
  label: 'Text',
  icon: 'fa-solid fa-font',
  position: 30,
  key: 't',
  creates: true,
  cursor: 'text',
  gesture: 'click',
  onPointerDown(event: PointerEvent, host: PointerHost) {
    const editing = host.editingElement();
    if (!editing) return 'take';
    if (event.target instanceof Node && editing.contains(event.target)) return 'yield';
    editing.blur();
    return 'take';
  },
  onGesture(input, host) {
    const text = isEditableText(input.entry);
    if (host.placeGuard()) return text ? edit(input) : { kind: 'none' };
    if (text) return edit(input);
    const style = host.style();
    const measureLine = host.measureLine(style.font, ptToMm(style.fontSizePt), style.bold, style.italic);
    return {
      kind: 'append',
      run: (page) => appendTextAt(page, input.click, { style, measureLine }),
    };
  },
};
