// Template language shared by the code and visual modes of the editor
// (docs/TZ_print_templates.md, разделы 6.3–6.4): what a palette item turns
// into in the Django template, and how "smart" blocks are marked so a
// visual-mode re-import recognises them (`data-var`, `data-code`,
// `data-file`). The renderer (backend/registration/printing/) knows nothing
// about these attributes — they are plain HTML to it.

import { DemoContext, DemoField } from '../../core/models/print.models';
import { findContactFieldKeys, type FormlyFieldLike } from '../../formly/formly-contact-fields';

export type VarCase = 'none' | 'upper' | 'title';

export interface VarOptions {
  prefix?: string;
  suffix?: string;
  case?: VarCase;
  /** multi-select answers are lists — joined with this (раздел 6.3). */
  join?: string;
  default?: string;
}

export interface PaletteItem {
  /** Django expression without braces, e.g. `form_data.field_12`. */
  expr: string;
  label: string;
  hint?: string;
  kind: DemoField['kind'] | 'ticket' | 'composite';
  sample: string;
  isList: boolean;
}

export interface PaletteGroup {
  title: string;
  items: PaletteItem[];
}

/** `{{ expr|filters }}` for the given options. Only Django builtins
 * (`upper`, `title`, `join`, `default`) — the node's restricted engine
 * forbids `{% load %}`, so nothing else is reachable anyway. */
export function variableExpression(expr: string, options: VarOptions = {}, isList = false): string {
  let out = expr;
  if (isList) {
    out += `|join:${JSON.stringify(options.join ?? ', ')}`;
  }
  if (options.default) {
    out += `|default:${JSON.stringify(options.default)}`;
  }
  if (options.case === 'upper') {
    out += '|upper';
  } else if (options.case === 'title') {
    out += '|title';
  }
  return `{{ ${out} }}`;
}

function escapeAttr(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/\{/g, '&#123;');
}

/** A standalone variable block — the thing a palette drop creates. The
 * `data-*` attributes carry the options so the visual editor can rebuild
 * its traits from exported HTML; the text node is the real expression. */
export function variableBlockHtml(item: PaletteItem, options: VarOptions = {}, style = ''): string {
  const attrs = [`data-var="${escapeAttr(item.expr)}"`];
  if (item.isList) {
    attrs.push('data-list="1"');
  }
  if (options.prefix) attrs.push(`data-prefix="${escapeAttr(options.prefix)}"`);
  if (options.suffix) attrs.push(`data-suffix="${escapeAttr(options.suffix)}"`);
  if (options.case && options.case !== 'none') attrs.push(`data-case="${options.case}"`);
  if (options.join !== undefined) attrs.push(`data-join="${escapeAttr(options.join)}"`);
  if (options.default) attrs.push(`data-default="${escapeAttr(options.default)}"`);
  const text = `${options.prefix ?? ''}${variableExpression(item.expr, options, item.isList)}${options.suffix ?? ''}`;
  const styleAttr = style ? ` style="${escapeAttr(style)}"` : '';
  return `<div class="tpl-var" ${attrs.join(' ')}${styleAttr}>${text}</div>`;
}

export type CodeKind = 'barcode' | 'qrcode' | 'vcard';

export const CODE_VARIABLES: Record<CodeKind, { svg: string; png: string; label: string }> = {
  barcode: { svg: 'barcode_svg', png: 'barcode', label: 'Штрихкод EAN-13' },
  qrcode: { svg: 'qrcode_svg', png: 'qrcode', label: 'QR-код билета' },
  vcard: { svg: 'vcard_qrcode_svg', png: 'vcard_qrcode', label: 'QR vCard' },
};

export const MM_PER_DOT_203 = 25.4 / 203;

/** Block size that puts a whole number of printer dots per module
 * (раздел 6.4): EAN-13 is 117 modules wide incl. quiet zones; a QR of the
 * demo code is `qrModules` per side incl. the 4-module border. */
export function codeWidthMm(kind: CodeKind, dotsPerModule: number, dpi: number, qrModules: number): number {
  const mmPerDot = 25.4 / dpi;
  const modules = kind === 'barcode' ? 117 : qrModules;
  return Math.round(modules * dotsPerModule * mmPerDot * 1000) / 1000;
}

/** Module widths (printer dots) a code may be printed at: below the first
 * value scanners fail, above the last the block no longer fits a 100 mm
 * label. vCard QR has no fixed module count, so it is sized in mm. */
export const CODE_DOTS: Record<Exclude<CodeKind, 'vcard'>, number[]> = {
  barcode: [2, 3, 4, 5],
  qrcode: [3, 4, 5, 6],
};

/** The nearest width that keeps a whole number of dots per module — what a
 * free resize of a barcode/QR block snaps to (раздел 6.4). */
export function snapCodeWidthMm(
  kind: Exclude<CodeKind, 'vcard'>,
  widthMm: number,
  dpi: number,
  qrModules: number
): { widthMm: number; dots: number } {
  let best = { widthMm: 0, dots: 0 };
  for (const dots of CODE_DOTS[kind]) {
    const candidate = codeWidthMm(kind, dots, dpi, qrModules);
    if (!best.dots || Math.abs(candidate - widthMm) < Math.abs(best.widthMm - widthMm)) {
      best = { widthMm: candidate, dots };
    }
  }
  return best;
}

export function codeBlockHtml(
  kind: CodeKind,
  options: { vector?: boolean; widthMm: number; heightMm?: number; leftMm?: number; topMm?: number }
): string {
  const variable = options.vector === false ? CODE_VARIABLES[kind].png : CODE_VARIABLES[kind].svg;
  const styles = [
    `position:absolute`,
    `left:${options.leftMm ?? 5}mm`,
    `top:${options.topMm ?? 5}mm`,
    `width:${options.widthMm}mm`,
  ];
  if (kind === 'barcode') {
    styles.push(`height:${options.heightMm ?? 15}mm`);
  }
  styles.push('image-rendering:pixelated');
  return `<img class="tpl-code" data-code="${kind}" src="{{ ${variable} }}" style="${styles.join(';')}">`;
}

export function fileImageHtml(name: string): string {
  return `<img class="tpl-file" data-file="${escapeAttr(name)}" src="{{ files.${name} }}" style="position:absolute;left:5mm;top:5mm;width:30mm">`;
}

export const TICKET_ITEMS: PaletteItem[] = [
  { expr: 'ticket.number', label: 'Номер регистрации', kind: 'ticket', sample: 'A-00042', isList: false },
  { expr: 'ticket.ticket_code.code', label: 'Код билета', kind: 'ticket', sample: '2000000000008', isList: false },
  { expr: 'ticket.ticket_type.name', label: 'Вид билета', kind: 'ticket', sample: 'Standard', isList: false },
];

/** Builds the palette from the demo context (field list with samples) and
 * the Formly schema (for the composite Фамилия+Имя items, detected the
 * same way the registration list does). */
export function buildPalette(demo: DemoContext | null, schema: FormlyFieldLike[] | null): PaletteGroup[] {
  const groups: PaletteGroup[] = [];
  if (demo) {
    const bySection = new Map<string, PaletteItem[]>();
    for (const field of demo.fields) {
      const list = bySection.get(field.section) ?? [];
      list.push({
        expr: `form_data.${field.key}`,
        label: field.label || field.key,
        hint: kindLabel(field.kind),
        kind: field.kind,
        sample: Array.isArray(field.sample) ? field.sample.join(', ') : String(field.sample),
        isList: field.kind === 'multichoice',
      });
      bySection.set(field.section, list);
    }
    for (const [section, items] of bySection) {
      groups.push({ title: section ? `Анкета — ${section}` : 'Анкета', items });
    }

    if (schema) {
      const keys = findContactFieldKeys(schema);
      const sample = (key?: string) => {
        const value = key ? demo.form_data[key] : '';
        return Array.isArray(value) ? value.join(' ') : String(value ?? '');
      };
      if (keys.lastNameKey && keys.firstNameKey) {
        groups.push({
          title: 'Анкета — составные',
          items: [
            {
              expr: `form_data.${keys.lastNameKey} }} {{ form_data.${keys.firstNameKey}`,
              label: 'Фамилия Имя',
              kind: 'composite',
              sample: `${sample(keys.lastNameKey)} ${sample(keys.firstNameKey)}`,
              isList: false,
            },
            {
              expr: `form_data.${keys.firstNameKey} }} {{ form_data.${keys.lastNameKey}`,
              label: 'Имя Фамилия',
              kind: 'composite',
              sample: `${sample(keys.firstNameKey)} ${sample(keys.lastNameKey)}`,
              isList: false,
            },
          ],
        });
      }
    }
  }
  groups.push({
    title: 'Билет',
    items: TICKET_ITEMS.map((item) => ({
      ...item,
      sample:
        item.expr === 'ticket.number'
          ? demo?.ticket.number ?? item.sample
          : item.expr === 'ticket.ticket_code.code'
            ? demo?.ticket.code ?? item.sample
            : demo?.ticket.ticket_type_name ?? item.sample,
    })),
  });
  return groups;
}

export function kindLabel(kind: DemoField['kind']): string {
  return (
    {
      text: 'текст',
      textarea: 'текст',
      choice: 'выбор',
      multichoice: 'несколько вариантов',
      boolean: 'да/нет',
      phone: 'телефон',
      email: 'e-mail',
      date: 'дата',
    } as Record<string, string>
  )[kind] ?? kind;
}

/** Demo value for an expression, for the visual canvas. Composite items
 * carry two expressions glued with `}} {{`; evaluate each. */
export function demoValueFor(expr: string, demo: DemoContext | null): string {
  if (!demo) {
    return expr;
  }
  return expr
    .split('}} {{')
    .map((part) => {
      const key = part.trim();
      if (key.startsWith('form_data.')) {
        const value = demo.form_data[key.slice('form_data.'.length)];
        return Array.isArray(value) ? value.join(', ') : String(value ?? '');
      }
      if (key === 'ticket.number') return demo.ticket.number;
      if (key === 'ticket.ticket_code.code') return demo.ticket.code;
      if (key === 'ticket.ticket_type.name') return demo.ticket.ticket_type_name;
      return `{{ ${key} }}`;
    })
    .join(' ');
}

/** `{% load static %}` (cloud templates carry it but use no tag from it) is
 * dropped: the node's engine forbids `{% load %}` altogether. */
const LOAD_STATIC_RE = /^[ \t]*\{%-?\s*load\s+static\s*-?%\}[ \t]*\r?\n?/gim;

export function stripHarmlessTags(html: string): string {
  return html.replace(LOAD_STATIC_RE, '');
}

/** Hand-written `src={{ barcode }}` (no quotes) is not valid HTML for the
 * parser: quote such values so they survive as one attribute. */
function quoteBareVariables(html: string): string {
  return html.replace(/=\s*(\{\{[^}]*\}\})/g, '="$1"');
}

/** Why the visual mode can't open this html, or null when it can. */
export function visualModeBlocker(html: string): string | null {
  if (/\{%/.test(stripHarmlessTags(html))) {
    return 'В макете есть логика шаблона ({% if %}, {% for %}) — визуальный редактор её сломает. Редактируйте кодом.';
  }
  if (/<script[\s>]/i.test(html)) {
    return 'В макете есть <script> — такие макеты редактируются только кодом.';
  }
  if (/<link[\s>]/i.test(html)) {
    return 'В макете есть внешний <link> — такие макеты редактируются только кодом.';
  }
  return null;
}

/** Splits a stored document into the CSS (all <style> blocks) and the body
 * markup the visual editor works with. */
export function splitDocument(html: string): { css: string; body: string } {
  const doc = new DOMParser().parseFromString(quoteBareVariables(stripHarmlessTags(html)), 'text/html');
  const css = Array.from(doc.querySelectorAll('style'))
    .map((s) => s.textContent ?? '')
    .join('\n');
  doc.querySelectorAll('style').forEach((s) => s.remove());
  return { css, body: doc.body.innerHTML };
}

export function joinDocument(css: string, body: string): string {
  return `<!doctype html>\n<html>\n<head>\n<meta charset="utf-8">\n<style>\n${css}\n</style>\n</head>\n<body>\n${body}\n</body>\n</html>\n`;
}
