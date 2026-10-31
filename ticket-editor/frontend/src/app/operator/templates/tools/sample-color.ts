import type { SheetPoint } from '../draw-tool';
import type { PageEntry } from '../page-view';
import { normalizeHex } from '../text-style';

/** Topmost entry whose rectangle contains the sheet point. Later contents paint above earlier ones. */
export function entryAtPoint(contents: PageEntry[], point: SheetPoint): PageEntry | null {
  for (let index = contents.length - 1; index >= 0; index--) {
    const entry = contents[index];
    if (
      point.x >= entry.posX &&
      point.x <= entry.posX + entry.width &&
      point.y >= entry.posY &&
      point.y <= entry.posY + entry.height
    ) {
      return entry;
    }
  }
  return null;
}

/**
 * Text and rectangle boxes contribute `text.color`, or `#000000` when absent.
 * Image, code, and photo entries return null here; the host samples the picture.
 */
export function sampleTextForeground(entry: PageEntry): string | null {
  if (!entry.text) return null;
  return normalizeHex(entry.text.color ?? '') ?? '#000000';
}

/** Drop alpha. A fully transparent pixel is not a color. */
export function rgbaToHex(r: number, g: number, b: number, a: number): string | null {
  if (a === 0) return null;
  const hex = (n: number) => n.toString(16).padStart(2, '0').toUpperCase();
  return `#${hex(r)}${hex(g)}${hex(b)}`;
}

/** Sheet point → pixel indices inside the painted picture, or null when outside the box. */
export function picturePixel(
  entry: PageEntry,
  point: SheetPoint,
  naturalWidth: number,
  naturalHeight: number,
): { x: number; y: number } | null {
  if (entry.width <= 0 || entry.height <= 0 || naturalWidth <= 0 || naturalHeight <= 0) return null;
  const localX = (point.x - entry.posX) / entry.width;
  const localY = (point.y - entry.posY) / entry.height;
  if (localX < 0 || localX > 1 || localY < 0 || localY > 1) return null;
  return {
    x: Math.min(naturalWidth - 1, Math.max(0, Math.floor(localX * naturalWidth))),
    y: Math.min(naturalHeight - 1, Math.max(0, Math.floor(localY * naturalHeight))),
  };
}
