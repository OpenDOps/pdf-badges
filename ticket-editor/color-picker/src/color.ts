export interface Hsv {
  h: number;
  s: number;
  v: number;
}

const SIZE = 200;

export function controlSize(size?: number): number {
  return size && size > 0 ? size : SIZE;
}

export function hexToHsv(hex: string): Hsv {
  const match = /^#([0-9a-fA-F]{6})$/.exec(hex.trim());
  const raw = match ? match[1] : '000000';
  const r = parseInt(raw.slice(0, 2), 16) / 255;
  const g = parseInt(raw.slice(2, 4), 16) / 255;
  const b = parseInt(raw.slice(4, 6), 16) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  let h = 0;
  if (d !== 0) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
    if (h < 0) h += 360;
  }
  return { h, s: max === 0 ? 0 : d / max, v: max };
}

export function hsvToHex(hsv: Hsv): string {
  const h = ((hsv.h % 360) + 360) % 360;
  const s = clamp01(hsv.s);
  const v = clamp01(hsv.v);
  const c = v * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = v - c;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return `#${byte(r + m)}${byte(g + m)}${byte(b + m)}`;
}

/** Hue 0 sits at the right, matching `conic-gradient(from 90deg, ...)`. */
export const WHEEL_GRADIENT = 'conic-gradient(from 90deg, red, yellow, lime, aqua, blue, magenta, red)';

export function boxPoint(color: string, size?: number): { x: number; y: number } {
  const hsv = hexToHsv(color);
  const width = controlSize(size);
  return { x: hsv.s * width, y: (1 - hsv.v) * width };
}

export function hexFromBox(color: string, x: number, y: number, size?: number): string {
  const width = controlSize(size);
  const hsv = hexToHsv(color);
  return hsvToHex({ h: hsv.h, s: clamp01(x / width), v: clamp01(1 - y / width) });
}

export function wheelPoint(color: string, size?: number): { x: number; y: number } {
  const hsv = hexToHsv(color);
  const width = controlSize(size);
  const radius = width / 2;
  const angle = (hsv.h * Math.PI) / 180;
  return {
    x: radius + Math.cos(angle) * hsv.s * radius,
    y: radius + Math.sin(angle) * hsv.s * radius,
  };
}

export function hexFromWheel(color: string, x: number, y: number, size?: number): string {
  const width = controlSize(size);
  const radius = width / 2;
  const dx = x - radius;
  const dy = y - radius;
  const dist = Math.min(Math.hypot(dx, dy), radius);
  let h = (Math.atan2(dy, dx) * 180) / Math.PI;
  if (h < 0) h += 360;
  const hsv = hexToHsv(color);
  return hsvToHex({ h, s: radius === 0 ? 0 : dist / radius, v: hsv.v });
}

export function sliderPoint(color: string, channel: 'hue' | 'value', size?: number): { x: number } {
  const hsv = hexToHsv(color);
  const width = controlSize(size);
  const t = channel === 'hue' ? hsv.h / 360 : hsv.v;
  return { x: t * width };
}

export function hexFromSlider(color: string, channel: 'hue' | 'value', x: number, size?: number): string {
  const width = controlSize(size);
  const hsv = hexToHsv(color);
  const t = clamp01(x / width);
  if (channel === 'hue') return hsvToHex({ h: t * 360, s: hsv.s, v: hsv.v });
  return hsvToHex({ h: hsv.h, s: hsv.s, v: t });
}

function clamp01(n: number): number {
  if (!Number.isFinite(n)) return 0;
  return Math.min(1, Math.max(0, n));
}

function byte(channel: number): string {
  return Math.round(clamp01(channel) * 255).toString(16).padStart(2, '0').toUpperCase();
}
