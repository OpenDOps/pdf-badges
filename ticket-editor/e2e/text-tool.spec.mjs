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

function canvasFrames(page) {
  return page.frames().filter((frame) => frame !== page.mainFrame());
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

function occurrences(text, needle) {
  return text.split(needle).length - 1;
}

async function canvasText(page) {
  const chunks = [];
  for (const frame of canvasFrames(page)) {
    chunks.push(await frame.locator('body').textContent().catch(() => ''));
  }
  return chunks.join('\n');
}

/** A click in the sheet document, in sheet millimetres. */
async function clickSheet(page, xMm, yMm) {
  const frame = await sheetFrame(page);
  const clientX = (DESK_PAD_MM + xMm) * PX_PER_MM;
  const clientY = (DESK_PAD_MM + yMm) * PX_PER_MM;
  await frame.evaluate(
    ({ clientX, clientY }) => {
      const fire = (type) =>
        document.dispatchEvent(
          new PointerEvent(type, {
            clientX,
            clientY,
            bubbles: true,
            cancelable: true,
            button: 0,
            pointerId: 1,
          }),
        );
      fire('pointerdown');
      fire('pointerup');
    },
    { clientX, clientY },
  );
}

test('the text tool places a box on click and writes it only on save', async ({ page, request }) => {
  test.setTimeout(150_000);
  const seeded = await request.put(api, { data: { name: 'Text tool', page: emptyPage() } });
  expect(seeded.status()).toBe(200);
  const before = await storedYaml();

  page.on('pageerror', (error) => console.log('PAGEERROR', error.message));
  await page.addInitScript(() => localStorage.removeItem('ticket-editor:text-style'));
  await page.goto('/editor/1');
  const textTool = page.getByRole('radio', { name: 'Text' });
  await expect(textTool).toBeVisible({ timeout: 45_000 });
  const selectTool = page.getByRole('radio', { name: 'Select', exact: true });
  await expect(selectTool).toHaveAttribute('aria-pressed', 'true');

  await clickSheet(page, 12.2, 7.4);
  await expect.poll(async () => canvasText(page)).not.toContain('Текст');
  expect(await storedYaml()).toBe(before);

  await textTool.click();
  await expect(textTool).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('app-visual-editor')).toHaveAttribute('data-tool', 'text');
  await clickSheet(page, 12.2, 7.4);
  await expect.poll(async () => canvasText(page)).toContain('Текст');
  expect(await storedYaml()).toBe(before);
  await expect(textTool).toHaveAttribute('aria-pressed', 'true');

  await page.keyboard.press('Meta+z');
  await expect.poll(async () => canvasText(page)).not.toContain('Текст');
  await page.keyboard.press('Meta+Shift+z');
  await expect.poll(async () => canvasText(page)).toContain('Текст');
  expect(await storedYaml()).toBe(before);

  await clickSheet(page, 30.1, 20.2);
  await expect.poll(async () => occurrences(await canvasText(page), 'Текст')).toBe(2);
  await expect(textTool).toHaveAttribute('aria-pressed', 'true');
  expect(await storedYaml()).toBe(before);

  await selectTool.click();
  await expect(page.locator('app-visual-editor')).toHaveAttribute('data-tool', 'select');
  const frame = await sheetFrame(page);
  const box = frame.locator('[data-yaml-id="p1-t1"]');
  await box.dblclick();
  await expect(box).toHaveAttribute('contenteditable', 'true');
  const longer = 'Hello, this line is much longer than the label';
  await page.keyboard.press('Meta+a');
  await page.keyboard.type(longer);
  await page.keyboard.press('Enter');
  await page.keyboard.type('Next');
  await frame.locator('body').click({ position: { x: 2, y: 2 } });
  await expect.poll(async () => canvasText(page)).toContain(longer);
  await expect.poll(async () => canvasText(page)).toContain('Next');
  expect(await storedYaml()).toBe(before);

  const saved = page.waitForResponse(
    (response) => response.url().endsWith('/api/print_template/1') && response.request().method() === 'PUT',
  );
  await page.getByRole('button', { name: 'Сохранить' }).click();
  expect((await saved).status()).toBe(200);

  const stored = parse(await storedYaml());
  expect(stored.contents).toHaveLength(2);
  expect(stored.contents[0]).toMatchObject({
    id: 'p1-t1',
    posX: 12,
    posY: 2,
    height: 10.5,
    preentered: `${longer}<br>Next`,
    content: null,
    alignment: { horizontal: 'left', vertical: 'top' },
    padding: { top: 0, right: 0, bottom: 0, left: 0 },
    text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
  });
  expect(stored.contents[1]).toMatchObject({
    id: 'p1-t2',
    posX: 30,
    posY: 14.5,
    height: 5.5,
    preentered: 'Текст',
    content: null,
  });
  expect(stored.contents[1].width).toBeGreaterThan(5);
  expect(stored.contents[1].width * 2).toBe(Math.round(stored.contents[1].width * 2));
  expect(stored.contents[0].width).toBeGreaterThan(stored.contents[1].width);
});
