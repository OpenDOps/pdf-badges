import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { stringify } from 'yaml';
import { templatesDir } from './paths.mjs';

const api = 'http://127.0.0.1:4310/api/print_template';

function pageOf(width, height) {
  return {
    page_size: {
      width,
      height,
      units: 'mm',
      bleeds: { top: 0, right: 0, bottom: 0, left: 0 },
    },
    resources: { images: {}, fonts: {} },
    contents: [],
  };
}

test('the list shows both templates and opens the second', async ({ page, request }) => {
  const renamed = await request.put(`${api}/1`, { data: { name: 'Hall' } });
  expect(renamed.status()).toBe(200);

  const dir = path.join(templatesDir, '2');
  await mkdir(dir, { recursive: true });
  await writeFile(path.join(dir, 'template.yml'), stringify(pageOf(148, 210)));
  await writeFile(
    path.join(dir, 'meta.yml'),
    stringify({
      id: 2,
      name: 'Visitor',
      source: 'local',
      is_default: false,
      updated_at: '2026-02-03T04:05:06.000Z',
      margin_top_mm: 0,
      margin_right_mm: 0,
      margin_bottom_mm: 0,
      margin_left_mm: 0,
      manifest: {},
    }),
  );

  await page.goto('/editor/list');
  await expect(page.getByRole('link', { name: 'Hall', exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Visitor', exact: true })).toBeVisible();
  await expect(page.getByText('100×100 мм')).toBeVisible();
  await expect(page.getByText('148×210 мм')).toBeVisible();

  await page.getByRole('link', { name: 'Visitor', exact: true }).click();
  await expect(page).toHaveURL(/\/editor\/2$/);
  await expect(page.locator('#template-name')).toHaveValue('Visitor');

  await page.goto('/setup');
  await expect(page).toHaveURL(/\/editor\/list$/);
  await expect(page.getByRole('heading', { name: 'Макеты' })).toBeVisible();

  await page.goto('/editor');
  await expect(page).toHaveURL(/\/editor\/1$/);
  await expect(page.locator('#template-name')).toHaveValue('Hall');
});
