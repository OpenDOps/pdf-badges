import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import JSZip from 'jszip';
import { parse, stringify } from 'yaml';
import { blankMeta, blankPage, blankTemplate, createApp, demoFieldValue, readTemplate } from './app.mjs';
import { htmlToPage } from './scripts/html-to-page.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const fontSource = path.join(here, '..', 'tests', 'fixtures', 'struct-to-pdf', 'fonts', 'LiberationSerif-Regular.ttf');

async function start(options) {
  const dir = await mkdtemp(path.join(tmpdir(), 'ticket-editor-'));
  const started = await listen(createApp(dir, options), dir);
  started.close = async function close() {
    await started.stop();
    await rm(dir, { recursive: true, force: true });
  };
  return started;
}

async function startOn(dir) {
  return listen(createApp(dir), dir);
}

async function listen(app, dir) {
  const server = app.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return {
    dir,
    base: `http://127.0.0.1:${server.address().port}`,
    async stop() {
      server.closeAllConnections?.();
      server.close();
      await once(server, 'close');
    },
    async close() {
      await this.stop();
    },
  };
}

async function storedPage(dir, id = '1') {
  return parse(await readFile(path.join(dir, id, 'template.yml'), 'utf8'));
}

async function storedMeta(dir, id = '1') {
  return parse(await readFile(path.join(dir, id, 'meta.yml'), 'utf8'));
}

function stored(dir, id = '1') {
  return readTemplate(path.join(dir, id, 'template.yml'));
}

function pngForm(name, bytes, filename) {
  const body = new FormData();
  if (name !== undefined) body.set('name', name);
  body.set('file', new Blob([bytes]), filename);
  return body;
}

test('GET /api/print_template/1 is the blank 100×100 template', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const response = await fetch(`${ctx.base}/api/print_template/1`);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('access-control-allow-origin'), 'http://localhost:4200');
  const body = await response.json();
  assert.equal(body.html, null);
  assert.equal(body.editor_state, null);
  assert.deepEqual(body.page, blankPage());
  assert.deepEqual(body, blankTemplate());
  assert.deepEqual(await storedPage(ctx.dir), blankPage());
  assert.deepEqual(await storedMeta(ctx.dir), blankMeta());
  assert.equal(fs.existsSync(path.join(ctx.dir, '1', 'template.json')), false);
  assert.equal(fs.existsSync(path.join(ctx.dir, '1', 'meta.json')), false);
  const font = path.join(ctx.dir, '1', 'fonts', 'LiberationSerif-Regular.ttf');
  assert.equal(fs.readFileSync(font).equals(fs.readFileSync(fontSource)), true);
});

test('a loopback page origin is echoed so the dev server can call the API', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const response = await fetch(`${ctx.base}/api/print_template/1`, {
    headers: { Origin: 'http://127.0.0.1:4213' },
  });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('access-control-allow-origin'), 'http://127.0.0.1:4213');
});

test('DEFAULT_FONT is the file copied into a new template', async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'ticket-editor-font-'));
  const font = path.join(dir, 'custom.ttf');
  const templates = path.join(dir, 'templates');
  fs.writeFileSync(font, Buffer.from('custom-face'));
  const copied = path.join(templates, '1', 'fonts', 'LiberationSerif-Regular.ttf');
  const child = spawn(
    process.execPath,
    [
      '--input-type=module',
      '-e',
      `import fs from 'node:fs';
       import { createApp } from ${JSON.stringify(path.join(here, 'app.mjs'))};
       createApp(${JSON.stringify(templates)});
       const got = fs.readFileSync(${JSON.stringify(copied)});
       if (!got.equals(Buffer.from('custom-face'))) process.exit(2);`,
    ],
    { env: { ...process.env, DEFAULT_FONT: font } },
  );
  const code = await new Promise((resolve, reject) => {
    child.on('error', reject);
    child.on('exit', resolve);
  });
  assert.equal(code, 0);
  assert.equal(fs.readFileSync(copied).equals(Buffer.from('custom-face')), true);
  await rm(dir, { recursive: true, force: true });
});

test('a missing id and a non-numeric id are 404', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  for (const id of ['2', 'nope']) {
    const response = await fetch(`${ctx.base}/api/print_template/${id}`);
    assert.equal(response.status, 404);
    assert.deepEqual(await response.json(), { detail: 'not found' });
  }
  assert.equal(fs.existsSync(path.join(ctx.dir, '2')), false);
});

test('OPTIONS stays open for the editor origin', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const response = await fetch(`${ctx.base}/api/print_template/1`, { method: 'OPTIONS' });
  assert.equal(response.status, 204);
  assert.equal(response.headers.get('access-control-allow-origin'), 'http://localhost:4200');
  assert.match(response.headers.get('access-control-allow-methods'), /PUT/);
});

test('PUT merges the page and ignores the optimistic lock', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const response = await fetch(`${ctx.base}/api/print_template/1`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      name: 'Badge',
      html: '<p>not stored</p>',
      editor_state: { pages: [] },
      page_width_mm: 90,
      page_height_mm: 130,
      updated_at: '2000-01-01T00:00:00.000Z',
      source: 'cloud',
    }),
  });
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.page_width_mm, 90);
  assert.equal(body.page_height_mm, 130);
  assert.equal(body.name, 'Badge');
  assert.equal(body.source, 'local');
  assert.notEqual(body.updated_at, '2000-01-01T00:00:00.000Z');
  assert.equal(Number.isNaN(Date.parse(body.updated_at)), false);
  assert.equal(body.editor_state, null);
  assert.equal(body.html, '<p>not stored</p>');
  assert.deepEqual(stored(ctx.dir), body);
  assert.equal((await storedMeta(ctx.dir)).html, '<p>not stored</p>');
  assert.equal((await storedMeta(ctx.dir)).editor_state, undefined);
  const again = await fetch(`${ctx.base}/api/print_template/1`);
  assert.equal((await again.json()).html, '<p>not stored</p>');
  const page = await storedPage(ctx.dir);
  assert.equal(page.page_size.width, 90);
  assert.equal(page.page_size.height, 130);
  assert.equal(page.page_size.units, 'mm');
  assert.deepEqual(page.contents, []);
  assert.equal(page.resources.fonts.text.font_family, 'Liberation Serif');
  assert.deepEqual((await storedMeta(ctx.dir)).manifest, {});
});

test('PUT of a 5 mm side is 400 and leaves the file unchanged', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const saved = await fetch(`${ctx.base}/api/print_template/1`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'Badge', page_width_mm: 90, page_height_mm: 130 }),
  });
  assert.equal(saved.status, 200);
  const before = await stored(ctx.dir);

  const rejected = await fetch(`${ctx.base}/api/print_template/1`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ page_width_mm: 5 }),
  });
  assert.equal(rejected.status, 400);
  const detail = await rejected.json();
  assert.equal(detail.detail, 'page_width_mm must be between 10 and 1000 mm');
  assert.deepEqual(stored(ctx.dir), before);
  const page = await storedPage(ctx.dir);
  assert.equal(page.page_size.width, 90);
  assert.equal(page.page_size.height, 130);
});

test('page size is the millimetre sides, and margins must leave a page', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const savedResponse = await fetch(`${ctx.base}/api/print_template/1`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ page_format: 'A4', page_width_mm: 90, page_height_mm: 130, orientation: 'landscape' }),
  });
  assert.equal(savedResponse.status, 200);
  const saved = await savedResponse.json();
  assert.equal(saved.page_width_mm, 90);
  assert.equal(saved.page_height_mm, 130);
  assert.equal(saved.page_format, undefined);
  assert.equal(saved.orientation, undefined);
  assert.equal(saved.ticket_type, undefined);
  assert.equal(saved.ticket_type_name, undefined);
  const page = await storedPage(ctx.dir);
  assert.equal(page.page_size.width, 90);
  assert.equal(page.page_size.height, 130);
  const meta = await storedMeta(ctx.dir);
  assert.equal(meta.page_format, undefined);
  assert.equal(meta.orientation, undefined);
  assert.equal(meta.ticket_type, undefined);
  assert.equal(meta.ticket_type_name, undefined);

  const ignored = await fetch(`${ctx.base}/api/print_template/1`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ page_format: 'roll', orientation: 'landscape' }),
  });
  assert.equal(ignored.status, 200);
  assert.equal((await storedPage(ctx.dir)).page_size.width, 90);
  assert.equal((await storedPage(ctx.dir)).page_size.height, 130);

  const margins = await fetch(`${ctx.base}/api/print_template/1`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      page_width_mm: 100,
      page_height_mm: 100,
      margin_left_mm: 60,
      margin_right_mm: 60,
    }),
  });
  assert.equal(margins.status, 400);
  assert.equal((await margins.json()).detail, 'margins leave no printable area');
  assert.equal((await storedPage(ctx.dir)).page_size.width, 90);
  assert.equal((await storedPage(ctx.dir)).page_size.height, 130);
});

test('upload, read, and delete a logo', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const uploaded = await fetch(`${ctx.base}/api/print_template/1/assets`, {
    method: 'POST',
    body: pngForm('logo', Buffer.from('PNG'), 'logo.png'),
  });
  assert.equal(uploaded.status, 201);
  const created = await uploaded.json();
  assert.equal(created.name, 'logo');
  assert.equal(created.path, 'files/logo.png');
  assert.deepEqual(created.manifest, { logo: 'files/logo.png' });
  assert.equal(fs.readFileSync(path.join(ctx.dir, '1', 'files', 'logo.png'), 'utf8'), 'PNG');
  assert.deepEqual((await storedPage(ctx.dir)).resources.images, {
    logo: { name: 'logo', extracted_path: 'files/logo.png' },
  });

  const fetched = await fetch(`${ctx.base}/api/print_template/1/assets/logo`);
  assert.equal(fetched.status, 200);
  assert.equal(fetched.headers.get('cache-control'), 'no-store');
  assert.equal(Buffer.from(await fetched.arrayBuffer()).toString(), 'PNG');

  const removed = await fetch(`${ctx.base}/api/print_template/1/assets/logo`, { method: 'DELETE' });
  assert.equal(removed.status, 200);
  assert.deepEqual(await removed.json(), { manifest: {} });
  assert.deepEqual((await storedMeta(ctx.dir)).manifest, {});
  assert.equal(fs.existsSync(path.join(ctx.dir, '1', 'files', 'logo.png')), false);

  const missing = await fetch(`${ctx.base}/api/print_template/1/assets/logo`);
  assert.equal(missing.status, 404);
  assert.deepEqual(await missing.json(), { detail: 'not found' });
});

test('an empty asset name uses the filename, and a bad extension is refused', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const named = await fetch(`${ctx.base}/api/print_template/1/assets`, {
    method: 'POST',
    body: pngForm('', Buffer.from('PNG'), 'logo.png'),
  });
  assert.equal(named.status, 201);
  assert.equal((await named.json()).name, 'logo');

  const text = new FormData();
  text.set('name', 'notes');
  text.set('file', new Blob([Buffer.from('hello')]), 'notes.txt');
  const rejected = await fetch(`${ctx.base}/api/print_template/1/assets`, { method: 'POST', body: text });
  assert.equal(rejected.status, 400);
  assert.match((await rejected.json()).detail, /\.txt/);
  assert.deepEqual((await storedMeta(ctx.dir)).manifest, { logo: 'files/logo.png' });
});

test('replacing an asset deletes the previous file', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const first = await fetch(`${ctx.base}/api/print_template/1/assets`, {
    method: 'POST',
    body: pngForm('логотип', Buffer.from('PNG'), 'logo.png'),
  });
  assert.equal(first.status, 201);

  const jpeg = new FormData();
  jpeg.set('name', 'логотип');
  jpeg.set('file', new Blob([Buffer.from('JPG')]), 'logo.jpeg');
  const second = await fetch(`${ctx.base}/api/print_template/1/assets`, { method: 'POST', body: jpeg });
  assert.equal(second.status, 201);
  assert.deepEqual((await second.json()).manifest, { 'логотип': 'files/логотип.jpeg' });
  assert.equal(fs.existsSync(path.join(ctx.dir, '1', 'files', 'логотип.png')), false);
  assert.equal(fs.readFileSync(path.join(ctx.dir, '1', 'files', 'логотип.jpeg'), 'utf8'), 'JPG');
});

test('an asset over the size limit is 400', async (t) => {
  const ctx = await start({ maxAssetBytes: 4 });
  t.after(() => ctx.close());

  const uploaded = await fetch(`${ctx.base}/api/print_template/1/assets`, {
    method: 'POST',
    body: pngForm('logo', Buffer.from('PNG!!'), 'logo.png'),
  });
  assert.equal(uploaded.status, 400);
  assert.match((await uploaded.json()).detail, /МБ/);
  assert.deepEqual((await storedMeta(ctx.dir)).manifest, {});
});

test('startup migrates template.json once and does not overwrite template.yml', async (t) => {
  const dir = await mkdtemp(path.join(tmpdir(), 'ticket-editor-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const json = path.join(dir, '1', 'template.json');
  fs.mkdirSync(path.dirname(json), { recursive: true });
  const existing = {
    ...blankTemplate(),
    name: 'Kept',
    page_width_mm: 90,
    page_height_mm: 130,
    html: '<section>badge</section>',
    editor_state: { pages: [1] },
  };
  fs.writeFileSync(json, JSON.stringify(existing));

  const app = createApp(dir);
  const server = app.listen(0, '127.0.0.1');
  t.after(() => new Promise((resolve) => server.close(resolve)));
  await once(server, 'listening');
  const base = `http://127.0.0.1:${server.address().port}`;
  const response = await fetch(`${base}/api/print_template/1`);
  const migrated = await response.json();
  assert.equal(migrated.name, 'Kept');
  assert.equal(migrated.html, '<section>badge</section>');
  assert.equal(migrated.editor_state, null);
  assert.equal(fs.existsSync(json), true);
  assert.equal(JSON.parse(fs.readFileSync(json, 'utf8')).html, '<section>badge</section>');
  assert.equal((await storedMeta(dir)).html, '<section>badge</section>');
  const page = await storedPage(dir);
  assert.equal(page.page_size.width, 90);
  assert.equal(page.page_size.height, 130);
  assert.equal(fs.existsSync(path.join(dir, '1', 'fonts', 'LiberationSerif-Regular.ttf')), true);

  const yml = path.join(dir, '1', 'template.yml');
  const kept = parse(fs.readFileSync(yml, 'utf8'));
  kept.page_size.bleeds = { top: 3, right: 3, bottom: 3, left: 3 };
  fs.writeFileSync(yml, `${JSON.stringify(kept)}\n`);
  const renamed = await fetch(`${base}/api/print_template/1`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'Still kept' }),
  });
  assert.equal(renamed.status, 200);
  assert.deepEqual((await storedPage(dir)).page_size.bleeds, { top: 3, right: 3, bottom: 3, left: 3 });
  assert.equal((await storedMeta(dir)).name, 'Still kept');
  assert.equal((await storedMeta(dir)).html, '<section>badge</section>');
  assert.equal((await renamed.json()).html, '<section>badge</section>');
  assert.equal(fs.existsSync(json), true);
});

function threeEntryPage() {
  const page = blankPage();
  page.contents = [
    {
      id: 'p1-t1',
      posX: 10,
      posY: 20,
      width: 80,
      height: 12,
      template: true,
      preentered: '{{field_1|upper}}',
      text: { font: 'text', font_size: 6.35, leading: 0, line_height: 7.62 },
    },
    {
      id: 'p1-b1',
      posX: 10,
      posY: 40,
      width: 25,
      height: 25,
      template: true,
      barcode: { type: 'qr', preentered: '{{code}}' },
    },
    {
      id: 'p1-i1',
      posX: 60,
      posY: 40,
      width: 30,
      height: 20,
      image: { resource_name: 'logo' },
    },
  ];
  return page;
}

async function putJson(base, body) {
  return fetch(`${base}/api/print_template/1`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

test('PUT writes the page object and leaves the file alone when the writer would reject it', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const uploaded = await fetch(`${ctx.base}/api/print_template/1/assets`, {
    method: 'POST',
    body: pngForm('logo', Buffer.from('PNG'), 'logo.png'),
  });
  assert.equal(uploaded.status, 201);

  const kept = await putJson(ctx.base, { html: '<p>badge</p>' });
  assert.equal(kept.status, 200);
  assert.equal((await kept.json()).html, '<p>badge</p>');

  const first = await putJson(ctx.base, { page: threeEntryPage() });
  assert.equal(first.status, 200);
  const written = await first.json();
  assert.equal(written.html, null);
  assert.equal((await storedMeta(ctx.dir)).html, undefined);
  assert.equal(written.editor_state, null);
  const page = await storedPage(ctx.dir);
  assert.deepEqual(written.page, page);
  assert.deepEqual(
    page.contents.map((entry) => entry.id),
    ['p1-t1', 'p1-b1', 'p1-i1'],
  );
  assert.equal(page.contents[0].preentered, '{{field_1|upper}}');
  assert.equal(page.contents[1].barcode.type, 'qr');
  assert.equal(page.contents[1].barcode.preentered, '{{code}}');
  assert.equal(page.resources.images.logo.extracted_path, 'files/logo.png');
  assert.equal(page.resources.fonts.text.font_family, 'Liberation Serif');

  const added = structuredClone(page);
  added.contents.push({
    id: 'p1-t2',
    posX: 10,
    posY: 56,
    width: 80,
    height: 12,
    template: true,
    preentered: '{{field_2}}',
    text: { font: 'text', font_size: 6.35, leading: 0, line_height: 7.62 },
  });
  const second = await putJson(ctx.base, { page: added });
  assert.equal(second.status, 200);
  const withField = await storedPage(ctx.dir);
  assert.deepEqual(
    withField.contents.map((entry) => entry.id),
    ['p1-t1', 'p1-b1', 'p1-i1', 'p1-t2'],
  );
  assert.equal(withField.contents[0].preentered, '{{field_1|upper}}');
  assert.equal(withField.contents[3].preentered, '{{field_2}}');
  assert.equal(withField.contents[3].template, true);

  const yml = path.join(ctx.dir, '1', 'template.yml');
  const before = await readFile(yml, 'utf8');
  const rejectedPage = structuredClone(withField);
  rejectedPage.contents[0].background = 'orange';
  const rejected = await putJson(ctx.base, { page: rejectedPage });
  assert.equal(rejected.status, 400);
  assert.match((await rejected.json()).detail, /#RRGGBB/);
  assert.equal(await readFile(yml, 'utf8'), before);
});

test('PUT of one variable stores posX, template, and the mustache', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const page = blankPage();
  page.contents = [
    {
      id: 'p1-t1',
      posX: 12,
      posY: 8,
      width: 70,
      height: 10,
      template: true,
      preentered: '{{field_1}}',
      text: { font: 'text', font_size: 4, leading: 0, line_height: 4.8 },
    },
  ];
  const response = await putJson(ctx.base, { page });
  assert.equal(response.status, 200);
  const yml = await readFile(path.join(ctx.dir, '1', 'template.yml'), 'utf8');
  assert.match(yml, /posX: 12/);
  assert.match(yml, /template: true/);
  assert.match(yml, /\{\{field_1\}\}/);
});

test('a posX-only write leaves preentered byte for byte, spans included', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const source = 'Hello <span color="#112233">{{field_1}}</span>';
  const page = blankPage();
  page.contents = [
    {
      id: 'p1-t1',
      posX: 10,
      posY: 20,
      width: 80,
      height: 12,
      template: true,
      preentered: source,
      text: { font: 'text', font_size: 6.35, leading: 0, line_height: 7.62 },
    },
  ];
  const first = await putJson(ctx.base, { page });
  assert.equal(first.status, 200);

  const moved = structuredClone(await storedPage(ctx.dir));
  moved.contents[0].posX = 18;
  const second = await putJson(ctx.base, { page: moved });
  assert.equal(second.status, 200);
  const stored = await storedPage(ctx.dir);
  assert.equal(stored.contents[0].posX, 18);
  assert.equal(stored.contents[0].preentered, source);
});

test('an uploaded font is added beside the text face', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const body = new FormData();
  body.set('name', 'face');
  body.set('file', new Blob([Buffer.from('font')]), 'face.ttf');
  const uploaded = await fetch(`${ctx.base}/api/print_template/1/assets`, { method: 'POST', body });
  assert.equal(uploaded.status, 201);

  const page = await storedPage(ctx.dir);
  assert.equal(page.resources.fonts.text.font_family, 'Liberation Serif');
  assert.equal(page.resources.fonts.face.font_family, 'face');
  assert.equal(page.resources.fonts.face.source_path, 'files/face.ttf');
  assert.equal(page.resources.images.face, undefined);
});

test('an unknown tag in preentered is rejected and the file stays', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const page = blankPage();
  page.contents = [
    {
      id: 'p1-t1',
      posX: 10,
      posY: 10,
      width: 40,
      height: 10,
      template: true,
      preentered: '{{field_1}}',
      text: { font: 'text', font_size: 4, leading: 0, line_height: 4.8 },
    },
  ];
  assert.equal((await putJson(ctx.base, { page })).status, 200);
  const yml = path.join(ctx.dir, '1', 'template.yml');
  const before = await readFile(yml, 'utf8');
  const rejected = structuredClone(page);
  rejected.contents[0].preentered = '<div>{{field_1}}</div>';
  const response = await putJson(ctx.base, { page: rejected });
  assert.equal(response.status, 400);
  assert.match((await response.json()).detail, /unknown tag div/);
  assert.equal(await readFile(yml, 'utf8'), before);
});

test('html-to-page converts sample.html into one text box', async () => {
  const out = await mkdtemp(path.join(tmpdir(), 'html-to-page-'));
  const { page } = htmlToPage(path.join(here, 'fixtures/pir/sample.html'), out);
  assert.equal(page.page_size.width, 90);
  assert.equal(page.page_size.height, 130);
  assert.deepEqual(page.page_size.bleeds, { top: 0, right: 0, bottom: 0, left: 0 });
  assert.equal(page.contents.length, 1);
  const box = page.contents[0];
  assert.equal(box.posX, 13);
  assert.equal(box.posY, 49.5);
  assert.equal(box.width, 64);
  assert.equal(box.height, 37.5);
  assert.ok(Math.abs(box.text.font_size - 8.467) < 0.01);
  assert.equal(box.text.color, '#FF6A00');
  assert.equal(box.template, true);
  assert.equal(box.text.content, null);
  assert.match(box.text.preentered, /\{\{company\}\}/);
  assert.match(box.text.preentered, /\{\{surname\}\}/);
  assert.match(box.text.preentered, /\{\{name\}\}/);
  await rm(out, { recursive: true, force: true });
});

test('html-to-page places the PIR sheet image first at the page size', async () => {
  const out = await mkdtemp(path.join(tmpdir(), 'html-to-page-pir-'));
  const { page, meta, report } = htmlToPage(path.join(here, 'fixtures/pir/unpacked'), out);
  assert.equal(page.page_size.width, 210);
  assert.equal(page.page_size.height, 297);
  assert.equal(page.contents.length, 5);
  assert.deepEqual(page.contents[0].image, { resource_name: 'ticket_visitor_pirexpo2026' });
  assert.equal(meta.margin_left_mm, 2);
  assert.equal(page.contents[0].posX, 0);
  assert.equal(page.contents[0].posY, 0);
  assert.equal(page.contents[0].width, 210);
  assert.equal(page.contents[0].height, 297);
  assert.equal(page.contents[1].posX, 110);
  assert.equal(page.contents[1].posY, 100);
  assert.equal(page.contents[1].width, 100);
  assert.equal(page.contents[1].height, 7);
  for (const entry of page.contents) {
    assert.ok(entry.posX >= 0 && entry.posY >= 0);
    assert.ok(entry.posX + entry.width <= page.page_size.width);
    assert.ok(entry.posY + entry.height <= page.page_size.height);
  }
  assert.match(page.contents[1].text.preentered, /\{\{field_108\}\}/);
  assert.match(page.contents[1].text.preentered, /\{\{field_107\}\}/);
  assert.equal(page.contents[3].barcode.type, 'ean13');
  assert.equal(page.contents[4].barcode.type, 'qr');
  assert.equal(page.contents[4].barcode.preentered, '{{vcard}}');
  assert.equal(fs.existsSync(path.join(out, 'files/56_a4e54ef7-ce30-45b3-8352-4e92aa7c6ab7.jpg')), true);
  assert.equal(report.skipped.some((line) => line.includes('ticket_visitor_pirexpo2026')), false);
  await rm(out, { recursive: true, force: true });
});

test('meta.json is read until a save writes meta.yml', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());
  const dir = path.join(ctx.dir, '2');
  fs.mkdirSync(dir);
  fs.writeFileSync(path.join(dir, 'meta.json'), `${JSON.stringify({ ...blankMeta(), id: 2, name: 'From JSON' })}\n`);
  fs.writeFileSync(path.join(dir, 'template.yml'), stringify(blankPage()));

  const loaded = await fetch(`${ctx.base}/api/print_template/2`);
  assert.equal(loaded.status, 200);
  assert.equal((await loaded.json()).name, 'From JSON');
  const listed = await fetch(`${ctx.base}/api/print_template/list`);
  assert.equal((await listed.json()).some((row) => row.id === 2 && row.name === 'From JSON'), true);

  const saved = await fetch(`${ctx.base}/api/print_template/2`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'From YAML' }),
  });
  assert.equal(saved.status, 200);
  assert.equal(fs.existsSync(path.join(dir, 'meta.json')), false);
  assert.equal((await storedMeta(ctx.dir, '2')).name, 'From YAML');
});

test('GET /api/print_template/list returns templates that have meta.yml or meta.json', async (t) => {
  const ctx = await start();
  t.after(() => ctx.close());

  const seeded = await fetch(`${ctx.base}/api/print_template/list`);
  assert.equal(seeded.status, 200);
  assert.deepEqual(await seeded.json(), [{
    id: 1,
    name: 'Badge',
    source: 'local',
    is_default: true,
    updated_at: '',
    updated_by_name: '',
    page_width_mm: 100,
    page_height_mm: 100,
  }]);

  function writeDir(id, meta, page) {
    const dir = path.join(ctx.dir, String(id));
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'meta.json'), `${JSON.stringify(meta, null, 2)}\n`);
    if (page) fs.writeFileSync(path.join(dir, 'template.yml'), stringify(page));
  }

  const visitor = blankPage();
  visitor.page_size.width = 105;
  visitor.page_size.height = 148;
  writeDir(2, {
    ...blankMeta(),
    id: 2,
    name: 'Visitor',
    ticket_type: 3,
    ticket_type_name: 'Guest',
    is_default: false,
    updated_at: '2026-01-02T00:00:00.000Z',
    updated_by_name: 'Ada',
    page_format: 'A6',
    orientation: 'landscape',
    page_width_mm: 10,
    page_height_mm: 10,
  }, visitor);

  writeDir('pir', { ...blankMeta(), id: 'pir', name: 'PIR' }, blankPage());
  fs.mkdirSync(path.join(ctx.dir, '3'));
  fs.writeFileSync(path.join(ctx.dir, '3', 'template.yml'), stringify(blankPage()));
  writeDir(10, {
    ...blankMeta(),
    id: 10,
    name: 'Meta only',
    page_width_mm: 58,
    page_height_mm: 40,
  });

  const listed = await fetch(`${ctx.base}/api/print_template/list`);
  assert.equal(listed.status, 200);
  const rows = await listed.json();
  assert.deepEqual(rows.map((row) => row.id), [1, 2, 10, 'pir']);
  assert.equal(rows[1].name, 'Visitor');
  assert.equal(rows[1].ticket_type, undefined);
  assert.equal(rows[1].ticket_type_name, undefined);
  assert.equal(rows[1].page_format, undefined);
  assert.equal(rows[1].orientation, undefined);
  assert.equal(rows[1].page_width_mm, 105);
  assert.equal(rows[1].page_height_mm, 148);
  assert.equal(rows[1].updated_by_name, 'Ada');
  assert.equal(rows[2].page_width_mm, 58);
  assert.equal(rows[2].page_height_mm, 40);
  assert.equal(rows[3].name, 'PIR');
  assert.equal(rows[3].id, 'pir');

  const detail = await fetch(`${ctx.base}/api/print_template/1`);
  assert.equal(detail.status, 200);
  assert.deepEqual(await detail.json(), blankTemplate());
});

test('GET /api/print_template/pir serves the imported page and its sheet image', async (t) => {
  const dir = await mkdtemp(path.join(tmpdir(), 'ticket-editor-pir-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  htmlToPage(path.join(here, 'fixtures/pir/unpacked'), path.join(dir, 'pir'), { id: 'pir' });
  const ctx = await startOn(dir);
  t.after(() => ctx.close());

  const response = await fetch(`${ctx.base}/api/print_template/pir`);
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.html, null);
  assert.equal(body.editor_state, null);
  assert.equal(body.page.page_size.width, 210);
  assert.equal(body.page.page_size.height, 297);
  assert.equal(body.page.contents.length, 5);
  assert.equal(body.page.contents[0].image.resource_name, 'ticket_visitor_pirexpo2026');
  assert.equal(body.page.contents[0].posX, 0);
  assert.equal(body.page.contents[0].posY, 0);
  assert.equal(body.page.contents[0].width, 210);
  assert.equal(body.page.contents[0].height, 297);
  assert.ok(body.page.contents.some((entry) => String(entry.text?.preentered ?? '').includes('{{')));
  assert.equal(body.manifest.ticket_visitor_pirexpo2026, body.page.resources.images.ticket_visitor_pirexpo2026.extracted_path);
  assert.equal(fs.existsSync(path.join(dir, 'pir', 'fonts', 'LiberationSerif-Regular.ttf')), true);

  const asset = await fetch(`${ctx.base}/api/print_template/pir/assets/ticket_visitor_pirexpo2026`);
  assert.equal(asset.status, 200);
  assert.equal(asset.headers.get('content-type'), 'image/jpeg');
  const bytes = Buffer.from(await asset.arrayBuffer());
  const stored = fs.readFileSync(path.join(dir, 'pir', 'files/56_a4e54ef7-ce30-45b3-8352-4e92aa7c6ab7.jpg'));
  assert.equal(bytes.equals(stored), true);
});

test('demo values apply join, then default, then case', () => {
  assert.equal(demoFieldValue('field_1|upper'), 'ПЕТРОВА');
  assert.equal(demoFieldValue('field_4|title'), 'Руководитель Отдела Маркетинга');
  assert.equal(demoFieldValue('field_5|join:", "|upper'), 'ДОКЛАД, СТЕНД');
  assert.equal(demoFieldValue('field_9|default:"Гость"|upper'), 'ГОСТЬ');
  assert.equal(demoFieldValue('field_6'), 'true');
});

function previewPage() {
  const page = blankPage();
  page.contents = [
    {
      id: 'p1-t1',
      posX: 10,
      posY: 20,
      width: 80,
      height: 12,
      template: true,
      text: {
        font: 'text',
        font_size: 6.35,
        leading: 0,
        line_height: 7.62,
        preentered: '{{field_1|upper}}',
        content: null,
      },
    },
    {
      id: 'p1-b1',
      posX: 10,
      posY: 40,
      width: 25,
      height: 25,
      template: true,
      barcode: { type: 'qr', preentered: '{{code}}', content: null },
    },
    {
      id: 'p1-t2',
      posX: 10,
      posY: 70,
      width: 40,
      height: 10,
      ifSt: 'missing',
      template: true,
      text: {
        font: 'text',
        font_size: 4,
        leading: 0,
        line_height: 5,
        preentered: 'hidden',
        content: null,
      },
    },
  ];
  return page;
}

test('preview zips the posted page, fills the row, and leaves the stored file', async () => {
  const calls = [];
  const pdfClient = {
    async loadTemplate(zip) {
      calls.push(['load', zip]);
      return {
        session_id: 's1',
        fields: [
          { name: 'field_1|upper', box_ids: ['p1-t1'] },
          { name: 'field_5|join:", "|upper', box_ids: [] },
        ],
      };
    },
    async renderChunks(sessionId, values) {
      calls.push(['render', sessionId, values]);
      return Buffer.from('%PDF-1.4\n');
    },
    async close(sessionId) {
      calls.push(['close', sessionId]);
    },
  };
  const ctx = await start({ pdfClient });
  try {
    const page = previewPage();
    const saved = await fetch(`${ctx.base}/api/print_template/1`, {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ page }),
    });
    assert.equal(saved.status, 200);
    const before = fs.readFileSync(path.join(ctx.dir, '1', 'template.yml'), 'utf8');

    const response = await fetch(`${ctx.base}/api/print_template/preview`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ template_id: 1, output: 'pdf', document: page }),
    });
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('content-type'), 'application/pdf');
    assert.equal(Buffer.from(await response.arrayBuffer()).subarray(0, 5).toString(), '%PDF-');

    const packed = await JSZip.loadAsync(calls[0][1]);
    const copy = parse(await packed.file('template.yml').async('string'));
    assert.equal(copy.contents[0].text.preentered, '{{field_1|upper}}');
    const barcode = copy.contents.find((entry) => entry.barcode);
    assert.equal(barcode.barcode.preentered, '2000000000008');
    assert.equal(barcode.template, undefined);
    assert.equal(copy.contents.some((entry) => entry.ifSt === 'missing'), false);
    assert.equal(await packed.file('fonts/LiberationSerif-Regular.ttf').async('nodebuffer').then((bytes) => bytes.length > 0), true);

    assert.deepEqual(calls[1], ['render', 's1', [
      { field: 'field_1|upper', value: 'ПЕТРОВА' },
      { field: 'field_5|join:", "|upper', value: 'ДОКЛАД, СТЕНД' },
    ]]);
    assert.deepEqual(calls[2], ['close', 's1']);
    assert.equal(fs.readFileSync(path.join(ctx.dir, '1', 'template.yml'), 'utf8'), before);
    assert.match(before, /ifSt: missing/);
  } finally {
    await ctx.close();
  }
});

test('a page the writer would reject never calls pdf-tooling', async () => {
  let called = false;
  const ctx = await start({
    pdfClient: {
      async loadTemplate() { called = true; },
      async renderChunks() { called = true; },
      async close() { called = true; },
    },
  });
  try {
    const page = blankPage();
    page.page_size.background = 'orange';
    const response = await fetch(`${ctx.base}/api/print_template/preview`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ template_id: 1, output: 'pdf', document: page }),
    });
    assert.equal(response.status, 400);
    const body = await response.json();
    assert.equal(typeof body.detail, 'string');
    assert.equal(called, false);
  } finally {
    await ctx.close();
  }
});

test('preview rejects an output other than pdf', async () => {
  const ctx = await start({ pdfClient: {} });
  try {
    const response = await fetch(`${ctx.base}/api/print_template/preview`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ template_id: 1, output: 'html', document: blankPage() }),
    });
    assert.equal(response.status, 400);
  } finally {
    await ctx.close();
  }
});

test('an invalid template from pdf-tooling is 400 and the session is closed', async () => {
  const calls = [];
  const ctx = await start({
    pdfClient: {
      async loadTemplate() {
        return { session_id: 's1', fields: [] };
      },
      async renderChunks() {
        const error = new Error('3 INVALID_ARGUMENT: unknown field');
        error.code = 3;
        error.details = 'unknown field';
        throw error;
      },
      async close(sessionId) {
        calls.push(sessionId);
      },
    },
  });
  try {
    const response = await fetch(`${ctx.base}/api/print_template/preview`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ template_id: 1, output: 'pdf', document: blankPage() }),
    });
    assert.equal(response.status, 400);
    assert.deepEqual(await response.json(), { detail: 'unknown field' });
    assert.deepEqual(calls, ['s1']);
  } finally {
    await ctx.close();
  }
});

test('an unreachable pdf-tooling is 502', async () => {
  const ctx = await start({
    pdfClient: {
      async loadTemplate() {
        const error = new Error('14 UNAVAILABLE: connect failed');
        error.code = 14;
        throw error;
      },
    },
  });
  try {
    const response = await fetch(`${ctx.base}/api/print_template/preview`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ template_id: 1, output: 'pdf', document: blankPage() }),
    });
    assert.equal(response.status, 502);
    assert.deepEqual(await response.json(), { detail: 'pdf-tooling unavailable' });
  } finally {
    await ctx.close();
  }
});
