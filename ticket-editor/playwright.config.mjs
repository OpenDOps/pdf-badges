import { defineConfig } from '@playwright/test';
import { frontendRoot, templatesDir, ticketEditorRoot } from './e2e/paths.mjs';

/** Prefer a free port when 4310 is already bound (e.g. Docker). Specs read the same env. */
const apiPort = process.env.E2E_API_PORT || process.env.PORT || '4310';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  timeout: 90_000,
  expect: { timeout: 20_000 },
  use: {
    baseURL: 'http://localhost:4201',
    channel: 'chrome',
  },
  webServer: [
    {
      command: 'node server.mjs',
      cwd: ticketEditorRoot,
      env: {
        ...process.env,
        PORT: apiPort,
        TEMPLATES_DIR: templatesDir,
        EDITOR_ORIGIN: 'http://localhost:4201',
      },
      url: `http://127.0.0.1:${apiPort}/api/print_template/1`,
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      command: 'npx ng serve --port 4201 --host localhost',
      cwd: frontendRoot,
      url: 'http://localhost:4201/editor/1',
      reuseExistingServer: false,
      timeout: 180_000,
    },
  ],
});
