import { boxesInside, type BoxRect } from '../draw-tool';
import type { PageEntry } from '../page-view';
import type { GestureInput, SheetTool, ToolEffect, ToolHost } from './sheet-tool';

function asBox(entry: PageEntry): BoxRect {
  return { id: entry.id, posX: entry.posX, posY: entry.posY, width: entry.width, height: entry.height };
}

/** Ids fully inside the band. Shift unions with the current selection. */
export function selectIdsFromBand(
  contents: PageEntry[],
  band: { posX: number; posY: number; width: number; height: number },
  current: string[],
  shift: boolean,
): string[] {
  const inside = boxesInside(band, contents.map(asBox));
  if (!shift) return inside;
  const seen = new Set(current);
  const next = [...current];
  for (const id of inside) {
    if (seen.has(id)) continue;
    seen.add(id);
    next.push(id);
  }
  return next;
}

/**
 * Select box. A finished drag selects every entry fully inside the band.
 * Shift adds to the current selection. An empty band clears it.
 */
export const marqueeTool: SheetTool & { id: 'marquee' } = {
  id: 'marquee',
  label: 'Select box',
  icon: 'fa-solid fa-crop',
  position: 20,
  key: 'm',
  creates: false,
  cursor: null,
  gesture: 'drag',
  onGesture(input: GestureInput, host: ToolHost): ToolEffect {
    if (!input.rect) return { kind: 'none' };
    const page = host.page();
    if (!page) return { kind: 'none' };
    const ids = selectIdsFromBand(page.contents, input.rect, host.selection?.() ?? [], input.shift);
    return { kind: 'select-entries', ids };
  },
};
