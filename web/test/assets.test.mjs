import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const assets = ['index.html', 'app.js', 'app.css'];

test('committed Studio bundle contains the Vite entry assets', async () => {
  for (const asset of assets) {
    const contents = await readFile(resolve(root, 'dist', asset));
    assert.ok(contents.length > 0, `${asset} must not be empty`);
  }
  const html = await readFile(resolve(root, 'dist', 'index.html'), 'utf8');
  assert.match(html, /app\.js/);
  const javascript = await readFile(resolve(root, 'dist', 'app.js'), 'utf8');
  assert.match(javascript, /react/i);
});

test('React Studio controls have unique DOM ids', async () => {
  const source = await readFile(resolve(root, 'src', 'app.tsx'), 'utf8');
  const ids = [...source.matchAll(/\bid=["']([^"']+)["']/g)].map((match) => match[1]);
  assert.equal(new Set(ids).size, ids.length, 'React source contains duplicate ids');
});
