/** Screen pixels in one millimetre. The canvas uses this same value. */
export const PX_PER_MM = 96 / 25.4;

const GRID_MM = 0.5;
const MIN_SIDE_MM = 2;

export interface SheetPoint {
  x: number;
  y: number;
}

export interface MillimetreRect {
  posX: number;
  posY: number;
  width: number;
  height: number;
}

/** A drag rectangle. `anchorX` and `anchorY` are the corner the drag started at. */
export interface DrawnRect extends MillimetreRect {
  anchorX: number;
  anchorY: number;
}

export interface BoxRect extends MillimetreRect {
  id: string;
}

/** Nearest 0.5 mm. The canvas grid uses this same step. */
export function snapMm(mm: number): number {
  return Math.round(mm / GRID_MM) * GRID_MM;
}

/** Both corners snap to 0.5 mm. A side under 2 mm is not a box. */
export function normalizeRect(anchor: SheetPoint, pointer: SheetPoint): DrawnRect | null {
  const anchorX = snapMm(anchor.x);
  const anchorY = snapMm(anchor.y);
  const pointerX = snapMm(pointer.x);
  const pointerY = snapMm(pointer.y);
  const width = Math.abs(pointerX - anchorX);
  const height = Math.abs(pointerY - anchorY);
  if (width < MIN_SIDE_MM || height < MIN_SIDE_MM) return null;
  return {
    posX: Math.min(anchorX, pointerX),
    posY: Math.min(anchorY, pointerY),
    width,
    height,
    anchorX,
    anchorY,
  };
}

/** QR with Shift is a square from the anchor. A barcode keeps the drag, Shift included. */
export function placedCodeRect(rect: DrawnRect, kind: 'qr' | 'ean13', shift: boolean): DrawnRect {
  return kind === 'qr' && shift ? lockSquare(rect) : rect;
}

/** The larger side becomes both sides. The corner the drag started at stays put. */
export function lockSquare(rect: DrawnRect): DrawnRect {
  const side = Math.max(rect.width, rect.height);
  const posX = rect.anchorX > rect.posX ? rect.anchorX - side : rect.anchorX;
  const posY = rect.anchorY > rect.posY ? rect.anchorY - side : rect.anchorY;
  return { ...rect, posX, posY, width: side, height: side };
}

/** Ids whose boxes lie fully inside the band. A box that only crosses an edge is left out. */
export function boxesInside(band: MillimetreRect, entries: BoxRect[]): string[] {
  const right = band.posX + band.width;
  const bottom = band.posY + band.height;
  return entries
    .filter(
      (entry) =>
        entry.posX >= band.posX &&
        entry.posY >= band.posY &&
        entry.posX + entry.width <= right &&
        entry.posY + entry.height <= bottom,
    )
    .map((entry) => entry.id);
}
