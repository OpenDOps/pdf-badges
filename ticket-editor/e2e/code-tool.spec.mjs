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
async function dragSheet(page, x1, y1, x2, y2, shift = false) {
  const frame = await sheetFrame(page);
  await frame.evaluate(
    ({ start, end, shiftKey }) => {
      const fire = (type, point) =>
        document.dispatchEvent(
          new PointerEvent(type, {
            clientX: point.clientX,
            clientY: point.clientY,
            bubbles: true,
            cancelable: true,
            button: 0,
            pointerId: 1,
            shiftKey,
          }),
        );
      fire('pointerdown', start);
      fire('pointermove', end);
      fire('pointerup', end);
    },
    { start: client(x1, y1), end: client(x2, y2), shiftKey: shift },
  );
}

test('the QR and barcode tools place a code on drag and write it only on save', async ({ page, request }) => {
  test.setTimeout(150_000);
  const seeded = await request.put(api, { data: { name: 'Code tool', page: emptyPage() } });
  expect(seeded.status()).toBe(200);
  const before = await storedYaml();

  page.on('pageerror', (error) => console.log('PAGEERROR', error.message));
  await page.goto('/editor/1');
  const qrTool = page.getByRole('radio', { name: 'QR' });
  const barcodeTool = page.getByRole('radio', { name: 'Barcode' });
  await expect(qrTool).toBeVisible({ timeout: 45_000 });
  const selectTool = page.getByRole('radio', { name: 'Select', exact: true });
  await expect(selectTool).toHaveAttribute('aria-pressed', 'true');

  await dragSheet(page, 0, 0, 30, 12);
  const empty = await sheetFrame(page);
  await expect.poll(async () => empty.locator('[data-yaml-id]').count()).toBe(0);
  expect(await storedYaml()).toBe(before);

  await qrTool.click();
  await expect(qrTool).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('app-visual-editor')).toHaveAttribute('data-tool', 'qr');
  await dragSheet(page, 0, 0, 30, 12);
  const drawn = await sheetFrame(page);
  const qr = drawn.locator('[data-yaml-id="p1-b1"]');
  await expect(qr).toBeVisible();
  await expect(qr).toHaveAttribute('data-code', 'qrcode');
  await expect(qr).toHaveAttribute('src', /^data:image/);
  expect(await storedYaml()).toBe(before);
  await expect(qrTool).toHaveAttribute('aria-pressed', 'true');

  await page.keyboard.press('Meta+z');
  const undone = await sheetFrame(page);
  await expect.poll(async () => undone.locator('[data-yaml-id]').count()).toBe(0);
  await page.keyboard.press('Meta+Shift+z');
  const redone = await sheetFrame(page);
  await expect(redone.locator('[data-yaml-id="p1-b1"]')).toBeVisible();
  await expect(redone.locator('[data-yaml-id="p1-b1"]')).toHaveAttribute('data-code', 'qrcode');
  expect(await storedYaml()).toBe(before);

  await page.getByRole('radio', { name: 'Select', exact: true }).click();
  const toDelete = await sheetFrame(page);
  await toDelete.locator('[data-yaml-id="p1-b1"]').click();
  await page.keyboard.press('Backspace');
  const deleted = await sheetFrame(page);
  await expect.poll(async () => deleted.locator('[data-yaml-id]').count()).toBe(0);
  await qrTool.click();
  await dragSheet(page, 0, 0, 30, 12);
  const onlyNew = await sheetFrame(page);
  await expect.poll(async () => onlyNew.locator('[data-yaml-id]').count()).toBe(1);
  await expect(onlyNew.locator('[data-yaml-id="p1-b1"]')).toHaveAttribute('data-code', 'qrcode');
  await onlyNew.locator('[data-yaml-id="p1-b1"]').click();
  await page.keyboard.press('Meta+z');
  const undoneNew = await sheetFrame(page);
  await expect.poll(async () => undoneNew.locator('[data-yaml-id]').count()).toBe(0);
  await page.keyboard.press('Meta+z');
  const restored = await sheetFrame(page);
  await expect(restored.locator('[data-yaml-id="p1-b1"]')).toBeVisible();
  expect(await storedYaml()).toBe(before);

  await dragSheet(page, 50, 50, 51, 70);
  const still = await sheetFrame(page);
  await expect.poll(async () => still.locator('[data-yaml-id]').count()).toBe(1);
  expect(await storedYaml()).toBe(before);

  await dragSheet(page, 40, 0, 70, 12, true);
  const squared = await sheetFrame(page);
  const square = squared.locator('[data-yaml-id="p1-b2"]');
  await expect(square).toBeVisible();
  await expect(square).toHaveAttribute('data-code', 'qrcode');

  await barcodeTool.click();
  await expect(barcodeTool).toHaveAttribute('aria-pressed', 'true');
  await dragSheet(page, 0, 40, 30, 52);
  const barred = await sheetFrame(page);
  const barcode = barred.locator('[data-yaml-id="p1-b3"]');
  await expect(barcode).toBeVisible();
  await expect(barcode).toHaveAttribute('data-code', 'barcode');
  await expect(barcode).toHaveAttribute('src', /^data:image/);

  await page.keyboard.press('Meta+z');
  const barcodeUndone = await sheetFrame(page);
  await expect.poll(async () => barcodeUndone.locator('[data-yaml-id="p1-b3"]').count()).toBe(0);
  await expect.poll(async () => barcodeUndone.locator('[data-yaml-id="p1-b1"]').count()).toBe(1);
  await page.keyboard.press('Meta+Shift+z');
  const barcodeRedone = await sheetFrame(page);
  await expect(barcodeRedone.locator('[data-yaml-id="p1-b3"]')).toBeVisible();
  await expect(barcodeRedone.locator('[data-yaml-id="p1-b3"]')).toHaveAttribute('data-code', 'barcode');

  await dragSheet(page, 40, 40, 70, 52, true);
  const shifted = await sheetFrame(page);
  const plain = shifted.locator('[data-yaml-id="p1-b4"]');
  await expect(plain).toBeVisible();
  await expect(plain).toHaveAttribute('data-code', 'barcode');
  expect(await storedYaml()).toBe(before);
  await expect(barcodeTool).toHaveAttribute('aria-pressed', 'true');

  const saved = page.waitForResponse(
    (response) => response.url().includes('/api/print_template/1') && response.request().method() === 'PUT',
  );
  await page.getByRole('button', { name: 'Сохранить' }).click();
  expect((await saved).status()).toBe(200);

  const stored = parse(await storedYaml());
  expect(stored.contents).toHaveLength(4);
  expect(stored.contents[0]).toMatchObject({
    id: 'p1-b1',
    posX: 0,
    posY: 0,
    width: 30,
    height: 12,
    template: true,
    content: null,
    barcode: { type: 'qr', preentered: '{{code}}' },
  });
  expect(stored.contents[1]).toMatchObject({
    id: 'p1-b2',
    posX: 40,
    posY: 0,
    width: 30,
    height: 30,
    template: true,
    barcode: { type: 'qr', preentered: '{{code}}' },
  });
  expect(stored.contents[2]).toMatchObject({
    id: 'p1-b3',
    posX: 0,
    posY: 40,
    width: 30,
    height: 12,
    template: true,
    barcode: { type: 'ean13', preentered: '{{code}}' },
  });
  expect(stored.contents[3]).toMatchObject({
    id: 'p1-b4',
    posX: 40,
    posY: 40,
    width: 30,
    height: 12,
    template: true,
    barcode: { type: 'ean13', preentered: '{{code}}' },
  });
  for (const entry of stored.contents) {
    expect(entry.text).toBeUndefined();
    expect(entry.image).toBeUndefined();
    expect(entry.photo).toBeUndefined();
  }
});
