import type { DrawnRect, SheetPoint } from '../draw-tool';
import type { PageDocument, PageEntry } from '../page-view';
import type { TextStyleMemory } from '../text-style';

export type ToolCursor = 'crosshair' | 'text' | null;

export type ToolGesture = 'canvas' | 'click' | 'drag';

/** What the pointer session hands the active tool on pointer up. */
export interface GestureInput {
  click: SheetPoint;
  rect: DrawnRect | null;
  shift: boolean;
  /** Frame client coordinates. Text places the caret with these. */
  client: { x: number; y: number };
  target: Element | null;
  /** Resolved by the editor from the live page before `onGesture`. */
  entry: PageEntry | null;
}

export type ToolEffect =
  | { kind: 'none' }
  | { kind: 'edit-text'; id: string; client: { x: number; y: number } }
  | { kind: 'append'; run(page: PageDocument): PageEntry }
  /** Eyedropper: write foreground memory only. The page entry is unchanged. */
  | { kind: 'set-foreground'; hex: string }
  /** Select box: canvas selection becomes these ids (empty clears). */
  | { kind: 'select-entries'; ids: string[] };

/**
 * What `onPointerDown` may read. Append, style, and measure stay on ToolHost /
 * onGesture so a canvas stub cannot pretend to be the full host.
 */
export interface PointerHost {
  /** The box currently being typed, or null. */
  editingElement(): HTMLElement | null;
}

/** The door from a tool into the editor. Tools do not import the components. */
export interface ToolHost extends PointerHost {
  page(): PageDocument | null;
  style(): TextStyleMemory;
  measureLine(font: string, sizeMm: number, bold: boolean, italic: boolean): (line: string) => number;
  /** True for the click that just closed an edit. */
  placeGuard(): boolean;
  /** Open the caret in that box. The text tool is the caller. */
  beginTextEdit(id: string, client: { x: number; y: number }): void;
  /**
   * `#RRGGBB` from the painted picture of that entry at a sheet point.
   * Null when the pixel is transparent, missing, or not a picture.
   */
  samplePicture?(id: string, point: SheetPoint): string | null;
  /** Current selected entry ids. Select box unions with these when Shift is held. */
  selection?(): string[];
}

/** The page entry under a canvas target, by `data-yaml-id`. */
export function entryFromTarget(page: PageDocument | null | undefined, target: Element | null): PageEntry | null {
  if (!page || !(target instanceof Element)) return null;
  const host = target.closest('[data-yaml-id]');
  if (!(host instanceof HTMLElement)) return null;
  const id = host.getAttribute('data-yaml-id');
  if (!id) return null;
  return page.contents.find((entry) => entry.id === id) ?? null;
}

export interface SheetTool {
  id: string;
  label: string;
  /** Font Awesome classes, as the rail draws them. */
  icon: string;
  /** Top of the rail is the smaller number. */
  position: number;
  /** Bare key, no modifier. Escape stays Select and is not a tool key. */
  key?: string;
  /** A read-only sheet disables these buttons. */
  creates: boolean;
  cursor: ToolCursor;
  gesture: ToolGesture;
  /** Drag only. Called after the 2 mm check. */
  adjustRect?(rect: DrawnRect, shift: boolean): DrawnRect;
  onGesture(input: GestureInput, host: ToolHost): ToolEffect;
  /** `'yield'` leaves the pointer to the browser. Omitted means `'take'`. */
  onPointerDown?(event: PointerEvent, host: PointerHost): 'yield' | 'take';
}
