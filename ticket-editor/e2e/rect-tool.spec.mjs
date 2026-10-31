import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { parse } from 'yaml';
import { templatesDir } from './paths.mjs';

const api = `http://127.0.0.1:${process.env.E2E_API_PORT || '4310'}/api/print_template/1`;
/** The canvas frame origin: 8 mm of desk, then the sheet. Same constants as the visual editor. */
const PX_PER_MM = 96 / 25.4;
const DESK_PAD_MM = 8;

function emptyPage() {
  return {
    page_size: {
      width: 100,
      height: 100,
      units: 'mm',
      bleeds: { top: 0, right: 0, bottom: 0, left: 0 },
    },
    resources: { images: {}, fonts: { text: { font_family: 'Liberation Serif' } } },
    contents: [],
  };
}

async function storedYaml() {
  return readFile(path.join(templatesDir, '1', 'template.yml'), 'utf8');
}

/** The sheet. GrapesJS also leaves a hidden history iframe in the page. */
async function sheetFrame(page) {
  const iframe = page.locator('iframe.gjs-frame').first();
  await expect(iframe).toBeVisible();
  const handle = await iframe.elementHandle();
  const frame = await handle.contentFrame();
  if (!frame) throw new Error('canvas frame missing');
  await expect.poll(async () => frame.locator('body').count()).toBeGreaterThan(0);
  return frame;
}

function client(xMm, yMm) {
  return {
    clientX: (DESK_PAD_MM + xMm) * PX_PER_MM,
    clientY: (DESK_PAD_MM + yMm) * PX_PER_MM,
  };
}

/** A corner drag in the sheet document, in sheet millimetres. */
async function dragSheet(page, x1, y1, x2, y2) {
  const frame = await sheetFrame(page);
  await frame.evaluate(
    ({ start, end }) => {
      const fire = (type, point) =>
        document.dispatchEvent(
          new PointerEvent(type, {
            clientX: point.clientX,
            clientY: point.clientY,
            bubbles: true,
            cancelable: true,
            button: 0,
            pointerId: 1,
          }),
        );
      fire('pointerdown', start);
      fire('pointermove', end);
      fire('pointerup', end);
    },
    { start: client(x1, y1), end: client(x2, y2) },
  );
}

test('the rectangle tool places an empty box on drag and writes it only on save', async ({ page, request }) => {
  test.setTimeout(150_000);
  const seeded = await request.put(api, { data: { name: 'Rectangle tool', page: emptyPage() } });
  expect(seeded.status()).toBe(200);
  const before = await storedYaml();

  page.on('pageerror', (error) => console.log('PAGEERROR', error.message));
  await page.addInitScript(() => localStorage.removeItem('ticket-editor:text-style'));
  await page.goto('/editor/1');
  const rectTool = page.getByRole('radio', { name: 'Rectangle' });
  await expect(rectTool).toBeVisible({ timeout: 45_000 });
  const selectTool = page.getByRole('radio', { name: 'Select', exact: true });
  await expect(selectTool).toHaveAttribute('aria-pressed', 'true');

  await dragSheet(page, 5, 5, 25, 15);
  const empty = await sheetFrame(page);
  await expect.poll(async () => empty.locator('[data-yaml-id]').count()).toBe(0);
  expect(await storedYaml()).toBe(before);

  await rectTool.click();
  await expect(rectTool).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('app-visual-editor')).toHaveAttribute('data-tool', 'rect');
  await dragSheet(page, 5, 5, 25, 15);
  const drawn = await sheetFrame(page);
  const box = drawn.locator('[data-yaml-id="p1-t1"]');
  await expect(box).toBeVisible();
  await expect(box).toHaveAttribute('data-preentered', '');
  expect(await storedYaml()).toBe(before);
  await expect(rectTool).toHaveAttribute('aria-pressed', 'true');

  await dragSheet(page, 40, 40, 41, 55);
  const still = await sheetFrame(page);
  await expect.poll(async () => still.locator('[data-yaml-id]').count()).toBe(1);
  expect(await storedYaml()).toBe(before);

  const saved = page.waitForResponse(
    (response) => response.url().includes('/api/print_template/1') && response.request().method() === 'PUT',
  );
  await page.getByRole('button', { name: 'Сохранить' }).click();
  expect((await saved).status()).toBe(200);

  const stored = parse(await storedYaml());
  expect(stored.contents).toHaveLength(1);
  const entry = stored.contents[0];
  expect(entry).toMatchObject({
    id: 'p1-t1',
    posX: 5,
    posY: 5,
    width: 20,
    height: 10,
    preentered: '',
    content: null,
    background: '#FFFFFF',
    border: { width: 0.2, color: '#000000' },
    alignment: { horizontal: 'left', vertical: 'top' },
    padding: { top: 0, right: 0, bottom: 0, left: 0 },
    text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
  });
  expect(entry.template).toBeUndefined();
  expect(String(entry.preentered)).not.toContain('{{');
  expect(entry.text).toBeTruthy();
  expect(entry.image).toBeUndefined();
  expect(entry.barcode).toBeUndefined();
  expect(entry.photo).toBeUndefined();
});
