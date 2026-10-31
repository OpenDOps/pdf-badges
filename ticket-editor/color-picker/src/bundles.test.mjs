import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as bundled from '../dist/preact.js';
import * as external from '../dist/preact.external.js';
import * as react from '../dist/react.js';

const builds = [
  ['bundled preact', bundled],
  ['external preact', external],
  ['react', react],
];

async function source(name) {
  return readFile(new URL(`../dist/${name}`, import.meta.url), 'utf8');
}

test('react stays outside the React build', async () => {
  const file = await source('react.js');
  assert.match(file, /from\s*["']react["']/);
  assert.match(file, /from\s*["']react-dom\/client["']/);
  assert.doesNotMatch(file, /from\s*["']preact["']/);
  assert.doesNotMatch(file, /node_modules\/react/);
});

test('the default Preact build includes Preact', async () => {
  const file = await source('preact.js');
  assert.doesNotMatch(file, /from\s*["']preact["']/);
  assert.doesNotMatch(file, /from\s*["']react["']/);
  assert.match(file, /__k/);
});

test('Preact can be left external', async () => {
  const file = await source('preact.external.js');
  assert.match(file, /from\s*["']preact["']/);
  assert.doesNotMatch(file, /from\s*["']react["']/);
});

for (const [name, lib] of builds) {
  test(`${name} reads #FF6A00 from the box and the wheel`, () => {
    const size = 200;
    const hex = '#FF6A00';
    const box = lib.boxPoint(hex, size);
    const wheel = lib.wheelPoint(hex, size);
    const hue = lib.sliderPoint(hex, 'hue', size);
    assert.equal(lib.hexFromBox(hex, box.x, box.y, size), hex);
    assert.equal(lib.hexFromWheel(hex, wheel.x, wheel.y, size), hex);
    assert.equal(lib.hexFromSlider(hex, 'hue', hue.x, size), hex);
  });
}
