import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { rm } from 'node:fs/promises';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { captureWindow, visibleWindow } from './scenario.mjs';
const execute = promisify(execFile);

export async function fileScenario(session, { application, environment, evidence, fixtures }) {
  const input = await session.$('input[aria-label="Search apps and commands"]');
  const reopen = async () => {
    await execute(application, ['--activate'], { env: environment, timeout: 5000 });
    await session.waitUntil(async () => (await visibleWindow(environment)) !== '' && (await input.getValue()) === '', { timeout: 5000 });
  };
  const picker = async () => {
    await input.setValue('open file');
    await session.waitUntil(async () => (await session.$$('[role="option"]')).length === 1, { timeout: 5000 });
    await execute('xdotool', ['windowactivate', '--sync', await visibleWindow(environment), 'key', '--clearmodifiers', 'Return'], { env: environment, timeout: 5000 });
    let windowId;
    await session.waitUntil(async () => {
      try {
        windowId = (await execute('xdotool', ['search', '--onlyvisible', '--name', '^Open a local file$'], { env: environment })).stdout.trim().split('\n')[0];
        return Boolean(windowId);
      } catch (error) { if (error.code === 1) return false; throw error; }
    }, { timeout: 10000 });
    assert.notEqual(await visibleWindow(environment), '', 'Modal chooser must suspend blur dismissal');
    await execute('xdotool', ['windowactivate', '--sync', windowId], { env: environment });
    return windowId;
  };
  await reopen();
  await picker();
  await execute('xdotool', ['key', '--clearmodifiers', 'Escape'], { env: environment });
  await session.waitUntil(async () => !(await input.getAttribute('disabled')), { timeout: 5000 });
  await session.waitUntil(async () => (await session.$$('.result-row:disabled')).length === 0, { timeout: 5000 });
  assert.equal((await session.$$('[role="alert"]')).length, 0, 'Cancelling file selection is not an error');

  const chooserWindow = await picker();
  await execute('xdotool', ['key', '--clearmodifiers', 'ctrl+l'], { env: environment });
  await execute('xdotool', ['type', '--clearmodifiers', '--', fixtures.selectedFile], { env: environment });
  await execute('/usr/bin/python3', [fileURLToPath(new URL('./accept-file.py', import.meta.url))], { env: environment, timeout: 10000 });
  await session.waitUntil(async () => (await session.$('body')).getText().then(text => text.includes('Selected file')), { timeout: 10000 });
  await session.keys(['Control', 'k']);
  await session.keys('NULL');
  await (await session.$('button=Copy Path')).waitForExist({ timeout: 5000 });
  await (await session.$('button=Copy Path')).click();
  await session.waitUntil(async () => (await session.$('body')).getText().then(text => text.includes('Copied')), { timeout: 5000 });
  const { stdout } = await execute('/usr/bin/python3', [fileURLToPath(new URL('./clipboard.py', import.meta.url)), '--read'], { env: environment, timeout: 5000 });
  assert.equal(JSON.parse(stdout.trim()), fixtures.selectedFile);

  await rm(fixtures.selectedFile);
  await input.click();
  await session.keys('Enter');
  await (await session.$('[role="alert"]')).waitForDisplayed({ timeout: 5000 });
  assert.equal(await session.$('[role="option"]').getAttribute('title').then(title => title.includes(fixtures.selectedFile)), true);
  assert.notEqual(await visibleWindow(environment), '', 'Failed open must leave the launcher visible');
  await captureWindow(environment, path.join(evidence, 'native-file-unavailable.png'));
  await session.keys('ArrowDown');
  assert.equal(await session.$('[role="alert"]').isDisplayed(), true, 'Navigation must not clear an action failure');
  await session.keys('Escape');
  await session.waitUntil(async () => (await session.$('body')).getText().then(text => !text.includes('Selected file')), { timeout: 5000 });
  assert.notEqual(await visibleWindow(environment), '', 'First Escape leaves the selected-file view without hiding');
  await reopen();
}
