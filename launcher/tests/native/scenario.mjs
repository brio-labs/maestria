import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const execute = promisify(execFile);

export async function visibleWindow(environment) {
  try {
    const { stdout } = await execute('xdotool', ['search', '--onlyvisible', '--name', '^Maestria Launcher$'], { env: environment });
    return stdout.trim();
  } catch (error) {
    if (error.code === 1) return '';
    throw error;
  }
}

export async function captureWindow(environment, destination) {
  const window = await visibleWindow(environment);
  assert.notEqual(window, '', 'The native surface must be mapped before capture');
  await execute('/usr/bin/python3', [fileURLToPath(new URL('./capture.py', import.meta.url)), window, destination], {
    env: environment, timeout: 5000,
  });
}

export async function invoke(session, command, args = {}) {
  const encoded = await session.executeAsync((name, parameters, done) => {
    window.__TAURI_INTERNALS__.invoke(name, parameters).then(
      (value) => done(JSON.stringify({ ok: true, value })),
      (error) => done(JSON.stringify({ ok: false, error })),
    );
  }, command, args);
  return JSON.parse(encoded);
}

export async function boundaryScenario(session, { application, environment, evidence }) {
  for (const flag of ['--help', '--version']) {
    await execute(application, [flag], { env: { ...environment, DISPLAY: '', WAYLAND_DISPLAY: '' }, timeout: 5000 });
  }
  const input = await session.$('input[aria-label="Search apps and commands"]');
  await input.waitForEnabled({ timeout: 15000 });
  const firstWindow = await visibleWindow(environment);
  assert.notEqual(firstWindow, '', 'Ready launcher must be mapped');
  await session.$('.shortcut-offer').waitForDisplayed({ timeout: 5000 });
  await captureWindow(environment, path.join(evidence, 'native-shortcut-first-offer.png'));
  await session.$('button=Not Now').click();
  await session.waitUntil(async () => !(await session.$('.shortcut-offer').isExisting()), { timeout: 5000 });
  const beforeDrag = (await execute('xdotool', ['getwindowgeometry', '--shell', firstWindow], { env: environment })).stdout;
  const beforeX = Number(beforeDrag.match(/^X=(\d+)$/m)?.[1]);
  const beforeY = Number(beforeDrag.match(/^Y=(\d+)$/m)?.[1]);
  await execute('xdotool', ['mousemove', '--window', firstWindow, '350', '12', 'mousedown', '1',
    'sleep', '0.1', 'mousemove_relative', '--sync', '40', '30', 'sleep', '0.1', 'mouseup', '1'], { env: environment });
  await session.waitUntil(async () => {
    const geometry = (await execute('xdotool', ['getwindowgeometry', '--shell', firstWindow], { env: environment })).stdout;
    return Number(geometry.match(/^X=(\d+)$/m)?.[1]) === beforeX + 40
      && Number(geometry.match(/^Y=(\d+)$/m)?.[1]) === beforeY + 30;
  }, { timeout: 5000, timeoutMsg: 'The native drag region must move the actual X11 window' });
  const moved = (await execute('xdotool', ['getwindowgeometry', '--shell', firstWindow], { env: environment })).stdout;
  await execute('xdotool', ['mousemove', '--window', firstWindow, '200', '55', 'mousedown', '1',
    'sleep', '0.1', 'mousemove_relative', '--sync', '40', '30', 'sleep', '0.1', 'mouseup', '1'], { env: environment });
  assert.equal((await execute('xdotool', ['getwindowgeometry', '--shell', firstWindow], { env: environment })).stdout, moved,
    'Dragging inside the entry must not move the native window');
  const { stdout: processId } = await execute('xdotool', ['getwindowpid', firstWindow], { env: environment });
  await input.setValue('resident-reset-proof');
  await execute(application, [], { env: environment, timeout: 5000 });
  await session.waitUntil(async () => (await input.getValue()) === '', { timeout: 5000 });
  await session.waitUntil(async () => (await visibleWindow(environment)) === firstWindow, { timeout: 5000 });
  assert.equal(await visibleWindow(environment), firstWindow, 'Normal second invocation must reuse the resident window');
  assert.equal(await session.$('.shortcut-offer').isExisting(), false, 'Deferral must survive activation');
  await input.setValue('preferences');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length > 0, { timeout: 5000 });
  await session.saveScreenshot(path.join(evidence, 'native-command-search.png'));
  const response = await invoke(session, 'search', { query: '', generation: 10000 });
  assert.equal(response.ok, true);
  const commands = response.value.results.filter((result) => result.kind === 'command');
  assert.deepEqual(commands.map((result) => result.id).sort(), ['host.open-file', 'host.preferences', 'host.quit', 'host.refresh-applications'].sort());

  const forged = await invoke(session, 'execute_action', { resultId: 'app:forged.desktop', actionId: 'open', generation: 10000 });
  assert.equal(forged.ok, false);
  const stale = await invoke(session, 'execute_action', { resultId: commands[0].id, actionId: commands[0].actions[0].id, generation: 9999 });
  assert.equal(stale.ok, false);
  assert.equal(stale.error.code, 'stale_result');
  const unknown = await invoke(session, 'execute_action', { resultId: commands[0].id, actionId: 'forged-action', generation: 10000 });
  assert.equal(unknown.ok, false);
  const excessive = await invoke(session, 'search', { query: 'é'.repeat(2049), generation: 10001 });
  assert.equal(excessive.ok, false);
  assert.equal(excessive.error.code, 'invalid_request');

  const denied = await invoke(session, 'plugin:clipboard-manager|write_text', { text: 'must-not-be-written' });
  assert.equal(denied.ok, false, 'Unlisted plugin command must be denied');
  assert.match(String(denied.error), /not allowed|denied|permission/i);
  const deniedWindow = await invoke(session, 'plugin:window|set_always_on_top', { value: true });
  assert.equal(deniedWindow.ok, false, 'Unlisted window effects must be denied');
  assert.match(String(deniedWindow.error), /not allowed|denied|permission/i);
  const controlOutsideView = await invoke(session, 'execute_action', { resultId: 'host.preferences', actionId: 'host.copy-activation', generation: 10000 });
  assert.equal(controlOutsideView.ok, false);

  const origin = await session.getUrl();
  await session.execute(() => { location.href = 'https://example.com/'; });
  assert.equal(await session.getUrl(), origin, 'External navigation must remain blocked');
  const handles = await session.getWindowHandles();
  await session.execute(() => { window.open('https://example.com/'); });
  assert.deepEqual(await session.getWindowHandles(), handles, 'New windows must remain blocked');

  await execute(application, ['--activate'], { env: environment, timeout: 5000 });
  await session.waitUntil(async () => (await input.getValue()) === '', { timeout: 5000 });
  await input.setValue('preferences');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length === 1, { timeout: 5000 });
  await session.keys('Enter');
  await session.waitUntil(async () => (await session.$('button=Back')).isExisting(), { timeout: 5000 });
  await session.keys('Escape');
  await input.waitForDisplayed();
  await input.setValue('unmatched-query-qzx');
  await session.waitUntil(async () => (await session.$('body')).getText().then((text) => text.includes('No matching apps or commands')), { timeout: 5000 });
  await session.saveScreenshot(path.join(evidence, 'native-no-match.png'));
  await session.keys('Escape');
  await session.waitUntil(async () => (await visibleWindow(environment)) === '', { timeout: 5000 });
  await execute(application, ['--activate'], { env: environment, timeout: 5000 });
  await session.waitUntil(async () => (await input.getValue()) === '', { timeout: 5000 });
  await session.waitUntil(async () => (await visibleWindow(environment)) === firstWindow, { timeout: 5000 });
  assert.equal(await visibleWindow(environment), firstWindow, 'Reopen must reuse the mapped native window');
  return { windowId: firstWindow, processId: Number(processId.trim()) };
}

export async function quitScenario(session, { application, environment }, resident) {
  assert.equal(await visibleWindow(environment), resident.windowId);
  await execute(application, ['--quit'], { env: environment, timeout: 5000 });
  await session.waitUntil(() => {
    try { process.kill(resident.processId, 0); return false; }
    catch (error) { if (error.code === 'ESRCH') return true; throw error; }
  }, { timeout: 5000 });
  await execute(application, ['--quit'], { env: environment, timeout: 5000 });
  assert.equal(await visibleWindow(environment), '', '--quit without a resident must not present a window');
}
