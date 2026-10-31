/** The last font the operator picked. The next text box reuses it, the same way
 * the color picker reuses the last foreground or background. */
export interface TextStyleMemory {
  /** Resource key, such as `text`. */
  font: string;
  fontSizePt: number;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  /** Next text color. `#RRGGBB`. */
  foreground: string;
  /** Next box fill. `#RRGGBB`. */
  background: string;
}

const STORAGE_KEY = 'ticket-editor:text-style';

export interface StyleStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function defaultTextStyle(): TextStyleMemory {
  return { font: 'text', fontSizePt: 12, bold: false, italic: false, underline: false, foreground: '#000000', background: '#FFFFFF' };
}

/** `#RRGGBB` only. Anything else is refused. */
export function normalizeHex(value: string): string | null {
  const match = /^#([0-9a-fA-F]{6})$/.exec(value.trim());
  return match ? `#${match[1].toUpperCase()}` : null;
}

export function createTextStyleCache(storage: StyleStorage): {
  read(): TextStyleMemory;
  remember(style: TextStyleMemory): void;
} {
  let memory = readStored(storage);
  return {
    read: () => ({ ...memory }),
    remember(style: TextStyleMemory) {
      memory = sanitizeStyle(style);
      try {
        storage.setItem(STORAGE_KEY, JSON.stringify(memory));
      } catch {
        // A blocked store still keeps the last choice for this session.
      }
    },
  };
}

function browserStorage(): StyleStorage {
  try {
    if (typeof localStorage !== 'undefined') return localStorage;
  } catch {
    // Private mode.
  }
  const bag = new Map<string, string>();
  return {
    getItem: (key) => bag.get(key) ?? null,
    setItem: (key, value) => {
      bag.set(key, value);
    },
  };
}

export const textStyleCache = createTextStyleCache(browserStorage());

function readStored(storage: StyleStorage): TextStyleMemory {
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return defaultTextStyle();
    return sanitizeStyle(JSON.parse(raw) as Partial<TextStyleMemory>);
  } catch {
    return defaultTextStyle();
  }
}

function sanitizeStyle(value: Partial<TextStyleMemory> | null | undefined): TextStyleMemory {
  const font = typeof value?.font === 'string' && value.font.trim() ? value.font.trim() : 'text';
  const pt = Number(value?.fontSizePt);
  return {
    font,
    fontSizePt: Number.isFinite(pt) && pt > 0 ? Math.round(pt * 100) / 100 : 12,
    bold: !!value?.bold,
    italic: !!value?.italic,
    underline: !!value?.underline,
    foreground: normalizeHex(value?.foreground ?? '') ?? '#000000',
    background: normalizeHex(value?.background ?? '') ?? '#FFFFFF',
  };
}

/** Glyph width of one line, in millimetres. The canvas document supplies the face. */
export function measureLineMm(
  line: string,
  family: string,
  fontSizeMm: number,
  bold: boolean,
  italic: boolean,
  doc: Document | null = typeof document === 'undefined' ? null : document,
): number {
  if (!line || !doc) return 0;
  const canvas = doc.createElement('canvas');
  const ctx = canvas.getContext('2d');
  if (!ctx) return 0;
  const px = (fontSizeMm * 96) / 25.4;
  const style = italic ? 'italic' : 'normal';
  const weight = bold ? 'bold' : 'normal';
  const safe = family.replace(/["\\]/g, '');
  ctx.font = `${style} ${weight} ${px}px "${safe}"`;
  return (ctx.measureText(line).width * 25.4) / 96;
}

/** The frame that holds `plain`, before padding and the 0.5 mm grid. */
export function textFrameMm(
  plain: string,
  fontSizeMm: number,
  lineHeightMm: number,
  leadingMm: number,
  letterSpacingMm: number,
  measureLine: (line: string) => number,
): { width: number; height: number } {
  const lines = plain.split('\n');
  const used = lines.length > 0 ? lines : [''];
  let width = 0;
  for (const line of used) {
    const gaps = Math.max(0, Array.from(line).length - 1) * letterSpacingMm;
    width = Math.max(width, measureLine(line) + gaps);
  }
  if (width <= 0) width = fontSizeMm;
  const height = used.length * lineHeightMm + Math.max(0, used.length - 1) * leadingMm;
  return { width, height };
}

/** Smallest 0.5 mm step that still contains `mm`. */
export function ceilHalfMm(mm: number): number {
  return Math.ceil(mm / 0.5 - 1e-9) * 0.5;
}

/** Stored `<br>` is a line in the editor. Enter writes `<br>` back. */
export function breaksToLines(source: string): string {
  return source.replace(/<br\s*\/?>/gi, '\n');
}

export function linesToBreaks(typed: string): string {
  return typed.replace(/\r\n|\r|\n/g, '<br>');
}
