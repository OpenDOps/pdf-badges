import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { parse } from 'yaml';
import { templatesDir } from './paths.mjs';

const api = 'http://127.0.0.1:4310/api/print_template/1';

function textBox(id, preentered, pos) {
  return {
    id,
    posX: pos?.x ?? 10,
    posY: pos?.y ?? 20,
    width: 80,
    height: 12,
    template: true,
    preentered,
    text: { font: 'text', font_size: 6.35, leading: 0, line_height: 7.62 },
  };
}

function badgePage() {
  return {
    page_size: {
      width: 100,
      height: 100,
      units: 'mm',
      bleeds: { top: 0, right: 0, bottom: 0, left: 0 },
    },
    resources: { images: {}, fonts: {} },
    contents: [
      textBox('p1-t1', '{{field_1|upper}}', { x: 10, y: 20 }),
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
      textBox('p1-t2', 'Hello <span color="#112233">{{field_2}}</span>', { x: 10, y: 56 }),
    ],
  };
}

async function putPage(request, page) {
  const response = await request.put(api, { data: { page } });
  expect(response.status()).toBe(200);
  return response.json();
}

async function storedYaml() {
  return readFile(path.join(templatesDir, '1', 'template.yml'), 'utf8');
}

function trait(page, label) {
  return page.locator('.gjs-trt-trait').filter({ has: page.getByText(label, { exact: true }) });
}

function canvasFrames(page) {
  return page.frames().filter((frame) => frame !== page.mainFrame());
}

async function selectCanvasText(page, text) {
  for (const frame of canvasFrames(page)) {
    const loc = frame.getByText(text, { exact: true });
    if (await loc.count()) {
      await loc.click({ force: true });
      return;
    }
  }
  throw new Error(`canvas text not clickable: ${text}`);
}

async function canvasText(page) {
  const chunks = [];
  for (const frame of canvasFrames(page)) {
    chunks.push(await frame.locator('body').textContent().catch(() => ''));
  }
  return chunks.join('\n');
}

async function codeText(page) {
  const scroller = page.locator('.cm-scroller');
  await scroller.waitFor({ state: 'visible' });
  return scroller.evaluate(async (el) => {
    const lines = new Set();
    const step = Math.max(40, el.clientHeight - 20);
    for (let top = 0; top <= el.scrollHeight + step; top += step) {
      el.scrollTop = top;
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      for (const line of el.querySelectorAll('.cm-line')) lines.add(line.textContent ?? '');
    }
    return [...lines].join('\n');
  });
}

test.describe.configure({ mode: 'serial' });

test('PUT writes the three entries, keeps a new one, and rejects orange', async ({ request }) => {
  const logo = await request.post('http://127.0.0.1:4310/api/print_template/1/assets', {
    multipart: {
      name: 'logo',
      file: { name: 'logo.png', mimeType: 'image/png', buffer: Buffer.from('PNG') },
    },
  });
  expect(logo.status()).toBe(201);

  await putPage(request, badgePage());
  const first = parse(await storedYaml());
  expect(first.contents.map((entry) => entry.id)).toEqual(['p1-t1', 'p1-b1', 'p1-i1', 'p1-t2']);
  expect(first.contents[0].preentered).toBe('{{field_1|upper}}');
  expect(first.contents[1].barcode.type).toBe('qr');
  expect(first.resources.images.logo.extracted_path).toBe('files/logo.png');

  const added = structuredClone(first);
  added.contents.push(textBox('p1-t3', '{{field_3}}', { x: 10, y: 70 }));
  await putPage(request, added);
  const second = parse(await storedYaml());
  expect(second.contents.map((entry) => entry.id)).toEqual(['p1-t1', 'p1-b1', 'p1-i1', 'p1-t2', 'p1-t3']);
  expect(second.contents[0].preentered).toBe('{{field_1|upper}}');
  expect(second.contents[4].preentered).toBe('{{field_3}}');
  expect(second.contents[4].template).toBe(true);

  const before = await storedYaml();
  const rejected = structuredClone(second);
  rejected.contents[0].background = 'orange';
  const response = await request.put(api, { data: { page: rejected } });
  expect(response.status()).toBe(400);
  expect((await response.json()).detail).toMatch(/#RRGGBB/);
  expect(await storedYaml()).toBe(before);
});

test('PUT of one variable stores posX, template, and the mustache', async ({ request }) => {
  const page = badgePage();
  page.contents = [textBox('p1-t1', '{{field_1}}', { x: 12, y: 8 })];
  page.contents[0].width = 70;
  page.contents[0].height = 10;
  await putPage(request, page);
  const yml = await storedYaml();
  expect(yml).toMatch(/posX: 12/);
  expect(yml).toMatch(/template: true/);
  expect(yml).toContain('{{field_1}}');
});

test('the editor save keeps preentered and stores a new page width', async ({ page, request }) => {
  test.setTimeout(150_000);
  const source = 'Hello <span color="#112233">{{field_2}}</span>';
  const seeded = await request.put(api, { data: { name: 'Badge', page: badgePage() } });
  expect(seeded.status()).toBe(200);

  page.on('pageerror', (error) => console.log('PAGEERROR', error.message));
  await page.goto('/editor/1');
  await expect(page.locator('#template-name')).toBeVisible({ timeout: 45_000 });
  await expect(page.locator('#template-name')).toHaveValue('Badge');

  await page.locator('#template-name').fill('Kept');
  const renamed = page.waitForResponse(
    (response) => response.url().endsWith('/api/print_template/1') && response.request().method() === 'PUT',
  );
  await page.getByRole('button', { name: 'Сохранить' }).click();
  expect((await renamed).status()).toBe(200);

  let stored = parse(await storedYaml());
  expect(stored.contents[0].preentered).toBe('{{field_1|upper}}');
  expect(stored.contents[3].preentered).toBe(source);
  expect(stored.contents[0].posX).toBe(10);

  await page.locator('#page-width').fill('90');
  const resized = page.waitForResponse(
    (response) => response.url().endsWith('/api/print_template/1') && response.request().method() === 'PUT',
  );
  await page.getByRole('button', { name: 'Сохранить' }).click();
  expect((await resized).status()).toBe(200);

  stored = parse(await storedYaml());
  expect(stored.page_size.width).toBe(90);
  expect(stored.contents[0].preentered).toBe('{{field_1|upper}}');
  expect(stored.contents[3].preentered).toBe(source);

  await page.reload();
  await expect(page.locator('#template-name')).toHaveValue('Kept');
  await expect(page.locator('#page-width')).toHaveValue('90');
});

test('the canvas writes traits and bleeds into the page', async ({ page, request }) => {
  test.setTimeout(150_000);
  const empty = {
    page_size: { width: 100, height: 100, units: 'mm', bleeds: { top: 0, right: 0, bottom: 0, left: 0 } },
    resources: { images: {}, fonts: { text: { font_family: 'Liberation Serif' } } },
    contents: [
      {
        id: 'p1-t1',
        posX: 5,
        posY: 5,
        width: 40,
        height: 10,
        template: true,
        preentered: '{{field_1}}',
        content: null,
        alignment: { horizontal: 'left', vertical: 'top' },
        padding: { top: 0, right: 0, bottom: 0, left: 0 },
        text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
      },
    ],
  };
  const seeded = await request.put(api, {
    data: { name: 'Canvas', margin_top_mm: 4, margin_right_mm: 0, margin_bottom_mm: 0, margin_left_mm: 0, page: empty },
  });
  expect(seeded.status()).toBe(200);

  page.on('pageerror', (error) => console.log('PAGEERROR', error.message));
  await page.goto('/editor/1');
  await expect(page.locator('#template-name')).toBeVisible({ timeout: 45_000 });
  await expect(page.locator('#template-name')).toHaveValue('Canvas');
  await expect(page.getByText('Фон', { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText('Рамка', { exact: true })).toBeVisible();
  await expect(page.getByText('Скругление')).toBeVisible();
  await expect(page.getByText('Прозрачность')).toBeVisible();
  await expect(page.getByText('Межбуквенный')).toBeVisible();
  await expect(page.getByText('Масштаб X')).toBeVisible();
  await expect(page.getByText('Наклон X')).toBeVisible();
  await expect(page.getByText('Поворот')).toBeVisible();
  await expect(page.getByText('Перенос строк')).toHaveCount(0);
  await expect(page.locator('.gjs-sm-sectors option', { hasText: /^%$/ })).toHaveCount(0);

  await expect.poll(async () => canvasText(page)).toContain('Петрова');
  await selectCanvasText(page, 'Петрова');
  await expect(page.getByText('По вертикали')).toBeVisible();
  await expect(page.getByText('Уместить текст')).toBeVisible();
  await expect(page.getByText('Интервал, mm')).toBeVisible();
  await expect(trait(page, 'Шрифт').locator('select')).toContainText('Liberation Serif');

  await expect.poll(async () => canvasText(page)).toContain('Петрова');

  await page.getByRole('button', { name: 'Код' }).click();
  await expect.poll(async () => codeText(page)).toContain('{{field_1}}');
  await page.getByRole('button', { name: 'Визуально' }).click();
  await expect.poll(async () => canvasText(page)).toContain('Петрова');
  await selectCanvasText(page, 'Петрова');
  await expect(page.getByText('По вертикали')).toBeVisible();

  await trait(page, 'Регистр').locator('select').selectOption('upper');
  await expect.poll(async () => canvasText(page)).toContain('ПЕТРОВА');
  await expect(page.getByText('По вертикали', { exact: true })).toBeVisible();
  await trait(page, 'По вертикали').locator('select').selectOption('middle');
  const fit = trait(page, 'Уместить текст');
  await expect(fit).toBeVisible();
  await fit.locator('input[type="checkbox"]').evaluate((el) => {
    el.checked = true;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  });
  const condition = trait(page, 'Показывать если').locator('input');
  await condition.fill('field_6');
  await condition.press('Tab');

  await page.locator('#bleed-top').fill('3');
  await page.locator('#bleed-right').fill('3');
  await page.locator('#bleed-bottom').fill('3');
  await page.locator('#bleed-left').fill('3');

  const saved = page.waitForResponse(
    (response) => response.url().endsWith('/api/print_template/1') && response.request().method() === 'PUT',
  );
  await page.getByRole('button', { name: 'Сохранить' }).click();
  expect((await saved).status()).toBe(200);

  const stored = parse(await storedYaml());
  expect(stored.contents).toHaveLength(1);
  expect(stored.contents[0].id).toBe('p1-t1');
  expect(stored.contents[0].preentered).toBe('{{field_1|upper}}');
  expect(stored.contents[0].template).toBe(true);
  expect(stored.contents[0].content).toBeNull();
  expect(stored.contents[0].text.font).toBe('text');
  expect(stored.contents[0].text.leading).toBe(0);
  expect(stored.contents[0].alignment.vertical).toBe('middle');
  expect(stored.contents[0].auto_scale).toBe(true);
  expect(stored.contents[0].ifSt).toBe('field_6');
  expect(stored.contents[0].posX).toBe(5);
  expect(stored.contents[0].posY).toBe(5);
  expect(typeof stored.contents[0].width).toBe('number');
  expect(typeof stored.contents[0].height).toBe('number');
  expect(stored.page_size.bleeds).toEqual({ top: 3, right: 3, bottom: 3, left: 3 });
  expect(await storedYaml()).not.toMatch(/margin_/);

  await page.reload();
  await expect(page.locator('#template-name')).toBeVisible({ timeout: 45_000 });
  await page.getByRole('button', { name: 'Код' }).click();
  await expect.poll(async () => codeText(page)).toContain('"middle"');
  await expect.poll(async () => codeText(page)).toContain('auto_scale');
});
