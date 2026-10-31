import * as esbuild from 'esbuild';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const target = process.argv[2];
const externalPreact = process.argv.includes('--external');

if (target !== 'react' && target !== 'preact') {
  console.error('Usage: node scripts/build.mjs react | preact [--external]');
  process.exit(1);
}
if (target === 'react' && externalPreact) {
  console.error('The React build always leaves React external.');
  process.exit(1);
}

const shared = {
  absWorkingDir: root,
  entryPoints: ['src/index.ts'],
  bundle: true,
  format: 'esm',
  platform: 'browser',
  jsx: 'transform',
  jsxFactory: 'h',
  jsxFragment: 'Fragment',
  target: 'es2022',
  legalComments: 'none',
};

if (target === 'react') {
  await esbuild.build({
    ...shared,
    outfile: 'dist/react.js',
    alias: { preact: path.join(root, 'compat/preact-react.ts') },
    external: ['react', 'react-dom', 'react-dom/client'],
  });
} else if (externalPreact) {
  await esbuild.build({
    ...shared,
    outfile: 'dist/preact.external.js',
    external: ['preact'],
  });
} else {
  await esbuild.build({
    ...shared,
    outfile: 'dist/preact.js',
  });
}
