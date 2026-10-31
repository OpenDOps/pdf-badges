import { demoContext } from '../../../../../src/demo-context';
import { snapMm } from './draw-tool';
import { joinDocument } from './template-snippets';
import { ceilHalfMm, measureLineMm, normalizeHex, textFrameMm, textStyleCache, type TextStyleMemory } from './text-style';

export interface PageSize {
  width: number;
  height: number;
  units?: string;
  bleeds?: { top: number; right: number; bottom: number; left: number };
  background?: string;
}

export interface PageFont {
  font_family?: string;
}

export interface PageTextStyle {
  font: string;
  font_size: number;
  leading: number;
  line_height: number;
  color?: string;
  letter_spacing?: number;
  font_style?: { bold?: boolean; italic?: boolean; underline?: boolean };
}

export interface PageEntry {
  id: string;
  posX: number;
  posY: number;
  width: number;
  height: number;
  template?: boolean;
  preentered?: string | null;
  ifSt?: string | null;
  rotation?: number;
  background?: string;
  border?: { width: number; color: string };
  border_radius?: number;
  opacity?: number;
  scale_x?: number;
  scale_y?: number;
  skew_x?: number;
  skew_y?: number;
  auto_scale?: boolean;
  content?: string | null;
  alignment?: { horizontal?: string; vertical?: string };
  padding?: { top?: number; right?: number; bottom?: number; left?: number };
  text?: PageTextStyle;
  image?: { resource_name: string };
  barcode?: { type: string; preentered?: string | null };
  photo?: { field: string };
}

export interface PageDocument {
  page_size: PageSize;
  resources: {
    images?: Record<string, { name?: string; extracted_path?: string }>;
    fonts?: Record<string, PageFont>;
  };
  contents: PageEntry[];
}

const FLAT_PATHS: Record<string, string> = {
  number: 'ticket.number',
  code: 'ticket.ticket_code.code',
  ticket_type_name: 'ticket.ticket_type.name',
};

function escapeText(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function escapeAttr(value: string): string {
  return escapeText(value).replace(/"/g, '&quot;').replace(/\{/g, '&#123;').replace(/\}/g, '&#125;');
}

function cssNumber(value: number): string {
  const rounded = Math.round(value * 1000) / 1000;
  return String(rounded);
}

function editorPath(flat: string): string {
  if (FLAT_PATHS[flat]) return FLAT_PATHS[flat];
  if (/^field_\d+$/.test(flat)) return `form_data.${flat}`;
  return flat;
}

function pipeValue(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"')) return trimmed.slice(1, -1);
  return trimmed;
}

function frameStyles(entry: PageEntry): string[] {
  const styles = [
    'position:absolute',
    `left:${cssNumber(entry.posX)}mm`,
    `top:${cssNumber(entry.posY)}mm`,
    `width:${cssNumber(entry.width)}mm`,
    `height:${cssNumber(entry.height)}mm`,
  ];
  if (entry.background) styles.push(`background-color:${entry.background}`);
  if (entry.border) styles.push(`border:${cssNumber(entry.border.width)}mm solid ${entry.border.color}`);
  if (entry.border_radius) styles.push(`border-radius:${cssNumber(entry.border_radius)}mm`);
  if (entry.opacity !== undefined && entry.opacity !== 1) styles.push(`opacity:${cssNumber(entry.opacity)}`);
  if (entry.rotation === 90) styles.push('--rotation:90');
  if (entry.scale_x !== undefined && entry.scale_x !== 1) styles.push(`--scale-x:${cssNumber(entry.scale_x)}`);
  if (entry.scale_y !== undefined && entry.scale_y !== 1) styles.push(`--scale-y:${cssNumber(entry.scale_y)}`);
  if (entry.skew_x) styles.push(`--skew-x:${cssNumber(entry.skew_x)}`);
  if (entry.skew_y) styles.push(`--skew-y:${cssNumber(entry.skew_y)}`);
  if (entry.ifSt && !entryVisible(entry.ifSt, demoContext.form_data)) styles.push('display:none');
  const pad = entry.padding;
  if (pad && (pad.top || pad.right || pad.bottom || pad.left)) {
    styles.push(
      `padding:${cssNumber(pad.top ?? 0)}mm ${cssNumber(pad.right ?? 0)}mm ${cssNumber(pad.bottom ?? 0)}mm ${cssNumber(pad.left ?? 0)}mm`,
    );
  }
  return styles;
}

function frameAttrs(entry: PageEntry): string[] {
  const attrs: string[] = [];
  if (entry.ifSt) attrs.push(`data-if="${escapeAttr(entry.ifSt)}"`);
  if (entry.rotation) attrs.push(`data-rotation="${entry.rotation}"`);
  if (entry.scale_x !== undefined && entry.scale_x !== 1) attrs.push(`data-scale-x="${cssNumber(entry.scale_x)}"`);
  if (entry.scale_y !== undefined && entry.scale_y !== 1) attrs.push(`data-scale-y="${cssNumber(entry.scale_y)}"`);
  if (entry.skew_x) attrs.push(`data-skew-x="${cssNumber(entry.skew_x)}"`);
  if (entry.skew_y) attrs.push(`data-skew-y="${cssNumber(entry.skew_y)}"`);
  if (entry.auto_scale) attrs.push('data-auto-scale="true"');
  return attrs;
}

function textStyles(entry: PageEntry, page: PageDocument): string[] {
  const text = entry.text;
  if (!text) return [];
  const styles: string[] = [];
  const family = page.resources.fonts?.[text.font]?.font_family;
  if (family) styles.push(`font-family:'${escapeAttr(family)}'`);
  styles.push(`font-size:${cssNumber((text.font_size * 72) / 25.4)}pt`);
  if (!text.leading && text.font_size) styles.push(`line-height:${cssNumber(text.line_height / text.font_size)}`);
  const align = entry.alignment?.horizontal;
  if (align && align !== 'left') styles.push(`text-align:${align}`);
  const vertical = entry.alignment?.vertical;
  if (vertical === 'middle' || vertical === 'bottom') {
    styles.push('display:flex', `align-items:${vertical === 'middle' ? 'center' : 'flex-end'}`);
  }
  const style = text.font_style;
  if (style?.bold) styles.push('font-weight:bold');
  if (style?.italic) styles.push('font-style:italic');
  if (style?.underline) styles.push('text-decoration:underline');
  if (text.color && text.color.toLowerCase() !== '#000000') styles.push(`color:${text.color}`);
  if (text.letter_spacing) styles.push(`letter-spacing:${cssNumber(text.letter_spacing)}mm`);
  return styles;
}

function textAttrs(entry: PageEntry, source: string): string[] {
  const attrs = [
    `data-yaml-id="${escapeAttr(entry.id)}"`,
    `data-preentered="${escapeAttr(source)}"`,
    `data-font="${escapeAttr(entry.text?.font ?? 'text')}"`,
    `data-valign="${escapeAttr(entry.alignment?.vertical ?? 'top')}"`,
  ];
  attrs.push(`data-leading="${cssNumber(entry.text?.leading ?? 0)}"`);
  return attrs;
}

function mustacheParts(source: string): { prefix: string; name: string; pipes: string[]; suffix: string } | null {
  const found = source.match(/\{\{([^{}]*)\}\}/g);
  if (!found || found.length !== 1) return null;
  const match = source.match(/^(.*)\{\{([^{}]+)\}\}(.*)$/s);
  if (!match) return null;
  const [name, ...pipes] = match[2].split('|');
  return { prefix: match[1], name: name.trim(), pipes: pipes.map((pipe) => pipe.trim()), suffix: match[3] };
}

function variableAttrs(parts: { prefix: string; name: string; pipes: string[]; suffix: string }): string[] {
  const attrs = [`data-var="${escapeAttr(editorPath(parts.name))}"`];
  if (parts.prefix) attrs.push(`data-prefix="${escapeAttr(parts.prefix)}"`);
  if (parts.suffix) attrs.push(`data-suffix="${escapeAttr(parts.suffix)}"`);
  for (const pipe of parts.pipes) {
    if (pipe === 'upper' || pipe === 'title') attrs.push(`data-case="${pipe}"`);
    else if (pipe.startsWith('join:')) attrs.push(`data-join="${escapeAttr(pipeValue(pipe.slice(5)))}"`);
    else if (pipe.startsWith('default:')) attrs.push(`data-default="${escapeAttr(pipeValue(pipe.slice(8)))}"`);
  }
  return attrs;
}

function textBlock(entry: PageEntry, page: PageDocument): string {
  const source = entry.preentered ?? '';
  const parts = mustacheParts(source);
  const attrs = [...textAttrs(entry, source), ...(parts ? variableAttrs(parts) : ['data-text="1"']), ...frameAttrs(entry)];
  let style = [...frameStyles(entry), ...textStyles(entry, page)].join(';');
  if (entry.ifSt && !entryVisible(entry.ifSt, demoContext.form_data)) style += ';display:none';
  const kind = parts ? 'tpl-var' : 'tpl-text';
  const shown = parts ? source : visibleText(source);
  return `<div class="${kind}" ${attrs.join(' ')} style="${style}">${escapeText(shown)}</div>`;
}

function codeSrc(type: string): string {
  if (type === 'qr') return demoContext.codes.qrcode_svg;
  if (type === 'ean13') return demoContext.codes.barcode_svg;
  return '';
}

function codeKind(type: string): string {
  if (type === 'ean13') return 'barcode';
  if (type === 'qr') return 'qrcode';
  return type;
}

function imageBlock(entry: PageEntry): string {
  const name = entry.image?.resource_name ?? '';
  const attrs = [
    `class="tpl-file"`,
    `data-yaml-id="${escapeAttr(entry.id)}"`,
    `data-file="${escapeAttr(name)}"`,
    ...frameAttrs(entry),
    `src="{{ files.${name} }}"`,
    `style="${frameStyles(entry).join(';')}"`,
  ];
  return `<img ${attrs.join(' ')}>`;
}

function codeBlock(entry: PageEntry): string {
  const type = (entry.barcode?.type ?? '').toLowerCase();
  if (type === 'code128') return placeholder(entry);
  const attrs = [
    `class="tpl-code"`,
    `data-yaml-id="${escapeAttr(entry.id)}"`,
    `data-code="${codeKind(type)}"`,
    ...frameAttrs(entry),
    `src="${escapeAttr(codeSrc(type))}"`,
    `style="${[...frameStyles(entry), 'image-rendering:pixelated'].join(';')}"`,
  ];
  return `<img ${attrs.join(' ')}>`;
}

function placeholder(entry: PageEntry): string {
  const attrs = [`data-yaml-id="${escapeAttr(entry.id)}"`, ...frameAttrs(entry)];
  if (entry.photo?.field) attrs.push(`data-photo="${escapeAttr(entry.photo.field)}"`);
  if (entry.barcode?.type) attrs.push(`data-code="${escapeAttr(entry.barcode.type)}"`);
  return `<div class="tpl-placeholder" ${attrs.join(' ')} style="${frameStyles(entry).join(';')}"></div>`;
}

/** One entry as GrapesJS markup. Used for a full page paint and for an incremental append. */
export function entryToHtml(entry: PageEntry, page: PageDocument): string {
  if (entry.text) return textBlock(entry, page);
  if (entry.image) return imageBlock(entry);
  if (entry.barcode) return codeBlock(entry);
  if (entry.photo) return placeholder(entry);
  return '';
}

/** The GrapesJS document for a page object. The page stays the document the editor holds. */
export function pageToHtml(page: PageDocument): string {
  const css = `body { margin: 0; position: relative; width: ${cssNumber(page.page_size.width)}mm; height: ${cssNumber(page.page_size.height)}mm; overflow: hidden; background: #fff; }`;
  const body = page.contents.map((entry) => entryToHtml(entry, page)).join('\n');
  return joinDocument(css, body);
}

const PT_TO_MM = 25.4 / 72;
const ALLOWED_TAGS = new Set(['br', 'b', 'bold', 'i', 'u', 'span', 'leading']);

export interface StyleProperty {
  property: string;
  name?: string;
  type?: string;
  units?: string[];
  default?: string;
  min?: number;
  max?: number;
  step?: number;
  options?: { id: string; label?: string; name?: string }[];
  properties?: StyleProperty[];
}

export interface StyleSector {
  name: string;
  open?: boolean;
  properties: StyleProperty[];
}

export interface CanvasBox {
  style: Record<string, string>;
  attrs: Record<string, string>;
}

export interface PaletteEntry {
  pageKind: 'var' | 'text' | 'code' | 'file';
  editorPath?: string;
  codeKind?: 'barcode' | 'qrcode';
  fileName?: string;
  posX?: number;
  posY?: number;
}

/** Sectors for `/operator/templates/:id`. Percent sizes and the CSS the page cannot store stay here. */
export function operatorStyleSectors(): StyleSector[] {
  return [
    {
      name: 'Положение и размер (мм)',
      open: true,
      properties: [
        { property: 'left', name: 'Слева', type: 'number', units: ['mm'], default: '0', min: -500 },
        { property: 'top', name: 'Сверху', type: 'number', units: ['mm'], default: '0', min: -500 },
        { property: 'width', name: 'Ширина', type: 'number', units: ['mm', '%'], default: 'auto' },
        { property: 'height', name: 'Высота', type: 'number', units: ['mm', '%'], default: 'auto' },
        { property: 'transform', name: 'Поворот / масштаб', type: 'composite' },
      ],
    },
    {
      name: 'Текст',
      open: true,
      properties: [
        { property: 'font-family', name: 'Шрифт' },
        { property: 'font-size', name: 'Кегль', type: 'number', units: ['pt', 'mm', 'px'], default: '12pt' },
        { property: 'font-weight', name: 'Насыщенность' },
        { property: 'font-style', name: 'Начертание' },
        { property: 'text-align', name: 'Выравнивание' },
        { property: 'line-height', name: 'Межстрочный', type: 'number', units: ['', 'pt'], default: '1.2' },
        { property: 'letter-spacing', name: 'Межбуквенный', type: 'number', units: ['pt', 'px'], default: '0' },
        { property: 'color', name: 'Цвет' },
        { property: 'white-space', name: 'Перенос строк' },
        { property: 'text-overflow', name: 'Если не влезает' },
        { property: 'overflow', name: 'Переполнение' },
      ],
    },
    {
      name: 'Оформление',
      open: false,
      properties: [
        { property: 'background-color', name: 'Фон' },
        { property: 'border', name: 'Рамка' },
        { property: 'border-radius', name: 'Скругление', type: 'number', units: ['mm', 'px'], default: '0' },
        { property: 'padding', name: 'Внутренние отступы', type: 'composite' },
        { property: 'opacity', name: 'Прозрачность', type: 'slider', min: 0, max: 1, step: 0.05, default: '1' },
      ],
    },
  ];
}

/** Sectors for `/editor/:id`. Every control writes a page field. */
export function yamlStyleSectors(): StyleSector[] {
  return [
    {
      name: 'Положение и размер (мм)',
      open: true,
      properties: [
        { property: 'left', name: 'Слева', type: 'number', units: ['mm'], default: '0', min: -500 },
        { property: 'top', name: 'Сверху', type: 'number', units: ['mm'], default: '0', min: -500 },
        { property: 'width', name: 'Ширина', type: 'number', units: ['mm'], default: '40mm' },
        { property: 'height', name: 'Высота', type: 'number', units: ['mm'], default: '10mm' },
        {
          property: '--rotation',
          name: 'Поворот',
          type: 'select',
          default: '0',
          options: [
            { id: '0', label: '0' },
            { id: '90', label: '90' },
          ],
        },
        { property: '--scale-x', name: 'Масштаб X', type: 'number', default: '1' },
        { property: '--scale-y', name: 'Масштаб Y', type: 'number', default: '1' },
        { property: '--skew-x', name: 'Наклон X', type: 'number', default: '0' },
        { property: '--skew-y', name: 'Наклон Y', type: 'number', default: '0' },
      ],
    },
    {
      name: 'Текст',
      open: true,
      properties: [
        { property: 'font-family', name: 'Шрифт' },
        { property: 'font-size', name: 'Кегль', type: 'number', units: ['pt'], default: '12pt' },
        { property: 'font-weight', name: 'Насыщенность' },
        { property: 'font-style', name: 'Начертание' },
        {
          property: 'text-decoration',
          name: 'Подчёркивание',
          type: 'select',
          default: 'none',
          options: [
            { id: 'none', label: 'нет' },
            { id: 'underline', label: 'подчёркнутый' },
          ],
        },
        { property: 'text-align', name: 'Выравнивание' },
        { property: 'line-height', name: 'Межстрочный', type: 'number', units: [''], default: '1.2' },
        { property: 'letter-spacing', name: 'Межбуквенный', type: 'number', units: ['mm'], default: '0' },
        { property: 'color', name: 'Цвет', type: 'color' },
      ],
    },
    {
      name: 'Оформление',
      open: true,
      properties: [
        { property: 'background-color', name: 'Фон', type: 'color' },
        { property: 'border-width', name: 'Рамка', type: 'number', units: ['mm'], default: '0' },
        { property: 'border-color', name: 'Цвет рамки', type: 'color' },
        { property: 'border-radius', name: 'Скругление', type: 'number', units: ['mm'], default: '0' },
        {
          property: 'padding',
          name: 'Внутренние отступы',
          type: 'composite',
          properties: [
            { property: 'padding-top', name: 'Сверху', type: 'number', units: ['mm'] },
            { property: 'padding-right', name: 'Справа', type: 'number', units: ['mm'] },
            { property: 'padding-bottom', name: 'Снизу', type: 'number', units: ['mm'] },
            { property: 'padding-left', name: 'Слева', type: 'number', units: ['mm'] },
          ],
        },
        { property: 'opacity', name: 'Прозрачность', type: 'slider', min: 0, max: 1, step: 0.05, default: '1' },
      ],
    },
  ];
}

export function ptToMm(pt: number): number {
  return round3(pt * PT_TO_MM);
}

export function flatKey(path: string): string {
  const trimmed = path.trim();
  for (const [flat, editor] of Object.entries(FLAT_PATHS)) {
    if (editor === trimmed) return flat;
  }
  const field = trimmed.match(/^(?:form_data\.)?(field_\d+)$/);
  if (field) return field[1];
  if (trimmed.startsWith('form_data.')) return trimmed.slice('form_data.'.length);
  return trimmed;
}

export function flatMustache(path: string, pipes: string[] = []): string {
  const head = flatKey(path);
  return `{{${pipes.length ? `${head}|${pipes.join('|')}` : head}}}`;
}

export function nextEntryId(contents: { id: string }[], kind: 't' | 'i' | 'b'): string {
  const re = new RegExp(`^p1-${kind}(\\d+)$`);
  let max = 0;
  for (const entry of contents) {
    const match = entry.id.match(re);
    if (match) max = Math.max(max, Number(match[1]));
  }
  return `p1-${kind}${max + 1}`;
}

export function appendMustache(entry: PageEntry, path: string): void {
  const next = flatMustache(path);
  const current = entry.preentered ?? '';
  entry.preentered = current ? `${current}<br>${next}` : next;
  entry.template = true;
  entry.content = null;
}

export function appendPaletteEntry(page: PageDocument, insert: PaletteEntry): PageEntry {
  const at = { posX: insert.posX ?? 5, posY: insert.posY ?? 5 };
  let entry: PageEntry;
  if (insert.pageKind === 'var') {
    entry = {
      id: nextEntryId(page.contents, 't'),
      ...at,
      width: 40,
      height: 10,
      template: true,
      preentered: flatMustache(insert.editorPath ?? ''),
      content: null,
      alignment: { vertical: 'top' },
      text: defaultText(),
    };
  } else if (insert.pageKind === 'text') {
    entry = {
      id: nextEntryId(page.contents, 't'),
      ...at,
      width: 40,
      height: 10,
      preentered: 'Текст',
      content: null,
      alignment: { vertical: 'top' },
      text: defaultText(),
    };
  } else if (insert.pageKind === 'code') {
    const qr = insert.codeKind !== 'barcode';
    entry = {
      id: nextEntryId(page.contents, 'b'),
      ...at,
      width: qr ? 25 : 40,
      height: qr ? 25 : 15,
      template: true,
      content: null,
      barcode: { type: qr ? 'qr' : 'ean13', preentered: '{{code}}' },
    };
  } else {
    entry = {
      id: nextEntryId(page.contents, 'i'),
      ...at,
      width: 30,
      height: 20,
      content: null,
      image: { resource_name: insert.fileName ?? '' },
    };
  }
  page.contents.push(entry);
  return entry;
}

/** A Text click. The bottom-left corner snaps to 0.5 mm. The frame fits `Текст` at the last font. */
export function appendTextAt(
  page: PageDocument,
  point: { x: number; y: number },
  options?: { style?: TextStyleMemory; measureLine?: (line: string) => number },
): PageEntry {
  const requested = options?.style ?? textStyleCache.read();
  const font = page.resources.fonts?.[requested.font] ? requested.font : 'text';
  const style = { ...requested, font };
  const text = textFromStyle(style);
  const family = page.resources.fonts?.[font]?.font_family || 'Liberation Serif';
  const measure = options?.measureLine ?? ((line: string) => measureLineMm(line, family, text.font_size, style.bold, style.italic));
  const size = textFrameMm(visibleText('Текст'), text.font_size, text.line_height, text.leading, 0, measure);
  const width = ceilHalfMm(size.width);
  const height = ceilHalfMm(size.height);
  const entry: PageEntry = {
    id: nextEntryId(page.contents, 't'),
    posX: snapMm(point.x),
    posY: snapMm(point.y) - height,
    width,
    height,
    preentered: 'Текст',
    content: null,
    alignment: { horizontal: 'left', vertical: 'top' },
    padding: { top: 0, right: 0, bottom: 0, left: 0 },
    text,
  };
  page.contents.push(entry);
  return entry;
}

/** A Rectangle drag. The frame is the drag. The box is an empty text entry. */
export function appendRect(
  page: PageDocument,
  rect: { posX: number; posY: number; width: number; height: number },
  style?: TextStyleMemory,
): PageEntry {
  const requested = style ?? textStyleCache.read();
  const font = page.resources.fonts?.[requested.font] ? requested.font : 'text';
  const text = textFromStyle({ ...requested, font });
  const entry: PageEntry = {
    id: nextEntryId(page.contents, 't'),
    posX: rect.posX,
    posY: rect.posY,
    width: rect.width,
    height: rect.height,
    preentered: '',
    content: null,
    background: normalizeHex(requested.background ?? '') ?? '#FFFFFF',
    border: { width: 0.2, color: '#000000' },
    alignment: { horizontal: 'left', vertical: 'top' },
    padding: { top: 0, right: 0, bottom: 0, left: 0 },
    text,
  };
  page.contents.push(entry);
  return entry;
}

/** A QR or barcode drag. The frame is the drag. The payload stays `{{code}}` until preview. */
export function appendCode(
  page: PageDocument,
  rect: { posX: number; posY: number; width: number; height: number },
  kind: 'qr' | 'ean13',
): PageEntry {
  const entry: PageEntry = {
    id: nextEntryId(page.contents, 'b'),
    posX: rect.posX,
    posY: rect.posY,
    width: rect.width,
    height: rect.height,
    template: true,
    content: null,
    barcode: { type: kind, preentered: '{{code}}' },
  };
  page.contents.push(entry);
  return entry;
}

/** Drop one entry. A canvas delete has to change the page, or the next add draws it again. */
export function removeEntry(page: PageDocument, id: string): boolean {
  const index = page.contents.findIndex((entry) => entry.id === id);
  if (index < 0) return false;
  page.contents.splice(index, 1);
  return true;
}

/** Widen or heighten a text box so the glyphs fit. `auto_scale` keeps the frame. */
export function growEntryToText(entry: PageEntry, measureLine: (line: string) => number): boolean {
  if (entry.auto_scale || !entry.text) return false;
  const text = entry.text;
  const size = textFrameMm(
    visibleText(entry.preentered ?? ''),
    text.font_size,
    text.line_height,
    text.leading ?? 0,
    text.letter_spacing ?? 0,
    measureLine,
  );
  const pad = entry.padding ?? {};
  const width = ceilHalfMm(size.width + (pad.left ?? 0) + (pad.right ?? 0));
  const height = ceilHalfMm(size.height + (pad.top ?? 0) + (pad.bottom ?? 0));
  let grew = false;
  if (width > entry.width) {
    entry.width = width;
    grew = true;
  }
  if (height > entry.height) {
    entry.height = height;
    grew = true;
  }
  return grew;
}

function textFromStyle(style: TextStyleMemory): PageTextStyle {
  const font_size = ptToMm(style.fontSizePt);
  const text: PageTextStyle = {
    font: style.font || 'text',
    font_size,
    leading: 0,
    line_height: round3(font_size * 1.2),
  };
  if (style.bold || style.italic || style.underline) {
    text.font_style = { bold: style.bold, italic: style.italic, underline: style.underline };
  }
  const foreground = normalizeHex(style.foreground ?? '');
  if (foreground && foreground !== '#000000') text.color = foreground;
  return text;
}

/** Foreground writes `text.color`. Background writes `background`. A non-hex is ignored. */
export function applyEntryColor(entry: PageEntry, target: 'foreground' | 'background', hex: string): boolean {
  const color = normalizeHex(hex);
  if (!color) return false;
  if (target === 'background') {
    entry.background = color;
    return true;
  }
  if (!entry.text) return false;
  if (color === '#000000') delete entry.text.color;
  else entry.text.color = color;
  return true;
}

/** A drag, resize, or trait edit writes the entry. Several mustaches stay byte for byte. */
export function syncEntryFromCanvas(entry: PageEntry, box: CanvasBox, source: 'trait' | 'geometry' = 'trait'): void {
  const { style, attrs } = box;
  const left = parseMm(style['left']);
  const top = parseMm(style['top']);
  const width = parseMm(style['width']);
  const height = parseMm(style['height']);
  if (left !== undefined) entry.posX = left;
  if (top !== undefined) entry.posY = top;
  if (width !== undefined && width > 0) entry.width = width;
  if (height !== undefined && height > 0) entry.height = height;

  if (entry.text) {
    const pt = parsePt(style['font-size']);
    if (pt !== undefined) entry.text.font_size = ptToMm(pt);
    const lineHeight = style['line-height']?.trim();
    if (lineHeight && /^[\d.]+$/.test(lineHeight) && entry.text.font_size) {
      entry.text.line_height = round3(entry.text.font_size * Number(lineHeight));
    }
    if ('data-leading' in attrs && attrs['data-leading'] !== '') {
      entry.text.leading = Number(attrs['data-leading']) || 0;
    }
    if (attrs['data-font']) entry.text.font = attrs['data-font'];
    if ('color' in style) {
      const color = toHex(style['color']);
      if (color && color !== '#000000') entry.text.color = color;
      else delete entry.text.color;
    }
    if ('font-weight' in style || 'font-style' in style || 'text-decoration' in style) {
      const fontStyle = {
        bold: style['font-weight'] === 'bold' || Number(style['font-weight']) >= 600,
        italic: style['font-style'] === 'italic',
        underline: (style['text-decoration'] ?? '').includes('underline'),
      };
      if (fontStyle.bold || fontStyle.italic || fontStyle.underline) entry.text.font_style = fontStyle;
      else delete entry.text.font_style;
    }
    if ('letter-spacing' in style) {
      const spacing = parseMm(style['letter-spacing']) ?? 0;
      if (spacing) entry.text.letter_spacing = spacing;
      else delete entry.text.letter_spacing;
    }
  }

  const vertical = attrs['data-valign'];
  const horizontal = style['text-align'];
  if (vertical || horizontal) {
    entry.alignment = { ...entry.alignment };
    if (vertical === 'top' || vertical === 'middle' || vertical === 'bottom') entry.alignment.vertical = vertical;
    if (horizontal) entry.alignment.horizontal = horizontal;
  }
  if ('data-auto-scale' in attrs) {
    if (attrs['data-auto-scale'] === 'true') entry.auto_scale = true;
    else delete entry.auto_scale;
  }
  if ('data-if' in attrs) {
    const value = attrs['data-if'].trim();
    if (value) entry.ifSt = value;
    else delete entry.ifSt;
  }
  const rotation = style['--rotation'] ?? attrs['data-rotation'];
  if (rotation === '90') entry.rotation = 90;
  else if (rotation === '0') delete entry.rotation;
  writeOptionalNumber(entry, 'opacity', style['opacity'], 1);
  writeScale(entry, 'scale_x', style['--scale-x'] ?? attrs['data-scale-x'], 1);
  writeScale(entry, 'scale_y', style['--scale-y'] ?? attrs['data-scale-y'], 1);
  writeScale(entry, 'skew_x', style['--skew-x'] ?? attrs['data-skew-x'], 0);
  writeScale(entry, 'skew_y', style['--skew-y'] ?? attrs['data-skew-y'], 0);
  writeBackground(entry, style);
  writeBorder(entry, style);
  writeRadius(entry, style);
  writePadding(entry, style);
  if (source === 'trait') writePreentered(entry, attrs);
}

export function formatPageJson(page: PageDocument): string {
  return JSON.stringify(page, null, 2);
}

export function applyPageJson(text: string): { ok: true; page: PageDocument } | { ok: false; detail: string } {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return { ok: false, detail: 'invalid json' };
  }
  const detail = validateDocument(value);
  if (detail) return { ok: false, detail };
  return { ok: true, page: value as PageDocument };
}

export function entryVisible(ifSt: string | null | undefined, formData: Record<string, unknown>): boolean {
  if (!ifSt) return true;
  const value = formData[ifSt];
  if (value === undefined || value === null || value === '' || value === false || value === 0) return false;
  return true;
}

function defaultText(): PageTextStyle {
  const font_size = ptToMm(12);
  return { font: 'text', font_size, leading: 0, line_height: round3(font_size * 1.2) };
}

function round3(value: number): number {
  return Math.round(value * 1000) / 1000;
}

function parseMm(value: string | undefined): number | undefined {
  if (value === undefined || value === '') return undefined;
  const match = String(value).trim().match(/^(-?[\d.]+)\s*mm$/i);
  if (!match) return undefined;
  const parsed = Number(match[1]);
  return Number.isFinite(parsed) ? round3(parsed) : undefined;
}

function parsePt(value: string | undefined): number | undefined {
  if (!value) return undefined;
  const match = String(value).trim().match(/^([\d.]+)\s*pt$/i);
  if (!match) return undefined;
  const parsed = Number(match[1]);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function toHex(value: string | undefined): string | undefined {
  if (!value) return undefined;
  const hex = value.trim().match(/^#([0-9a-fA-F]{6})$/);
  if (hex) return `#${hex[1].toUpperCase()}`;
  const rgb = value.trim().match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
  if (!rgb) return undefined;
  const channel = (part: string) => Number(part).toString(16).padStart(2, '0');
  return `#${channel(rgb[1])}${channel(rgb[2])}${channel(rgb[3])}`.toUpperCase();
}

function writeOptionalNumber(entry: PageEntry, key: 'opacity', raw: string | undefined, identity: number) {
  if (raw === undefined || raw === '') return;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) return;
  if (parsed === identity) delete entry[key];
  else entry[key] = parsed;
}

function writeScale(entry: PageEntry, key: 'scale_x' | 'scale_y' | 'skew_x' | 'skew_y', raw: string | undefined, identity: number) {
  if (raw === undefined || raw === '') return;
  const parsed = Number(String(raw).replace(/deg$/i, ''));
  if (!Number.isFinite(parsed)) return;
  if (parsed === identity) delete entry[key];
  else entry[key] = parsed;
}

function writeBackground(entry: PageEntry, style: Record<string, string>) {
  if (!('background-color' in style) && !('background' in style)) return;
  const color = toHex(style['background-color'] || style['background']);
  if (color) entry.background = color;
  else delete entry.background;
}

function writeBorder(entry: PageEntry, style: Record<string, string>) {
  const touched = ['border', 'border-width', 'border-color', 'border-top-width', 'border-top-color'].some((key) => key in style);
  if (!touched) return;
  const width = parseMm(style['border-width']) ?? parseMm(style['border-top-width']);
  const color = toHex(style['border-color'] || style['border-top-color'] || '');
  if (width && width > 0 && color) {
    entry.border = { width, color };
    return;
  }
  const shorthand = style['border'] ?? '';
  const match = shorthand.match(/([\d.]+)mm\s+solid\s+(#[0-9A-Fa-f]{6})/i);
  if (match) entry.border = { width: Number(match[1]), color: `#${match[2].slice(-6).toUpperCase()}` };
  else delete entry.border;
}

function writeRadius(entry: PageEntry, style: Record<string, string>) {
  if (!('border-radius' in style)) return;
  const radius = parseMm(style['border-radius']) ?? 0;
  if (radius > 0) entry.border_radius = radius;
  else delete entry.border_radius;
}

function writePadding(entry: PageEntry, style: Record<string, string>) {
  const sides = ['padding-top', 'padding-right', 'padding-bottom', 'padding-left'] as const;
  if (!('padding' in style) && !sides.some((side) => side in style)) return;
  const fromShorthand = parsePadding(style['padding']);
  entry.padding = {
    top: parseMm(style['padding-top']) ?? fromShorthand.top,
    right: parseMm(style['padding-right']) ?? fromShorthand.right,
    bottom: parseMm(style['padding-bottom']) ?? fromShorthand.bottom,
    left: parseMm(style['padding-left']) ?? fromShorthand.left,
  };
}

function parsePadding(value: string | undefined): { top: number; right: number; bottom: number; left: number } {
  const parts = (value ?? '').trim().split(/\s+/).map((part) => parseMm(part) ?? 0);
  if (parts.length === 1) return { top: parts[0], right: parts[0], bottom: parts[0], left: parts[0] };
  if (parts.length === 2) return { top: parts[0], right: parts[1], bottom: parts[0], left: parts[1] };
  if (parts.length === 3) return { top: parts[0], right: parts[1], bottom: parts[2], left: parts[1] };
  return { top: parts[0] ?? 0, right: parts[1] ?? 0, bottom: parts[2] ?? 0, left: parts[3] ?? 0 };
}

function writePreentered(entry: PageEntry, attrs: Record<string, string>) {
  if (!entry.text) return;
  const source = entry.preentered ?? '';
  const count = source.match(/\{\{[^{}]*\}\}/g)?.length ?? 0;
  if (count > 1) return;
  if (!attrs['data-var']) {
    if (count === 0 && attrs['data-preentered'] !== undefined) {
      entry.preentered = attrs['data-preentered'];
      entry.content = null;
    }
    return;
  }
  const pipes: string[] = [];
  if (attrs['data-join']) pipes.push(`join:${JSON.stringify(attrs['data-join'])}`);
  if (attrs['data-default']) pipes.push(`default:${JSON.stringify(attrs['data-default'])}`);
  if (attrs['data-case'] === 'upper' || attrs['data-case'] === 'title') pipes.push(attrs['data-case']);
  entry.preentered = `${attrs['data-prefix'] ?? ''}${flatMustache(attrs['data-var'], pipes)}${attrs['data-suffix'] ?? ''}`;
  entry.template = true;
  entry.content = null;
}

function visibleText(source: string): string {
  const withBreaks = source.replace(/<br\s*\/?>/gi, '\n');
  const withoutTags = withBreaks.replace(/<[^>]+>/g, '');
  return withoutTags.replace(/\{\{([^{}]+)\}\}/g, (_full, body: string) => demoForMustache(body));
}

function demoForMustache(body: string): string {
  const [name, ...pipes] = body.split('|').map((part) => part.trim());
  let value = demoValue(name);
  if (pipes.includes('upper')) value = value.toUpperCase();
  if (pipes.includes('title')) {
    value = value.replace(/\p{L}+/gu, (word) => word[0].toUpperCase() + word.slice(1).toLowerCase());
  }
  return value;
}

function demoValue(flat: string): string {
  if (flat === 'number') return demoContext.ticket.number;
  if (flat === 'code') return demoContext.ticket.code;
  if (flat === 'ticket_type_name') return demoContext.ticket.ticket_type_name;
  const sample = demoContext.form_data[flat];
  if (Array.isArray(sample)) return sample.join(', ');
  if (typeof sample === 'boolean') return sample ? 'да' : '';
  return typeof sample === 'string' ? sample : '';
}

function validateDocument(value: unknown): string | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return 'page must be an object';
  const page = value as Record<string, unknown>;
  if ('pages' in page || 'metadata' in page) return 'page must not contain pages or metadata';
  const size = page['page_size'] as Record<string, unknown> | undefined;
  if (!size || typeof size !== 'object' || Array.isArray(size)) return 'page_size is required';
  if (rejectNumber(size['width'], 'page_size.width') || rejectNumber(size['height'], 'page_size.height')) {
    return 'page_size must be positive';
  }
  if (Number(size['width']) <= 0 || Number(size['height']) <= 0) return 'page_size must be positive';
  if (size['units'] !== 'mm' && size['units'] !== 'points') return 'page_size.units must be mm or points';
  const bleeds = (size['bleeds'] ?? { top: 0, right: 0, bottom: 0, left: 0 }) as Record<string, unknown>;
  for (const side of ['top', 'right', 'bottom', 'left']) {
    if (rejectNumber(bleeds[side], `bleeds.${side}`) || Number(bleeds[side]) < 0) return `bleeds.${side} must not be negative`;
  }
  if (size['background'] !== undefined && !isColor(size['background'])) return 'page_size.background must be #RRGGBB';
  if (!Array.isArray(page['contents'])) return 'contents must be an array';
  const ids = new Set<string>();
  for (const entry of page['contents']) {
    const detail = validateEntry(entry, ids);
    if (detail) return detail;
  }
  return null;
}

function validateEntry(value: unknown, ids: Set<string>): string | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return 'entry must be an object';
  const entry = value as Record<string, unknown>;
  if (typeof entry['id'] !== 'string' || !entry['id']) return 'entry id is required';
  if (ids.has(entry['id'])) return `duplicate entry id ${entry['id']}`;
  ids.add(entry['id']);
  for (const field of ['posX', 'posY', 'width', 'height']) {
    if (rejectNumber(entry[field], field)) return `${field} must be a number`;
  }
  if (Number(entry['width']) <= 0 || Number(entry['height']) <= 0) return 'entry width and height must be positive';
  const kinds = ['text', 'image', 'barcode', 'photo'].filter((kind) => entry[kind] != null);
  if (kinds.length !== 1) return 'entry must be text, image, barcode, or photo';
  if (entry['background'] !== undefined && !isColor(entry['background'])) return 'background must be #RRGGBB';
  if (entry['border'] !== undefined) {
    const border = entry['border'] as Record<string, unknown> | null;
    if (!border || typeof border !== 'object' || Array.isArray(border)) return 'border must be an object';
    if (rejectNumber(border['width'], 'border.width') || Number(border['width']) <= 0) return 'border.width must be positive';
    if (!isColor(border['color'])) return 'border.color must be #RRGGBB';
  }
  if (entry['border_radius'] !== undefined) {
    if (rejectNumber(entry['border_radius'], 'border_radius') || Number(entry['border_radius']) < 0) {
      return 'border_radius must not be negative';
    }
    if (Number(entry['border_radius']) > Math.min(Number(entry['width']), Number(entry['height'])) / 2) {
      return 'border_radius is larger than half the shorter side';
    }
  }
  if (entry['opacity'] !== undefined) {
    const opacity = Number(entry['opacity']);
    if (rejectNumber(entry['opacity'], 'opacity') || opacity < 0 || opacity > 1) return 'opacity must be between 0 and 1';
  }
  for (const field of ['scale_x', 'scale_y']) {
    if (entry[field] !== undefined && (rejectNumber(entry[field], field) || Number(entry[field]) === 0)) return `${field} must not be 0`;
  }
  for (const field of ['skew_x', 'skew_y']) {
    if (entry[field] === undefined) continue;
    if (rejectNumber(entry[field], field)) return `${field} must be a number`;
    const turns = ((Number(entry[field]) % 180) + 180) % 180;
    if (Math.abs(turns - 90) < 1e-6) return `${field} cannot be a right angle`;
  }
  if (entry['rotation'] !== undefined && entry['rotation'] !== 0 && entry['rotation'] !== 90) return 'rotation must be 0 or 90';
  if (entry['text']) {
    const text = entry['text'] as Record<string, unknown>;
    if (!text || typeof text['font'] !== 'string' || !text['font']) return 'text.font is required';
    for (const field of ['font_size', 'leading', 'line_height']) {
      if (rejectNumber(text[field], `text.${field}`)) return `text.${field} must be a number`;
    }
    if (text['color'] !== undefined && !isColor(text['color'])) return 'text.color must be #RRGGBB';
  }
  if (entry['image']) {
    const image = entry['image'] as Record<string, unknown>;
    if (!image || typeof image['resource_name'] !== 'string' || !image['resource_name']) return 'image.resource_name is required';
  }
  if (entry['barcode']) {
    const barcode = entry['barcode'] as Record<string, unknown>;
    if (!barcode || !['ean13', 'code128', 'qr'].includes(String(barcode['type']))) return 'barcode.type must be ean13, code128, or qr';
    const tags = unknownTag(barcode['preentered']);
    if (tags) return tags;
  }
  if (entry['photo']) {
    const photo = entry['photo'] as Record<string, unknown>;
    if (!photo || typeof photo['field'] !== 'string' || !photo['field']) return 'photo.field is required';
  }
  return unknownTag(entry['preentered']);
}

function unknownTag(value: unknown): string | null {
  if (typeof value !== 'string' || !value) return null;
  const tags = value.matchAll(/<\/?([a-zA-Z][\w:-]*)\b[^>]*>/g);
  for (const match of tags) {
    const name = match[1].toLowerCase();
    if (!ALLOWED_TAGS.has(name)) return `unknown tag ${name} in preentered`;
  }
  return null;
}

function rejectNumber(value: unknown, _field: string): boolean {
  return typeof value !== 'number' || !Number.isFinite(value);
}

function isColor(value: unknown): boolean {
  return typeof value === 'string' && /^#[0-9A-Fa-f]{6}$/.test(value);
}
