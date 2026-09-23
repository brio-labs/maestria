import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { readFile, rm, writeFile } from 'node:fs/promises';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { invoke, visibleWindow } from './scenario.mjs';

const execute = promisify(execFile);
const clipboardProgram = fileURLToPath(new URL('./clipboard.py', import.meta.url));

export async function catalogScenario(session, { application, environment, evidence, fixtures }) {
  const input = await session.$('input[aria-label="Search apps and commands"]');
  const reopen = async () => {
    await execute(application, ['--activate'], { env: environment, timeout: 5000 });
    await session.waitUntil(async () => (await input.getValue()) === '' && (await visibleWindow(environment)) !== '', { timeout: 5000 });
  };
  const paste = async () => {
    const { stdout } = await execute('/usr/bin/python3', [clipboardProgram], { env: environment, timeout: 5000 });
    return JSON.parse(stdout.trim());
  };
  let generation = 20000;
  const query = async (text) => {
    generation += 1000;
    const response = await invoke(session, 'search', { query: text, generation });
    assert.equal(response.ok, true, JSON.stringify(response));
    return response.value;
  };

  const catalog = await query('CatalogProbe');
  assert.deepEqual(catalog.results.filter((result) => result.kind === 'application').map((result) => result.id).sort(), [
    'app:maestria-test-launch.desktop', 'app:maestria-test-override.desktop', 'app:maestria-test-removed.desktop',
  ]);
  assert.equal(catalog.results.some((result) => result.title.includes('System Shadowed')), false);
  assert.equal(catalog.results.find((result) => result.id === 'app:maestria-test-override.desktop').title, 'User Priority Fixture');

  // Every request crosses real IPC; the newest generation wins under contention.
  const racing = await session.executeAsync((start, done) => {
    Promise.all(Array.from({ length: 32 }, (_, offset) => window.__TAURI_INTERNALS__.invoke('search', {
      query: offset === 31 ? 'User Priority' : 'Fixture', generation: start + offset,
    }).then((value) => ({ ok: true, value }), (failure) => ({ ok: false, failure })))).then((values) => done(JSON.stringify(values)));
  }, generation + 1);
  const raced = JSON.parse(racing);
  generation += 32;
  assert.equal(raced[31].ok, true);
  assert.equal(raced[31].value.results[0].id, 'app:maestria-test-override.desktop');
  const stale = await invoke(session, 'execute_action', { resultId: catalog.results[0].id, actionId: 'open', generation: generation - 32 });
  assert.equal(stale.ok, false);
  assert.equal(stale.error.code, 'stale_result');

  await reopen();
  await input.setValue('Fixture');
  const retained = await session.$('[id="result-app:maestria-test-override.desktop"]');
  await retained.waitForExist({ timeout: 5000 });
  await retained.click();
  await writeFile(path.join(fixtures.applications, 'maestria-test-added.desktop'),
    `[Desktop Entry]\nType=Application\nName=Added Fixture\nExec="${fixtures.program}"\nTerminal=false\n`);
  await session.waitUntil(async () => (await session.$('[id="result-app:maestria-test-added.desktop"]')).isExisting(), { timeout: 10000 });
  assert.equal(await retained.getAttribute('aria-selected'), 'true', 'Catalog refresh must retain selection by desktop ID');
  await reopen();
  await input.setValue('2 + 2');
  await session.waitUntil(async () => (await session.$('.calculator-value')).getText().then((text) => text === '4'), { timeout: 5000 });
  await session.saveScreenshot(path.join(evidence, 'native-calculator.png'));
  await session.keys('Enter');
  await session.waitUntil(async () => (await session.$('body')).getText().then((text) => text.includes('Copied')), { timeout: 5000 });
  assert.notEqual(await visibleWindow(environment), '', 'Copy must leave the launcher visible');
  assert.equal(await paste(), '4', 'Calculation must paste through the real clipboard into another GTK entry');
  await reopen();
  await input.setValue('2 +');
  await session.waitUntil(async () => (await session.$$('.calculator-value')).length === 0, { timeout: 5000 });
  await (await session.$('[role="alert"]')).waitForDisplayed({ timeout: 5000 });
  await session.saveScreenshot(path.join(evidence, 'native-calculation-error.png'));

  await input.setValue('User Priority');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length === 1, { timeout: 5000 });
  await session.keys(['Control', 'k']);
  await session.keys('NULL');
  await session.waitUntil(async () => (await session.$('button=Copy App Name')).isExisting(), { timeout: 5000 });
  await session.saveScreenshot(path.join(evidence, 'native-action-panel.png'));
  await session.keys('ArrowDown');
  await session.keys('Enter');
  assert.equal(await paste(), 'User Priority Fixture');

  await reopen();
  await input.setValue('日本語');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length === 1, { timeout: 5000 });
  assert.equal(await session.$('[role="option"]').getAttribute('title').then((title) => title.includes(fixtures.launchName)), true);
  await session.saveScreenshot(path.join(evidence, 'native-unicode-application.png'));
  await session.keys(['Enter', 'Enter']);
  await session.waitUntil(async () => {
    try { return (await readFile(fixtures.record, 'utf8')).trim().length > 0; }
    catch (error) { if (error.code === 'ENOENT') return false; throw error; }
  }, { timeout: 5000 });
  const launches = (await readFile(fixtures.record, 'utf8')).trim().split('\n').map(JSON.parse);
  assert.deepEqual(launches, [['two words', '$HOME', 'literal;not-a-shell', fixtures.launchName, fixtures.launchPath, '%']]);

  await reopen();
  const removed = await query('Removable Fixture');
  assert.equal(removed.results[0].id, 'app:maestria-test-removed.desktop');
  await rm(fixtures.removedPath);
  const missing = await invoke(session, 'execute_action', { resultId: removed.results[0].id, actionId: 'open', generation });
  assert.equal(missing.ok, false, 'Removing the desktop file must prevent cached Exec launch');
  assert.match(missing.error.code, /app_unavailable|stale_result/);
  assert.equal((await readFile(fixtures.record, 'utf8')).trim().split('\n').length, 1);
  await reopen();
}
