import grpc from '@grpc/grpc-js';
import protoLoader from '@grpc/proto-loader';
import express from 'express';
import JSZip from 'jszip';
import multer from 'multer';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse, stringify } from 'yaml';

const root = path.dirname(fileURLToPath(import.meta.url));
export const defaultTemplatesDir = path.join(root, 'data', 'templates');

/** Liberation file copied into each template. Compose sets `DEFAULT_FONT` inside the image. */
export function defaultFontPath() {
  if (process.env.DEFAULT_FONT) return path.resolve(process.env.DEFAULT_FONT);
  return path.join(root, '..', 'tests', 'fixtures', 'struct-to-pdf', 'fonts', 'LiberationSerif-Regular.ttf');
}

/** `pdf.proto` for the pdf-tooling client. Compose sets `PDF_PROTO`. */
export function pdfProtoPath() {
  if (process.env.PDF_PROTO) return path.resolve(process.env.PDF_PROTO);
  return path.join(root, '..', 'proto', 'irbis', 'pdf', 'v1', 'pdf.proto');
}

/** pdf-tooling gRPC address. Host default is the published port; Compose uses the service name. */
export function pdfToolingAddr() {
  return process.env.PDF_TOOLING_ADDR || '127.0.0.1:50052';
}

const origin = process.env.EDITOR_ORIGIN || 'http://localhost:4200';
const loopbackOrigin = /^https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?$/;

function allowOrigin(requestOrigin) {
  if (typeof requestOrigin === 'string' && loopbackOrigin.test(requestOrigin)) return requestOrigin;
  return origin;
}
const MAX_ASSET_BYTES = 10 * 1024 * 1024;

const WRITE_KEYS = [
  'name',
  'html',
  'editor_state',
  'page_width_mm',
  'page_height_mm',
  'margin_top_mm',
  'margin_right_mm',
  'margin_bottom_mm',
  'margin_left_mm',
  'updated_at',
];

const STANDARD_FORMATS = {
  A4: [210, 297],
  A5: [148, 210],
  A6: [105, 148],
  Letter: [215.9, 279.4],
  Legal: [215.9, 355.6],
};

const ASSET_EXTENSIONS = ['.png', '.jpg', '.jpeg', '.gif', '.svg', '.webp', '.ttf', '.otf', '.woff', '.woff2', '.css'];
const ASSET_NAME = /^[\p{L}\p{N}_.-]{1,100}$/u;

const CONTENT_TYPES = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.svg': 'image/svg+xml',
  '.webp': 'image/webp',
  '.ttf': 'font/ttf',
  '.otf': 'font/otf',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.css': 'text/css; charset=utf-8',
};

const FONT_FILE = /\.(ttf|otf)$/i;
const COLOR = /^#[0-9A-Fa-f]{6}$/;

export function blankPage() {
  return {
    page_size: {
      width: 100,
      height: 100,
      units: 'mm',
      bleeds: { top: 0, right: 0, bottom: 0, left: 0 },
    },
    resources: {
      images: {},
      fonts: {
        text: {
          name: 'text',
          font_family: 'Liberation Serif',
          weight: 'normal',
          italic: false,
          embedded: true,
          source_path: 'fonts/LiberationSerif-Regular.ttf',
        },
      },
    },
    contents: [],
  };
}

export function blankMeta() {
  return {
    id: 1,
    name: 'Badge',
    source: 'local',
    is_default: true,
    updated_at: '',
    margin_top_mm: 0,
    margin_right_mm: 0,
    margin_bottom_mm: 0,
    margin_left_mm: 0,
    manifest: {},
  };
}

export function blankTemplate() {
  return detailFrom(blankPage(), blankMeta());
}

function httpError(status, detail) {
  const error = new Error(detail);
  error.status = status;
  return error;
}

function templateDir(templatesDir, id) {
  if (!/^[A-Za-z0-9_-]+$/.test(id)) return null;
  return path.join(templatesDir, id);
}

function templateFile(templatesDir, id) {
  const dir = templateDir(templatesDir, id);
  return dir && path.join(dir, 'template.yml');
}

function metaYamlPath(dir) {
  return path.join(dir, 'meta.yml');
}

function metaJsonPath(dir) {
  return path.join(dir, 'meta.json');
}

function readMeta(dir) {
  const yml = metaYamlPath(dir);
  if (fs.existsSync(yml)) return parse(fs.readFileSync(yml, 'utf8'));
  const json = metaJsonPath(dir);
  if (fs.existsSync(json)) return JSON.parse(fs.readFileSync(json, 'utf8'));
  return null;
}

function writeMeta(dir, meta) {
  atomicWrite(metaYamlPath(dir), stringify(meta));
  const json = metaJsonPath(dir);
  if (fs.existsSync(json)) fs.unlinkSync(json);
}

function detailFrom(page, meta) {
  const width = page.page_size.width;
  const height = page.page_size.height;
  return {
    id: meta.id,
    name: meta.name,
    source: meta.source,
    is_default: meta.is_default,
    html: typeof meta.html === 'string' ? meta.html : null,
    page: structuredClone(page),
    editor_state: null,
    manifest: meta.manifest ?? {},
    created_at: '',
    updated_at: meta.updated_at ?? '',
    updated_by_name: '',
    page_width_mm: width,
    page_height_mm: height,
    margin_top_mm: meta.margin_top_mm,
    margin_right_mm: meta.margin_right_mm,
    margin_bottom_mm: meta.margin_bottom_mm,
    margin_left_mm: meta.margin_left_mm,
  };
}

function metaFrom(detail) {
  const meta = {
    id: detail.id,
    name: detail.name,
    source: detail.source,
    is_default: detail.is_default,
    updated_at: detail.updated_at,
    margin_top_mm: detail.margin_top_mm,
    margin_right_mm: detail.margin_right_mm,
    margin_bottom_mm: detail.margin_bottom_mm,
    margin_left_mm: detail.margin_left_mm,
    manifest: detail.manifest ?? {},
  };
  if (typeof detail.html === 'string') meta.html = detail.html;
  return meta;
}

export function readTemplate(file) {
  const dir = path.dirname(file);
  const page = parse(fs.readFileSync(file, 'utf8'));
  const meta = readMeta(dir);
  if (!meta) throw new Error(`missing meta in ${dir}`);
  return detailFrom(page, meta);
}

function atomicWrite(file, text) {
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, text);
  fs.renameSync(tmp, file);
}

function ensureFont(dir) {
  const target = path.join(dir, 'fonts', 'LiberationSerif-Regular.ttf');
  if (fs.existsSync(target)) return;
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.copyFileSync(defaultFontPath(), target);
}

function resourcesFrom(manifest) {
  const images = {};
  const fonts = { text: blankPage().resources.fonts.text };
  for (const [name, relative] of Object.entries(manifest ?? {})) {
    if (typeof relative !== 'string') continue;
    if (FONT_FILE.test(relative)) {
      fonts[name] = {
        name,
        font_family: name,
        weight: 'normal',
        italic: false,
        embedded: true,
        source_path: relative,
      };
    } else {
      images[name] = { name, extracted_path: relative };
    }
  }
  return { images, fonts };
}

function storedPageFrom(detail) {
  const page = structuredClone(detail.page ?? blankPage());
  page.page_size = {
    ...(page.page_size ?? {}),
    width: detail.page_width_mm,
    height: detail.page_height_mm,
    units: 'mm',
    bleeds: page.page_size?.bleeds ?? { top: 0, right: 0, bottom: 0, left: 0 },
  };
  page.resources = resourcesFrom(detail.manifest);
  if (!Array.isArray(page.contents)) page.contents = [];
  return page;
}

function writeTemplate(file, detail) {
  const dir = path.dirname(file);
  fs.mkdirSync(dir, { recursive: true });
  ensureFont(dir);
  const page = storedPageFrom(detail);
  writeMeta(dir, metaFrom(detail));
  atomicWrite(file, stringify(page));
  return page;
}

function pageFromLegacy(legacy) {
  const page = blankPage();
  const format = legacy.page_format ?? 'custom';
  const orientation = legacy.orientation ?? 'portrait';
  if (Object.hasOwn(STANDARD_FORMATS, format)) {
    let [width, height] = STANDARD_FORMATS[format];
    if (orientation === 'landscape') [width, height] = [height, width];
    page.page_size.width = width;
    page.page_size.height = height;
  } else if (Number.isFinite(legacy.page_width_mm) && Number.isFinite(legacy.page_height_mm)) {
    page.page_size.width = legacy.page_width_mm;
    page.page_size.height = legacy.page_height_mm;
  }
  return page;
}

function metaFromLegacy(legacy) {
  const meta = blankMeta();
  meta.id = legacy.id ?? meta.id;
  meta.name = legacy.name ?? meta.name;
  meta.source = legacy.source ?? meta.source;
  meta.is_default = legacy.is_default ?? meta.is_default;
  meta.updated_at = legacy.updated_at ?? '';
  meta.margin_top_mm = legacy.margin_top_mm ?? 0;
  meta.margin_right_mm = legacy.margin_right_mm ?? 0;
  meta.margin_bottom_mm = legacy.margin_bottom_mm ?? 0;
  meta.margin_left_mm = legacy.margin_left_mm ?? 0;
  meta.manifest = legacy.manifest ?? {};
  if (typeof legacy.html === 'string') meta.html = legacy.html;
  return meta;
}

function migrateJson(dir, jsonFile) {
  const legacy = JSON.parse(fs.readFileSync(jsonFile, 'utf8'));
  const page = pageFromLegacy(legacy);
  const meta = metaFromLegacy(legacy);
  fs.mkdirSync(dir, { recursive: true });
  ensureFont(dir);
  writeMeta(dir, meta);
  atomicWrite(path.join(dir, 'template.yml'), stringify(page));
}

function seedBlankTemplate(templatesDir) {
  const dir = templateDir(templatesDir, '1');
  const yml = path.join(dir, 'template.yml');
  const json = path.join(dir, 'template.json');
  if (!fs.existsSync(yml) && fs.existsSync(json)) {
    migrateJson(dir, json);
    return;
  }
  if (!fs.existsSync(yml)) writeTemplate(yml, blankTemplate());
}

function finiteNumber(value, field) {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw httpError(400, `${field} must be a finite number`);
  }
  return value;
}

function requireColor(value, field) {
  if (typeof value !== 'string' || !COLOR.test(value)) {
    throw httpError(400, `${field} must be a #RRGGBB string`);
  }
}

function remEuclid(value, modulus) {
  return ((value % modulus) + modulus) % modulus;
}

function validateText(text) {
  if (!text || typeof text !== 'object' || Array.isArray(text)) throw httpError(400, 'text must be an object');
  if (typeof text.font !== 'string' || !text.font) throw httpError(400, 'text.font is required');
  if (finiteNumber(text.font_size, 'text.font_size') <= 0) throw httpError(400, 'text.font_size must be positive');
  finiteNumber(text.leading, 'text.leading');
  finiteNumber(text.line_height, 'text.line_height');
  if (text.color !== undefined) requireColor(text.color, 'text.color');
  if (text.letter_spacing !== undefined) finiteNumber(text.letter_spacing, 'text.letter_spacing');
}

function validateEntry(entry, ids) {
  if (!entry || typeof entry !== 'object' || Array.isArray(entry)) throw httpError(400, 'entry must be an object');
  if (typeof entry.id !== 'string' || !entry.id) throw httpError(400, 'entry id is required');
  if (ids.has(entry.id)) throw httpError(400, `duplicate entry id ${entry.id}`);
  ids.add(entry.id);
  for (const field of ['posX', 'posY', 'width', 'height']) finiteNumber(entry[field], field);
  if (entry.width <= 0 || entry.height <= 0) throw httpError(400, 'entry width and height must be positive');

  const kinds = ['text', 'image', 'barcode', 'photo'].filter((kind) => entry[kind] != null);
  if (kinds.length !== 1) throw httpError(400, 'entry must be text, image, barcode, or photo');

  if (entry.background !== undefined) requireColor(entry.background, 'background');
  if (entry.border !== undefined) {
    if (!entry.border || typeof entry.border !== 'object' || Array.isArray(entry.border)) {
      throw httpError(400, 'border must be an object');
    }
    if (finiteNumber(entry.border.width, 'border.width') <= 0) throw httpError(400, 'border.width must be positive');
    requireColor(entry.border.color, 'border.color');
  }
  if (entry.border_radius !== undefined) {
    const radius = finiteNumber(entry.border_radius, 'border_radius');
    if (radius < 0) throw httpError(400, 'border_radius must not be negative');
    if (radius > Math.min(entry.width, entry.height) / 2) {
      throw httpError(400, 'border_radius is larger than half the shorter side');
    }
  }
  if (entry.opacity !== undefined) {
    const opacity = finiteNumber(entry.opacity, 'opacity');
    if (opacity < 0 || opacity > 1) throw httpError(400, 'opacity must be between 0 and 1');
  }
  for (const field of ['scale_x', 'scale_y']) {
    if (entry[field] !== undefined && finiteNumber(entry[field], field) === 0) {
      throw httpError(400, `${field} must not be 0`);
    }
  }
  for (const field of ['skew_x', 'skew_y']) {
    if (entry[field] === undefined) continue;
    const degrees = finiteNumber(entry[field], field);
    if (Math.abs(remEuclid(degrees / 90, 2) - 1) < 1e-6) throw httpError(400, `${field} cannot be a right angle`);
  }
  if (entry.rotation !== undefined && entry.rotation !== 0 && entry.rotation !== 90) {
    throw httpError(400, 'rotation must be 0 or 90');
  }
  if (entry.text) validateText(entry.text);
  if (entry.image && (typeof entry.image.resource_name !== 'string' || !entry.image.resource_name)) {
    throw httpError(400, 'image.resource_name is required');
  }
  if (entry.barcode && !['ean13', 'code128', 'qr'].includes(entry.barcode.type)) {
    throw httpError(400, 'barcode.type must be ean13, code128, or qr');
  }
  if (entry.photo && (typeof entry.photo.field !== 'string' || !entry.photo.field)) {
    throw httpError(400, 'photo.field is required');
  }
  rejectUnknownTags(entry.preentered, 'preentered');
  rejectUnknownTags(entry.barcode?.preentered, 'barcode.preentered');
}

const ALLOWED_TAGS = new Set(['br', 'b', 'bold', 'i', 'u', 'span', 'leading']);

function rejectUnknownTags(value, field) {
  if (typeof value !== 'string' || !value) return;
  const tags = value.matchAll(/<\/?([a-zA-Z][\w:-]*)\b[^>]*>/g);
  for (const match of tags) {
    const name = match[1].toLowerCase();
    if (!ALLOWED_TAGS.has(name)) throw httpError(400, `unknown tag ${name} in ${field}`);
  }
}

function validatePage(page) {
  if (!page || typeof page !== 'object' || Array.isArray(page)) throw httpError(400, 'page must be an object');
  if (page.pages !== undefined || page.metadata !== undefined) throw httpError(400, 'page must not contain pages or metadata');
  const size = page.page_size;
  if (!size || typeof size !== 'object' || Array.isArray(size)) throw httpError(400, 'page_size is required');
  if (finiteNumber(size.width, 'page_size.width') <= 0 || finiteNumber(size.height, 'page_size.height') <= 0) {
    throw httpError(400, 'page_size must be positive');
  }
  if (size.units !== 'mm' && size.units !== 'points') throw httpError(400, 'page_size.units must be mm or points');
  const bleeds = size.bleeds ?? { top: 0, right: 0, bottom: 0, left: 0 };
  for (const side of ['top', 'right', 'bottom', 'left']) {
    if (finiteNumber(bleeds[side], `bleeds.${side}`) < 0) throw httpError(400, `bleeds.${side} must not be negative`);
  }
  if (size.background !== undefined) requireColor(size.background, 'page_size.background');
  if (!Array.isArray(page.contents)) throw httpError(400, 'contents must be an array');
  const ids = new Set();
  for (const entry of page.contents) validateEntry(entry, ids);
  return page;
}

function asMillimetres(value, field) {
  if (value === null || value === undefined) throw httpError(400, `${field} is required`);
  const mm = typeof value === 'number' ? value : Number(String(value).trim());
  if (!Number.isFinite(mm)) throw httpError(400, `${field} must be a number of millimetres`);
  if (mm < 10 || mm > 1000) throw httpError(400, `${field} must be between 10 and 1000 mm`);
  return mm;
}

function marginMillimetres(value, name) {
  if (value === null || value === undefined || value === '') return 0;
  const mm = typeof value === 'number' ? value : Number(String(value).trim());
  if (!Number.isFinite(mm)) throw httpError(400, `margin ${name} must be a number of millimetres`);
  if (mm < 0) throw httpError(400, `margin ${name} must not be negative`);
  return mm;
}

export function assertPage(template) {
  const width = asMillimetres(template.page_width_mm, 'page_width_mm');
  const height = asMillimetres(template.page_height_mm, 'page_height_mm');

  const top = marginMillimetres(template.margin_top_mm, 'top');
  const right = marginMillimetres(template.margin_right_mm, 'right');
  const bottom = marginMillimetres(template.margin_bottom_mm, 'bottom');
  const left = marginMillimetres(template.margin_left_mm, 'left');
  if (left + right >= width || top + bottom >= height) {
    throw httpError(400, 'margins leave no printable area');
  }
  return { width, height };
}

function applyWrite(template, body) {
  if (!body || typeof body !== 'object' || Array.isArray(body)) {
    throw httpError(400, 'JSON object is required');
  }
  const next = structuredClone(template);
  for (const key of WRITE_KEYS) {
    if (!Object.hasOwn(body, key) || key === 'updated_at' || key === 'editor_state') continue;
    if (key === 'html') {
      if (body.html !== null && typeof body.html !== 'string') throw httpError(400, 'html must be a string');
      next.html = body.html;
      continue;
    }
    next[key] = body[key];
  }
  if (Object.hasOwn(body, 'page')) {
    next.page = structuredClone(validatePage(body.page));
    next.page_width_mm = next.page.page_size.width;
    next.page_height_mm = next.page.page_size.height;
    next.html = null;
  }
  const { width, height } = assertPage(next);
  next.page_width_mm = width;
  next.page_height_mm = height;
  next.editor_state = null;
  next.updated_at = new Date().toISOString();
  next.source = 'local';
  return next;
}

function assetExtension(filename) {
  return path.extname(path.basename(filename || '')).toLowerCase();
}

function assetStem(filename) {
  const base = path.basename(filename || '');
  const ext = path.extname(base);
  return ext ? base.slice(0, -ext.length) : base;
}

function assertAssetName(name) {
  if (!ASSET_NAME.test(name || '')) {
    throw httpError(400, 'имя файла: только буквы, цифры, точка, дефис и подчёркивание (до 100 символов)');
  }
}

function assertAssetExtension(filename) {
  const suffix = assetExtension(filename);
  if (!ASSET_EXTENSIONS.includes(suffix)) {
    throw httpError(
      400,
      `файлы ${suffix || 'без расширения'} не поддерживаются; допустимы: ${[...ASSET_EXTENSIONS].sort().join(', ')}`,
    );
  }
  return suffix;
}

function insideTemplate(templateDir, relative) {
  const rootDir = path.resolve(templateDir);
  const file = path.resolve(rootDir, relative);
  if (file !== rootDir && !file.startsWith(rootDir + path.sep)) return null;
  return file;
}

function sendError(res, error) {
  const status = error.status || 500;
  res.status(status).json({ detail: status === 500 ? 'internal error' : error.message });
}

function uploadSingle(maxBytes) {
  const parser = multer({
    storage: multer.memoryStorage(),
    limits: { fileSize: maxBytes, files: 1 },
  }).single('file');
  return (req, res, next) => {
    parser(req, res, (error) => {
      if (!error) {
        next();
        return;
      }
      if (error.code === 'LIMIT_FILE_SIZE') {
        const mb = Math.floor(maxBytes / (1024 * 1024));
        res.status(400).json({ detail: `файл больше ${mb} МБ` });
        return;
      }
      res.status(400).json({ detail: error.message });
    });
  };
}

function listTemplates(templatesDir) {
  const names = fs.readdirSync(templatesDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && templateDir(templatesDir, entry.name))
    .map((entry) => entry.name);
  const rows = [];
  for (const name of names) {
    const dir = path.join(templatesDir, name);
    const meta = readMeta(dir);
    if (!meta) continue;
    let width = meta.page_width_mm;
    let height = meta.page_height_mm;
    const yml = path.join(dir, 'template.yml');
    if (fs.existsSync(yml)) {
      const page = parse(fs.readFileSync(yml, 'utf8'));
      width = page?.page_size?.width;
      height = page?.page_size?.height;
    }
    rows.push({
      id: meta.id ?? name,
      name: meta.name,
      source: meta.source,
      is_default: meta.is_default,
      updated_at: meta.updated_at ?? '',
      updated_by_name: meta.updated_by_name ?? '',
      page_width_mm: width,
      page_height_mm: height,
    });
  }
  rows.sort((a, b) => String(a.id).localeCompare(String(b.id), undefined, { numeric: true }));
  return rows;
}

function openTemplate(templatesDir, id) {
  const dir = templateDir(templatesDir, id);
  if (!dir) return null;
  const yml = path.join(dir, 'template.yml');
  const json = path.join(dir, 'template.json');
  if (!fs.existsSync(yml) && fs.existsSync(json)) migrateJson(dir, json);
  if (!fs.existsSync(yml)) return null;
  return { file: yml, dir };
}

/** The same guest as `src/demo-context.ts`. Preview fills from this object. It does not fetch it. */
const DEMO = {
  form_data: {
    field_1: 'Петрова',
    field_2: 'Анна',
    field_3: 'ООО «Ромашка-Экспо»',
    field_4: 'Руководитель отдела маркетинга',
    field_5: ['Доклад', 'Стенд'],
    field_6: true,
  },
  ticket: { number: 'A-00042', code: '2000000000008', ticket_type_name: 'Standard' },
};

function demoLookup(name) {
  if (name === 'number') return DEMO.ticket.number;
  if (name === 'code') return DEMO.ticket.code;
  if (name === 'ticket_type_name') return DEMO.ticket.ticket_type_name;
  return DEMO.form_data[name];
}

function splitPipes(source) {
  const parts = [];
  let current = '';
  let quote = '';
  for (const ch of source) {
    if (quote) {
      current += ch;
      if (ch === quote) quote = '';
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      current += ch;
      continue;
    }
    if (ch === '|') {
      parts.push(current.trim());
      current = '';
      continue;
    }
    current += ch;
  }
  parts.push(current.trim());
  return parts.filter((part) => part !== '');
}

function pipeArgument(pipe, prefix) {
  const raw = pipe.slice(prefix.length).trim();
  if (raw.startsWith('"') && raw.endsWith('"')) {
    try {
      return JSON.parse(raw);
    } catch {
      return raw.slice(1, -1);
    }
  }
  if (raw.startsWith("'") && raw.endsWith("'")) return raw.slice(1, -1);
  return raw;
}

/** Join, then default, then case. The written order does not change that. */
export function demoFieldValue(fieldName) {
  const parts = splitPipes(fieldName);
  const rawName = parts[0] ?? '';
  const name = rawName.startsWith('form_data.') ? rawName.slice('form_data.'.length) : rawName;
  const pipes = parts.slice(1);
  const join = pipes.find((pipe) => pipe.startsWith('join:'));
  const fallback = pipes.find((pipe) => pipe.startsWith('default:'));
  const casing = pipes.find((pipe) => pipe === 'upper' || pipe === 'title');
  const raw = demoLookup(name);
  const empty = raw === undefined || raw === null || raw === '' || raw === false || raw === 0
    || (Array.isArray(raw) && raw.length === 0);
  let value;
  if (Array.isArray(raw)) {
    value = raw.join(join ? pipeArgument(join, 'join:') : ', ');
  } else if (typeof raw === 'boolean') {
    value = raw ? 'true' : 'false';
  } else if (raw === undefined || raw === null) {
    value = '';
  } else {
    value = String(raw);
  }
  if (fallback && empty) value = pipeArgument(fallback, 'default:');
  if (casing === 'upper') value = value.toUpperCase();
  if (casing === 'title') {
    value = value.replace(/\p{L}+/gu, (word) => word[0].toUpperCase() + word.slice(1).toLowerCase());
  }
  return value;
}

function entryShown(ifSt) {
  if (!ifSt) return true;
  const value = demoLookup(ifSt);
  if (value === undefined || value === null || value === '' || value === false || value === 0) return false;
  return true;
}

function fillMustaches(source) {
  return source.replace(/\{\{([^{}]*)\}\}/g, (_full, body) => demoFieldValue(body.trim()));
}

/** A copy the PDF writer can draw. The stored page is left alone. */
export function previewCopy(page) {
  const copy = structuredClone(page);
  copy.contents = [];
  for (const entry of page.contents) {
    if (!entryShown(entry.ifSt)) continue;
    const next = structuredClone(entry);
    delete next.ifSt;
    if (next.text) {
      next.alignment = {
        horizontal: next.alignment?.horizontal ?? 'left',
        vertical: next.alignment?.vertical ?? 'top',
      };
      next.padding = {
        top: next.padding?.top ?? 0,
        right: next.padding?.right ?? 0,
        bottom: next.padding?.bottom ?? 0,
        left: next.padding?.left ?? 0,
      };
    }
    if (next.barcode) {
      delete next.template;
      if (typeof next.barcode.preentered === 'string') {
        next.barcode.preentered = fillMustaches(next.barcode.preentered);
      }
    }
    copy.contents.push(next);
  }
  return copy;
}

async function zipPreview(templateDir, page) {
  const zip = new JSZip();
  zip.file('template.yml', stringify(page));
  const relatives = new Set();
  for (const font of Object.values(page.resources?.fonts ?? {})) {
    if (font?.source_path) relatives.add(font.source_path);
  }
  for (const image of Object.values(page.resources?.images ?? {})) {
    if (image?.extracted_path) relatives.add(image.extracted_path);
  }
  for (const relative of relatives) {
    const file = insideTemplate(templateDir, relative);
    if (!file || !fs.existsSync(file)) continue;
    zip.file(relative, fs.readFileSync(file));
  }
  return zip.generateAsync({ type: 'nodebuffer' });
}

export function createPdfClient(addr) {
  const definition = protoLoader.loadSync(pdfProtoPath(), {
    keepCase: true,
    longs: String,
    enums: String,
    defaults: true,
    oneofs: true,
  });
  const root = grpc.loadPackageDefinition(definition);
  const Service = root.irbis.pdf.v1.PdfTooling;
  const client = new Service(addr, grpc.credentials.createInsecure());
  return {
    loadTemplate(zip) {
      return new Promise((resolve, reject) => {
        const call = client.LoadTemplate((error, view) => (error ? reject(error) : resolve(view)));
        call.write({ part: 'meta', meta: { filename: 'template.yml' } });
        const size = 64 * 1024;
        for (let offset = 0; offset < zip.length; offset += size) {
          call.write({ part: 'data', data: zip.subarray(offset, offset + size) });
        }
        call.end();
      });
    },
    renderChunks(sessionId, values) {
      return new Promise((resolve, reject) => {
        const chunks = [];
        const call = client.RenderChunks({
          session_id: sessionId,
          rows: [{ values }],
        });
        call.on('data', (chunk) => chunks.push(Buffer.from(chunk.data)));
        call.on('error', reject);
        call.on('end', () => resolve(Buffer.concat(chunks)));
      });
    },
    close(sessionId) {
      return new Promise((resolve, reject) => {
        client.Close({ session_id: sessionId }, (error) => (error ? reject(error) : resolve()));
      });
    },
  };
}

function mapPdfError(error) {
  if (error?.status) return error;
  if (error?.code === grpc.status.INVALID_ARGUMENT) {
    return httpError(400, error.details || error.message);
  }
  return httpError(502, 'pdf-tooling unavailable');
}

export function createApp(templatesDir = defaultTemplatesDir, options = {}) {
  const maxAssetBytes = options.maxAssetBytes ?? MAX_ASSET_BYTES;
  fs.mkdirSync(templatesDir, { recursive: true });
  seedBlankTemplate(templatesDir);

  const app = express();
  app.use((req, res, next) => {
    res.set('Access-Control-Allow-Origin', allowOrigin(req.get('origin')));
    res.set('Access-Control-Allow-Methods', 'GET, PUT, POST, DELETE, OPTIONS');
    res.set('Access-Control-Allow-Headers', 'Content-Type');
    if (req.method === 'OPTIONS') {
      res.sendStatus(204);
      return;
    }
    next();
  });

  app.get('/api/print_template/list', (req, res) => {
    res.json(listTemplates(templatesDir));
  });

  let pdfClient = options.pdfClient ?? null;
  function pdf() {
    pdfClient ??= createPdfClient(pdfToolingAddr());
    return pdfClient;
  }

  app.post('/api/print_template/preview', express.json({ limit: '2mb' }), async (req, res) => {
    let sessionId = '';
    try {
      if (req.body?.output !== 'pdf') throw httpError(400, 'output must be pdf');
      const opened = openTemplate(templatesDir, String(req.body?.template_id ?? ''));
      if (!opened) throw httpError(404, 'not found');
      const page = validatePage(req.body?.document);
      const copy = previewCopy(page);
      const zip = await zipPreview(opened.dir, copy);
      const client = pdf();
      const view = await client.loadTemplate(zip);
      sessionId = view.session_id;
      const values = (view.fields ?? []).map((field) => ({
        field: field.name,
        value: demoFieldValue(field.name),
      }));
      const bytes = await client.renderChunks(sessionId, values);
      const finished = sessionId;
      sessionId = '';
      try {
        await client.close(finished);
      } catch {
        // The PDF is ready. A failed close must not hide it.
      }
      res.set('Content-Type', 'application/pdf');
      res.send(bytes);
    } catch (error) {
      if (!res.headersSent) sendError(res, mapPdfError(error));
    } finally {
      if (sessionId) {
        try {
          await pdf().close(sessionId);
        } catch {
          // The PDF bytes are already sent, or the service never kept the session.
        }
      }
    }
  });

  app.use('/api/print_template/:id', (req, res, next) => {
    const opened = openTemplate(templatesDir, req.params.id);
    if (!opened) {
      res.status(404).json({ detail: 'not found' });
      return;
    }
    req.templateFile = opened.file;
    req.templateDir = opened.dir;
    next();
  });

  app.get('/api/print_template/:id', (req, res) => {
    res.json(readTemplate(req.templateFile));
  });

  app.put('/api/print_template/:id', express.json({ limit: '2mb' }), (req, res) => {
    try {
      const next = applyWrite(readTemplate(req.templateFile), req.body);
      next.page = writeTemplate(req.templateFile, next);
      res.json(next);
    } catch (error) {
      sendError(res, error);
    }
  });

  app.post('/api/print_template/:id/assets', uploadSingle(maxAssetBytes), (req, res) => {
    try {
      if (!req.file) throw httpError(400, 'file is required');
      let name = String(req.body?.name || '').trim();
      if (!name) name = assetStem(req.file.originalname).trim();
      assertAssetName(name);
      const suffix = assertAssetExtension(req.file.originalname);
      if (req.file.size > maxAssetBytes) {
        throw httpError(400, `файл больше ${Math.floor(maxAssetBytes / (1024 * 1024))} МБ`);
      }

      const template = readTemplate(req.templateFile);
      const manifest = { ...(template.manifest || {}) };
      const previous = manifest[name];
      if (previous) {
        const oldPath = insideTemplate(req.templateDir, previous);
        if (oldPath && fs.existsSync(oldPath)) fs.unlinkSync(oldPath);
      }

      const relative = `files/${name}${suffix}`;
      const target = insideTemplate(req.templateDir, relative);
      if (!target) throw httpError(400, 'invalid asset path');
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.writeFileSync(target, req.file.buffer);

      manifest[name] = relative;
      template.manifest = manifest;
      template.updated_at = new Date().toISOString();
      writeTemplate(req.templateFile, template);
      res.status(201).json({ name, path: relative, manifest });
    } catch (error) {
      sendError(res, error);
    }
  });

  app.get('/api/print_template/:id/assets/:name', (req, res) => {
    const template = readTemplate(req.templateFile);
    const relative = template.manifest?.[req.params.name];
    const file = relative && insideTemplate(req.templateDir, relative);
    if (!file || !fs.existsSync(file)) {
      res.status(404).json({ detail: 'not found' });
      return;
    }
    const suffix = path.extname(file).toLowerCase();
    res.set('Content-Type', CONTENT_TYPES[suffix] || 'application/octet-stream');
    res.set('Content-Disposition', `inline; filename="${path.basename(file)}"`);
    res.set('Cache-Control', 'no-store');
    res.send(fs.readFileSync(file));
  });

  app.delete('/api/print_template/:id/assets/:name', (req, res) => {
    try {
      const template = readTemplate(req.templateFile);
      const manifest = { ...(template.manifest || {}) };
      const relative = manifest[req.params.name];
      if (!relative) throw httpError(404, 'not found');
      const file = insideTemplate(req.templateDir, relative);
      if (file && fs.existsSync(file)) fs.unlinkSync(file);
      delete manifest[req.params.name];
      template.manifest = manifest;
      template.updated_at = new Date().toISOString();
      writeTemplate(req.templateFile, template);
      res.json({ manifest });
    } catch (error) {
      sendError(res, error);
    }
  });

  app.use((error, _req, res, next) => {
    if (error instanceof SyntaxError && error.status === 400) {
      res.status(400).json({ detail: 'invalid json' });
      return;
    }
    next(error);
  });

  return app;
}
