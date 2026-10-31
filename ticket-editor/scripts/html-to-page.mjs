import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { stringify } from 'yaml';

const here = path.dirname(fileURLToPath(import.meta.url));
const defaultFont = path.join(here, '..', '..', 'tests', 'fixtures', 'struct-to-pdf', 'fonts', 'LiberationSerif-Regular.ttf');

const SKIPPED_STYLE = new Set([
  'white-space',
  'text-overflow',
  'overflow',
  'z-index',
  'vertical-align',
  'transform',
  'display',
]);

const USED_STYLE = new Set([
  'position',
  'left',
  'top',
  'width',
  'height',
  'font-size',
  'line-height',
  'color',
  'font-weight',
  'font-style',
  'text-decoration',
  'text-align',
  'letter-spacing',
  'background',
  'background-color',
  'border',
  'border-width',
  'border-color',
  'border-top-width',
  'border-top-color',
  'border-radius',
  'padding',
  'padding-top',
  'padding-right',
  'padding-bottom',
  'padding-left',
  'opacity',
  '--scale-x',
  '--scale-y',
  '--skew-x',
  '--skew-y',
  '--rotation',
]);

export function htmlToPage(input, outDir, options = {}) {
  const source = readSource(input);
  const body = parseBody(source.html, rulesFrom(source.css));
  const margins = marginsFrom(source.manifest);
  const report = { boxes: 0, files: [], skipped: [], filled: [] };
  const contents = [];
  let textN = 0;
  let imageN = 0;
  let codeN = 0;
  for (const child of body.children) {
    const style = { ...matching(body.rules, child.tag, child.classes), ...child.style };
    const background = sheetBackground(child, style, body);
    if (!background) continue;
    imageN += 1;
    contents.push(imageEntry(`p1-i${imageN}`, background));
  }
  for (const child of body.children) {
    const style = { ...matching(body.rules, child.tag, child.classes), ...child.style };
    if (sheetBackground(child, style, body)) continue;
    if (!absolute(style)) {
      report.skipped.push(skipLine(child, 'has no position:absolute'));
      continue;
    }
    noteSkipped(report, child, style);
    const box = readBox(child, style, report);
    if (box.kind === 'skip') {
      report.skipped.push(box.reason);
      continue;
    }
    if (box.kind === 'text') {
      textN += 1;
      contents.push(textEntry(`p1-t${textN}`, box));
    } else if (box.kind === 'image') {
      imageN += 1;
      contents.push(imageEntry(`p1-i${imageN}`, box));
    } else {
      codeN += 1;
      contents.push(codeEntry(`p1-b${codeN}`, box));
    }
  }
  fillMissingHeights(contents, report);
  report.boxes = contents.length;

  const copied = copyAssets(source, outDir);
  report.files.push(...copied.files);
  const page = {
    page_size: {
      width: body.width,
      height: body.height,
      units: 'mm',
      bleeds: { top: 0, right: 0, bottom: 0, left: 0 },
    },
    resources: {
      images: copied.images,
      fonts: copied.fonts,
    },
    contents,
  };
  const meta = {
    id: options.id ?? path.basename(outDir),
    name: source.manifest?.name || 'Badge',
    source: 'local',
    is_default: false,
    updated_at: '',
    margin_top_mm: margins.top,
    margin_right_mm: margins.right,
    margin_bottom_mm: margins.bottom,
    margin_left_mm: margins.left,
    manifest: copied.manifest,
  };
  fs.mkdirSync(outDir, { recursive: true });
  fs.writeFileSync(path.join(outDir, 'template.yml'), stringify(page));
  fs.writeFileSync(path.join(outDir, 'meta.yml'), stringify(meta));
  return { page, meta, report };
}

export function formatReport(report) {
  const lines = [`boxes: ${report.boxes}`, `files: ${report.files.length}`];
  for (const file of report.files) lines.push(`file: ${file}`);
  for (const item of report.skipped) lines.push(`skipped: ${item}`);
  for (const item of report.filled) lines.push(`filled: ${item}`);
  return lines.join('\n');
}

function readSource(input) {
  const stat = fs.statSync(input);
  const dir = stat.isDirectory() ? input : path.dirname(input);
  const htmlPath = stat.isDirectory() ? path.join(input, 'template.html') : input;
  const manifestPath = path.join(dir, 'manifest.json');
  const manifest = fs.existsSync(manifestPath) ? JSON.parse(fs.readFileSync(manifestPath, 'utf8')) : null;
  let css = '';
  for (const relative of Object.values(manifest?.files ?? {})) {
    if (typeof relative === 'string' && relative.toLowerCase().endsWith('.css')) {
      css += `\n${fs.readFileSync(path.join(dir, relative), 'utf8')}`;
    }
  }
  return { dir, html: fs.readFileSync(htmlPath, 'utf8'), manifest, css, cloud: Boolean(manifest) && !manifest.page };
}

function parseBody(html, rules) {
  const styleText = [...html.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style>/gi)].map((match) => match[1]).join('\n');
  const all = rules.concat(rulesFrom(styleText));
  const body = html.match(/<body\b([^>]*)>([\s\S]*?)<\/body>/i);
  if (!body) throw new Error('template has no body');
  const attrs = parseAttrs(body[1]);
  const style = { ...matching(all, 'body', classesOf(attrs)), ...declarations(attrs.style) };
  const width = lengthMm(style.width);
  const height = lengthMm(style.height);
  if (width === undefined || height === undefined) throw new Error('body width and height must be lengths');
  return { width, height, rules: all, children: childElements(body[2]) };
}

function childElements(inner) {
  const out = [];
  const re = /<([a-zA-Z][\w:-]*)\b([^>]*)>([\s\S]*?)<\/\1>|<([a-zA-Z][\w:-]*)\b([^>]*?)\/?>/g;
  for (const match of inner.matchAll(re)) {
    const tag = (match[1] || match[4]).toLowerCase();
    const raw = match[2] ?? match[5] ?? '';
    const attrs = parseAttrs(raw);
    out.push({
      tag,
      attrs,
      classes: classesOf(attrs),
      style: declarations(attrs.style),
      text: visibleSource(match[3] ?? ''),
      src: attrs.src ?? '',
    });
  }
  return out;
}

function readBox(child, style, report) {
  const left = lengthMm(style.left);
  const top = lengthMm(style.top);
  const width = lengthOf(style.width, report, child);
  if (left === undefined || top === undefined || width === undefined) {
    return { kind: 'skip', reason: skipLine(child, 'left, top, or width is missing') };
  }
  const height = lengthMm(style.height);
  const placed = {
    posX: round3(left),
    posY: round3(top),
    width: round3(width),
    height: height === undefined ? undefined : round3(height),
    frame: frameFrom(style),
  };
  const code = codeFrom(child);
  if (code) return { ...placed, kind: 'code', ...code };
  const image = imageFrom(child);
  if (image) return { ...placed, kind: 'image', resource: image };
  return { ...placed, kind: 'text', ...textFrom(child, style, report) };
}

function textEntry(id, box) {
  const entry = {
    id,
    posX: box.posX,
    posY: box.posY,
    width: box.width,
    height: box.height,
    alignment: { horizontal: box.horizontal, vertical: box.vertical },
    padding: box.padding,
    ...box.frame,
    text: {
      font: box.font,
      font_size: box.font_size,
      leading: box.leading,
      line_height: box.line_height,
      preentered: box.preentered,
      content: null,
    },
  };
  if (box.template) entry.template = true;
  if (box.color) entry.text.color = box.color;
  if (box.font_style) entry.text.font_style = box.font_style;
  if (box.letter_spacing) entry.text.letter_spacing = box.letter_spacing;
  if (box.auto_scale) entry.auto_scale = true;
  if (box.ifSt) entry.ifSt = box.ifSt;
  if (box.rotation) entry.rotation = box.rotation;
  return entry;
}

function imageEntry(id, box) {
  return {
    id,
    posX: box.posX,
    posY: box.posY,
    width: box.width,
    height: box.height,
    ...box.frame,
    image: { resource_name: box.resource },
  };
}

function codeEntry(id, box) {
  return {
    id,
    posX: box.posX,
    posY: box.posY,
    width: box.width,
    height: box.height,
    template: true,
    ...box.frame,
    barcode: { type: box.type, preentered: box.preentered, content: null },
  };
}

function textFrom(child, style, report) {
  const boldTag = child.tag === 'b' || child.tag === 'strong';
  const weight = style['font-weight'];
  const bold = boldTag || weight === 'bold' || Number(weight) >= 600;
  const italic = style['font-style'] === 'italic';
  const underline = (style['text-decoration'] ?? '').includes('underline');
  let font_size = fontMm(style['font-size']);
  if (font_size === undefined) {
    font_size = round3(12 * 25.4 / 72);
    report.filled.push(`${label(child)} font-size missing, used 12pt`);
  }
  const line_height = lineHeightMm(style['line-height'], font_size);
  const preentered = preenteredFrom(child);
  const font_style = bold || italic || underline
    ? { weight: bold ? 'bold' : 'normal', italic, underline }
    : undefined;
  return {
    font: child.attrs['data-font'] || 'text',
    font_size,
    leading: child.attrs['data-leading'] ? Number(child.attrs['data-leading']) || 0 : 0,
    line_height,
    preentered,
    template: preentered.includes('{{'),
    color: colorOf(style.color),
    horizontal: style['text-align'] || 'left',
    vertical: valign(child.attrs['data-valign']),
    padding: paddingOf(style),
    letter_spacing: lengthMm(style['letter-spacing']) || undefined,
    font_style,
    auto_scale: Boolean(child.attrs['data-fit']),
    ifSt: child.attrs['data-if']?.trim() || '',
    rotation: child.attrs['data-rotation'] === '90' || style['--rotation'] === '90' ? 90 : 0,
  };
}

function preenteredFrom(child) {
  if (!child.attrs['data-var']) return child.text;
  const pipes = [];
  if (child.attrs['data-join']) pipes.push(`join:${JSON.stringify(child.attrs['data-join'])}`);
  if (child.attrs['data-default']) pipes.push(`default:${JSON.stringify(child.attrs['data-default'])}`);
  if (child.attrs['data-case'] === 'upper' || child.attrs['data-case'] === 'title') pipes.push(child.attrs['data-case']);
  const name = flatKey(child.attrs['data-var']);
  const mustache = `{{${pipes.length ? `${name}|${pipes.join('|')}` : name}}}`;
  return `${child.attrs['data-prefix'] ?? ''}${mustache}${child.attrs['data-suffix'] ?? ''}`;
}

function codeFrom(child) {
  const kind = (child.attrs['data-code'] || '').toLowerCase();
  const name = mustacheName(child.src);
  if (kind === 'barcode' || name === 'barcode' || name === 'barcode_svg') return { type: 'ean13', preentered: '{{code}}' };
  if (kind === 'qrcode' || name === 'qrcode' || name === 'qrcode_svg') return { type: 'qr', preentered: '{{code}}' };
  if (kind === 'vcard' || name === 'vcard_qrcode' || name === 'vcard_qrcode_svg') return { type: 'qr', preentered: '{{vcard}}' };
  return null;
}

/** A `{{ files.* }}` image with no position, sized to the body, is the sheet
 * artwork. It is painted first, at the body origin, at the body size. */
function sheetBackground(child, style, body) {
  const resource = imageFrom(child);
  if (!resource || absolute(style)) return null;
  const width = lengthMm(style.width);
  const height = lengthMm(style.height);
  const coversWidth = width === undefined || Math.abs(width - body.width) < 0.5;
  const coversHeight = height === undefined || Math.abs(height - body.height) < 0.5;
  if (!coversWidth || !coversHeight) return null;
  return {
    posX: 0,
    posY: 0,
    width: body.width,
    height: body.height,
    frame: frameFrom(style),
    resource,
  };
}

function imageFrom(child) {
  if (child.attrs['data-file']) return child.attrs['data-file'];
  const name = mustacheName(child.src);
  const files = name.match(/^files\.(.+)$/);
  if (files) return files[1];
  return null;
}

function fillMissingHeights(contents, report) {
  for (const entry of contents) {
    if (entry.height !== undefined && entry.height !== null) continue;
    const height = entry.barcode?.type === 'qr'
      ? entry.width
      : entry.barcode
        ? 15
        : entry.text.line_height;
    entry.height = round3(height);
    report.filled.push(`${entry.id} height missing, used ${entry.height}`);
  }
  for (let index = 0; index < contents.length; index += 1) {
    const entry = contents[index];
    if (!entry.text || !report.filled.some((line) => line.startsWith(`${entry.id} height`))) continue;
    const below = contents.slice(index + 1).find((other) => other.text && other.posY > entry.posY);
    if (!below) continue;
    const gap = round3(below.posY - entry.posY);
    if (gap > entry.text.line_height && gap < 40) {
      entry.height = gap;
      report.filled = report.filled.filter((line) => !line.startsWith(`${entry.id} height`));
      report.filled.push(`${entry.id} height missing, used ${gap}`);
    }
  }
}

function frameFrom(style) {
  const frame = {};
  const background = colorOf(style['background-color'] || style.background);
  if (background) frame.background = background;
  const border = borderOf(style);
  if (border) frame.border = border;
  const radius = lengthMm(style['border-radius']);
  if (radius) frame.border_radius = radius;
  if (style.opacity !== undefined && style.opacity !== '' && Number(style.opacity) !== 1) frame.opacity = Number(style.opacity);
  writeScale(frame, 'scale_x', style['--scale-x'], 1);
  writeScale(frame, 'scale_y', style['--scale-y'], 1);
  writeScale(frame, 'skew_x', style['--skew-x'], 0);
  writeScale(frame, 'skew_y', style['--skew-y'], 0);
  return frame;
}

function copyAssets(source, outDir) {
  const images = {};
  const fonts = {
    text: {
      name: 'text',
      font_family: 'Liberation Serif',
      weight: 'normal',
      italic: false,
      embedded: true,
      source_path: 'fonts/LiberationSerif-Regular.ttf',
    },
  };
  const manifest = {};
  const files = [];
  fs.mkdirSync(path.join(outDir, 'fonts'), { recursive: true });
  fs.copyFileSync(defaultFont, path.join(outDir, 'fonts', 'LiberationSerif-Regular.ttf'));
  for (const [name, relative] of Object.entries(source.manifest?.files ?? {})) {
    if (typeof relative !== 'string' || relative.toLowerCase().endsWith('.css')) continue;
    const from = path.join(source.dir, relative);
    const target = path.join(outDir, relative);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.copyFileSync(from, target);
    files.push(relative);
    manifest[name] = relative;
    if (/\.(ttf|otf)$/i.test(relative)) {
      fonts[name] = {
        name,
        font_family: name,
        weight: 'normal',
        italic: false,
        embedded: true,
        source_path: relative.startsWith('fonts/') ? relative : `fonts/${path.basename(relative)}`,
      };
      if (!relative.startsWith('fonts/')) {
        const fontTarget = path.join(outDir, 'fonts', path.basename(relative));
        fs.copyFileSync(from, fontTarget);
        fonts[name].source_path = `fonts/${path.basename(relative)}`;
      }
    } else if (/\.(woff2?)$/i.test(relative)) {
      files.push(`${relative} is not embedded`);
    } else {
      images[name] = { name, extracted_path: relative, file_format: path.extname(relative).slice(1) };
    }
  }
  return { images, fonts, manifest, files };
}

function marginsFrom(manifest) {
  const page = manifest?.page;
  if (page?.margins_mm) {
    return {
      top: Number(page.margins_mm.top) || 0,
      right: Number(page.margins_mm.right) || 0,
      bottom: Number(page.margins_mm.bottom) || 0,
      left: Number(page.margins_mm.left) || 0,
    };
  }
  if (manifest && !page) return { top: 2, right: 2, bottom: 2, left: 2 };
  return { top: 0, right: 0, bottom: 0, left: 0 };
}

function rulesFrom(css) {
  const rules = [];
  for (const match of css.matchAll(/([^{}@]+)\{([^{}]*)\}/g)) {
    const decls = declarations(match[2]);
    for (const selector of match[1].split(',')) {
      const trimmed = selector.trim();
      if (trimmed) rules.push({ selector: trimmed.toLowerCase(), decls });
    }
  }
  return rules;
}

function matching(rules, tag, classes) {
  const style = {};
  for (const rule of rules) {
    if (rule.selector === tag || classes.some((name) => rule.selector === `.${name}`)) {
      Object.assign(style, rule.decls);
    }
  }
  return style;
}

function declarations(style) {
  const out = {};
  if (!style) return out;
  for (const part of String(style).split(';')) {
    const index = part.indexOf(':');
    if (index < 0) continue;
    out[part.slice(0, index).trim().toLowerCase()] = part.slice(index + 1).trim();
  }
  return out;
}

function parseAttrs(raw) {
  const attrs = {};
  const re = /([^\s=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|\{\{([\s\S]*?)\}\}|([^\s"'=<>]+)))?/g;
  for (const match of String(raw).matchAll(re)) {
    const key = match[1].toLowerCase();
    if (match[2] !== undefined) attrs[key] = match[2];
    else if (match[3] !== undefined) attrs[key] = match[3];
    else if (match[4] !== undefined) attrs[key] = `{{${match[4]}}}`;
    else if (match[5] !== undefined) attrs[key] = match[5];
    else attrs[key] = '';
  }
  return attrs;
}

function classesOf(attrs) {
  return (attrs.class ?? '').split(/\s+/).map((name) => name.toLowerCase()).filter(Boolean);
}

function visibleSource(inner) {
  const withBreaks = inner.replace(/<br\s*\/?>/gi, '\n');
  const plain = withBreaks.replace(/<[^>]+>/g, '');
  const flat = plain.replace(/\{\{([\s\S]*?)\}\}/g, (_full, body) => `{{${flattenBody(body)}}}`);
  return flat.replace(/[ \t]*\n[ \t]*/g, '\n').replace(/[ \t]{2,}/g, ' ').trim().replace(/\n/g, '<br>');
}

function flattenBody(body) {
  const parts = body.split('|').map((part) => part.trim());
  parts[0] = flatKey(parts[0]);
  return parts.join('|');
}

function flatKey(pathName) {
  return pathName.trim().replace(/^form_data\./, '');
}

function mustacheName(src) {
  const match = String(src).match(/\{\{\s*([^}|]+?)\s*\}\}/);
  return match ? match[1].trim() : '';
}

function absolute(style) {
  return (style.position ?? '').toLowerCase() === 'absolute';
}

function lengthOf(value, report, child) {
  if (value && String(value).includes('%')) {
    report.skipped.push(skipLine(child, `width ${value}`));
    return undefined;
  }
  return lengthMm(value);
}

function lengthMm(value) {
  if (value === undefined || value === '') return undefined;
  const match = String(value).trim().match(/^(-?[\d.]+)\s*(mm|pt|px)$/i);
  if (!match) return undefined;
  const number = Number(match[1]);
  if (!Number.isFinite(number)) return undefined;
  const unit = match[2].toLowerCase();
  if (unit === 'mm') return round3(number);
  if (unit === 'pt') return round3(number * 25.4 / 72);
  return round3(number * 25.4 / 96);
}

function fontMm(value) {
  if (!value) return undefined;
  const match = String(value).trim().match(/^([\d.]+)\s*(pt|mm|px)?$/i);
  if (!match) return undefined;
  const number = Number(match[1]);
  const unit = (match[2] || 'pt').toLowerCase();
  if (unit === 'pt') return round3(number * 25.4 / 72);
  if (unit === 'px') return round3(number * 25.4 / 96);
  return round3(number);
}

function lineHeightMm(value, fontSize) {
  if (!value) return round3(fontSize * 1.2);
  if (/^[\d.]+$/.test(value.trim())) return round3(fontSize * Number(value));
  return lengthMm(value) ?? round3(fontSize * 1.2);
}

function colorOf(value) {
  if (!value) return undefined;
  const hex = String(value).trim().match(/^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/);
  if (hex) {
    const digits = hex[1].length === 3 ? [...hex[1]].map((ch) => ch + ch).join('') : hex[1];
    const color = `#${digits.toUpperCase()}`;
    return color === '#000000' ? undefined : color;
  }
  const rgb = String(value).trim().match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
  if (!rgb) return undefined;
  const channel = (part) => Number(part).toString(16).padStart(2, '0');
  const color = `#${channel(rgb[1])}${channel(rgb[2])}${channel(rgb[3])}`.toUpperCase();
  return color === '#000000' ? undefined : color;
}

function borderOf(style) {
  const shorthand = style.border ?? '';
  const match = shorthand.match(/([\d.]+)\s*mm\s+solid\s+(#[0-9A-Fa-f]{3,6})/i);
  if (match) {
    const color = colorOf(match[2]);
    if (color) return { width: Number(match[1]), color };
  }
  const width = lengthMm(style['border-width'] || style['border-top-width']);
  const color = colorOf(style['border-color'] || style['border-top-color'] || '');
  if (width && color) return { width, color };
  return undefined;
}

function paddingOf(style) {
  const from = paddingShorthand(style.padding);
  return {
    top: lengthMm(style['padding-top']) ?? from.top,
    right: lengthMm(style['padding-right']) ?? from.right,
    bottom: lengthMm(style['padding-bottom']) ?? from.bottom,
    left: lengthMm(style['padding-left']) ?? from.left,
  };
}

function paddingShorthand(value) {
  const parts = String(value ?? '').trim().split(/\s+/).map((part) => lengthMm(part) ?? 0);
  if (!value) return { top: 0, right: 0, bottom: 0, left: 0 };
  if (parts.length === 1) return { top: parts[0], right: parts[0], bottom: parts[0], left: parts[0] };
  if (parts.length === 2) return { top: parts[0], right: parts[1], bottom: parts[0], left: parts[1] };
  if (parts.length === 3) return { top: parts[0], right: parts[1], bottom: parts[2], left: parts[1] };
  return { top: parts[0] ?? 0, right: parts[1] ?? 0, bottom: parts[2] ?? 0, left: parts[3] ?? 0 };
}

function writeScale(frame, key, raw, identity) {
  if (raw === undefined || raw === '') return;
  const parsed = Number(String(raw).replace(/deg$/i, ''));
  if (!Number.isFinite(parsed) || parsed === identity) return;
  frame[key] = parsed;
}

function noteSkipped(report, child, style) {
  for (const key of Object.keys(style)) {
    if (USED_STYLE.has(key)) continue;
    report.skipped.push(`${label(child)} ${key}`);
  }
  if (child.attrs['data-dots']) report.skipped.push(`${label(child)} data-dots`);
  if (child.attrs['data-variant']) report.skipped.push(`${label(child)} data-variant`);
}

function skipLine(child, reason) {
  const src = child.src ? ` ${child.src.trim()}` : '';
  const size = [child.style.width, child.style.height].filter(Boolean).join(' × ');
  return `${child.tag}${size ? ` ${size}` : ''}${src} ${reason}`;
}

function label(child) {
  return child.tag;
}

function valign(value) {
  if (value === 'middle' || value === 'bottom') return value;
  return 'top';
}

function round3(value) {
  return Math.round(value * 1000) / 1000;
}

function parseArgs(argv) {
  const args = { input: '', out: '', id: '' };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === '--out') args.out = argv[++index];
    else if (token === '--id') args.id = argv[++index];
    else if (!args.input) args.input = token;
    else throw new Error(`unexpected argument ${token}`);
  }
  if (!args.input || !args.out) throw new Error('usage: html-to-page.mjs <html-or-dir> --out <dir>');
  return args;
}

const invoked = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (invoked) {
  const args = parseArgs(process.argv.slice(2));
  const { report } = htmlToPage(args.input, args.out, { id: args.id || undefined });
  console.log(formatReport(report));
}
