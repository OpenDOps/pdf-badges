import type { DrawnRect } from '../draw-tool';
import type { PageDocument, PageEntry } from '../page-view';
import type { EditorTool } from './registry';
import type { SheetTool, ToolEffect, ToolHost } from './sheet-tool';

/**
 * Corner drag shared by Rectangle, Image, QR, and Barcode.
 * A missing `place` finishes the drag without appending. Image uses that until its dialog exists.
 */
export function dragTool(spec: {
  id: EditorTool;
  label: string;
  icon: string;
  position: number;
  key: string;
  adjustRect?(rect: DrawnRect, shift: boolean): DrawnRect;
  place?(page: PageDocument, rect: DrawnRect, host: ToolHost): PageEntry;
}): SheetTool & { id: EditorTool } {
  return {
    id: spec.id,
    label: spec.label,
    icon: spec.icon,
    position: spec.position,
    key: spec.key,
    creates: true,
    cursor: 'crosshair',
    gesture: 'drag',
    adjustRect: spec.adjustRect,
    onGesture(input, host): ToolEffect {
      if (!input.rect || !spec.place) return { kind: 'none' };
      const rect = spec.adjustRect?.(input.rect, input.shift) ?? input.rect;
      const place = spec.place;
      return { kind: 'append', run: (page) => place(page, rect, host) };
    },
  };
}
