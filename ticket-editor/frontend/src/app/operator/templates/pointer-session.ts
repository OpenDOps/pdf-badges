import { PX_PER_MM, normalizeRect, snapMm, type DrawnRect, type SheetPoint } from './draw-tool';
import type { ToolCursor, ToolGesture } from './tools/sheet-tool';

/** What the active tool asks the pointer session to do. */
export interface DrawMode {
  gesture: ToolGesture;
  cursor: ToolCursor;
  /** Omitted means take the pointer. `'yield'` leaves it to the browser and does not finish. */
  onPointerDown?: (event: PointerEvent) => 'yield' | 'take';
}

/** Frame client coordinates to millimetres on the sheet. */
export function sheetPoint(clientX: number, clientY: number, origin: SheetPoint): SheetPoint {
  return { x: clientX / PX_PER_MM - origin.x, y: clientY / PX_PER_MM - origin.y };
}

const CURSOR_CSS = [
  'html[data-draw-cursor="crosshair"], html[data-draw-cursor="crosshair"] *, html[data-draw-cursor="crosshair"] .gjs-is__grab, html[data-draw-cursor="crosshair"] .gjs-is__grab * { cursor: crosshair !important; }',
  'html[data-draw-cursor="text"], html[data-draw-cursor="text"] *, html[data-draw-cursor="text"] .gjs-is__grab, html[data-draw-cursor="text"] .gjs-is__grab * { cursor: text !important; }',
].join('\n');

/**
 * Rubber band on the canvas document. A canvas gesture installs nothing, so
 * GrapesJS keeps the pointer. Click and drag take it. Drag draws the band and
 * drops a side under 2 mm. Click always finishes, with no rectangle.
 * Nothing is appended here.
 */
export class DrawPointerSession {
  private anchor: SheetPoint | null = null;
  private attached = false;
  private band: HTMLDivElement | null = null;
  private mode: DrawMode = { gesture: 'canvas', cursor: null };
  private readonly onPointerDown = (event: PointerEvent): void => {
    if (event.button !== 0) return;
    // `'yield'` is the caret inside a text box. Grapes must not drag it, and this gesture does not finish.
    if ((this.mode.onPointerDown?.(event) ?? 'take') === 'yield') {
      event.stopPropagation();
      return;
    }
    this.hold(event);
    this.anchor = this.point(event);
    const root = this.doc.documentElement;
    if (typeof root.setPointerCapture === 'function') {
      try {
        root.setPointerCapture(event.pointerId);
      } catch {
        // The pointer is not active in this document.
      }
    }
  };
  private readonly onPointerMove = (event: PointerEvent): void => {
    if (!this.anchor) return;
    this.hold(event);
    if (this.mode.gesture === 'drag') this.showBand(this.anchor, this.point(event));
  };
  private readonly onPointerUp = (event: PointerEvent): void => {
    if (!this.anchor) return;
    this.hold(event);
    const anchor = this.anchor;
    this.anchor = null;
    this.clearBand();
    let under: Element | null = null;
    try {
      under = this.doc.elementFromPoint(event.clientX, event.clientY);
    } catch {
      under = null;
    }
    const target = under ?? (event.target instanceof Element ? event.target : null);
    const click = { x: snapMm(anchor.x), y: snapMm(anchor.y) };
    const rect = this.mode.gesture === 'drag' ? normalizeRect(anchor, this.point(event)) : null;
    this.finish(rect, click, event.shiftKey, target, { x: event.clientX, y: event.clientY });
  };
  private readonly onPointerCancel = (): void => {
    this.anchor = null;
    this.clearBand();
  };

  constructor(
    private readonly doc: Document,
    private readonly origin: () => SheetPoint,
    private readonly finish: (
      rect: DrawnRect | null,
      click: SheetPoint,
      shift: boolean,
      target: Element | null,
      point: { x: number; y: number },
    ) => void = () => undefined,
  ) {}

  /** True while pointer listeners are on the canvas document. */
  get listening(): boolean {
    return this.attached;
  }

  /** A canvas gesture removes the listeners. Click and drag install them. */
  setTool(mode: DrawMode): void {
    this.mode = mode;
    this.setCursor(mode.cursor);
    if (mode.gesture !== 'drag') this.clearBand();
    if (mode.gesture === 'canvas') this.detach();
    else this.attach();
  }

  dispose(): void {
    this.setCursor(null);
    this.detach();
  }

  private setCursor(cursor: 'crosshair' | 'text' | null): void {
    const root = this.doc.documentElement;
    const current = this.doc.querySelector('style[data-draw-cursor]');
    if (!cursor) {
      root.removeAttribute('data-draw-cursor');
      current?.remove();
      return;
    }
    root.setAttribute('data-draw-cursor', cursor);
    const style = current ?? this.doc.createElement('style');
    style.setAttribute('data-draw-cursor', '');
    style.textContent = CURSOR_CSS;
    if (!current) (this.doc.head ?? root).appendChild(style);
  }

  private attach(): void {
    if (this.attached) return;
    this.attached = true;
    this.doc.addEventListener('pointerdown', this.onPointerDown, true);
    this.doc.addEventListener('pointermove', this.onPointerMove, true);
    this.doc.addEventListener('pointerup', this.onPointerUp, true);
    this.doc.addEventListener('pointercancel', this.onPointerCancel, true);
  }

  private detach(): void {
    if (!this.attached) return;
    this.attached = false;
    this.doc.removeEventListener('pointerdown', this.onPointerDown, true);
    this.doc.removeEventListener('pointermove', this.onPointerMove, true);
    this.doc.removeEventListener('pointerup', this.onPointerUp, true);
    this.doc.removeEventListener('pointercancel', this.onPointerCancel, true);
    this.anchor = null;
    this.clearBand();
  }

  private hold(event: PointerEvent): void {
    event.preventDefault();
    event.stopPropagation();
  }

  private point(event: PointerEvent): SheetPoint {
    return sheetPoint(event.clientX, event.clientY, this.origin());
  }

  private showBand(anchor: SheetPoint, pointer: SheetPoint): void {
    const band = this.band ?? this.createBand();
    const origin = this.origin();
    const posX = Math.min(anchor.x, pointer.x);
    const posY = Math.min(anchor.y, pointer.y);
    band.style.left = `${origin.x + posX}mm`;
    band.style.top = `${origin.y + posY}mm`;
    band.style.width = `${Math.abs(pointer.x - anchor.x)}mm`;
    band.style.height = `${Math.abs(pointer.y - anchor.y)}mm`;
  }

  private createBand(): HTMLDivElement {
    const band = this.doc.createElement('div');
    band.setAttribute('data-draw-band', '');
    band.style.position = 'absolute';
    band.style.boxSizing = 'border-box';
    band.style.border = '1px dashed #296df2';
    band.style.background = 'rgba(41, 109, 242, 0.12)';
    band.style.pointerEvents = 'none';
    band.style.zIndex = '1000';
    this.doc.documentElement.appendChild(band);
    this.band = band;
    return band;
  }

  private clearBand(): void {
    this.band?.remove();
    this.band = null;
  }
}
