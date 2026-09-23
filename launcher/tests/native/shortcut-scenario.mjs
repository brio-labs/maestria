import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { captureWindow, invoke, visibleWindow } from './scenario.mjs';

const execute = promisify(execFile);
const here = path.dirname(fileURLToPath(import.meta.url));

async function competingGrab(environment, mask) {
  const child = spawn('/usr/bin/python3', [path.join(here, 'grab-shortcut.py'), String(mask)], {
    env: environment, stdio: ['pipe', 'pipe', 'inherit'],
  });
  try {
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('The competing X11 grab did not become ready')), 5000);
      child.once('error', error => { clearTimeout(timeout); reject(error); });
      child.once('exit', code => { clearTimeout(timeout); reject(new Error(`The competing grab exited: ${code}`)); });
      child.stdout.on('data', data => {
        if (data.toString().includes('SHORTCUT_GRABBED')) { clearTimeout(timeout); resolve(); }
      });
    });
    return child;
  } catch (error) {
    child.kill('SIGTERM');
    throw error;
  }
}

async function releaseGrab(child) {
  if (child.exitCode !== null) return;
  const stopped = once(child, 'exit');
  child.stdin.end('\n');
  await stopped;
}

export async function shortcutScenario(session, context, resident) {
  const { application, environment, evidence } = context;
  const settingsPath = path.join(environment.XDG_CONFIG_HOME, 'io.github.briolabs.Maestria.Launcher', 'launcher.toml');
  const openPreferences = async () => {
    await session.keys(['Control', ',']);
    await session.keys('NULL');
    await session.$('.preference-field input').waitForEnabled({ timeout: 5000 });
  };
  const hideRoot = async () => {
    if (await session.$('.preferences-view').isExisting()) await session.keys('Escape');
    await session.keys('Escape');
    await session.waitUntil(async () => (await visibleWindow(environment)) === '', { timeout: 5000 });
  };
  const activate = async shortcut => {
    // Do not clear modifiers: Caps/Num lock are part of this acceptance check.
    await execute('xdotool', ['key', shortcut], { env: environment });
    await session.waitUntil(async () => (await visibleWindow(environment)) !== '', { timeout: 5000 });
    await session.waitUntil(async () => session.execute(() => document.activeElement?.classList.contains('search-input')), { timeout: 5000 });
    const window = await visibleWindow(environment);
    const pid = Number((await execute('xdotool', ['getwindowpid', window], { env: environment })).stdout.trim());
    assert.equal(pid, resident.processId, 'A global shortcut must activate the existing resident');
  };
  const restart = async contents => {
    await execute(application, ['--quit'], { env: environment, timeout: 5000 });
    await session.waitUntil(() => {
      try { process.kill(resident.processId, 0); return false; }
      catch (error) { if (error.code === 'ESRCH') return true; throw error; }
    }, { timeout: 5000 });
    if (contents !== undefined) await writeFile(settingsPath, contents);
    await session.reloadSession();
    await session.$('input[aria-label="Search apps and commands"]').waitForEnabled({ timeout: 15000 });
    resident.windowId = await visibleWindow(environment);
    resident.processId = Number((await execute('xdotool', ['getwindowpid', resident.windowId], { env: environment })).stdout.trim());
  };

  await openPreferences();
  const motion = await session.$('.preference-switch input');
  if (!(await motion.isSelected())) await motion.click();
  await session.$('button=Save').click();
  await session.waitUntil(async () => (await readFile(settingsPath, 'utf8')).includes('reduceMotion = true'), { timeout: 5000 });
  await restart();
  assert.equal(await session.$('.shortcut-offer').isExisting(), false, 'Deferral must survive process restart');
  await session.$('.result-row').waitForExist({ timeout: 5000 });
  const transitionDurations = await session.execute(() =>
    getComputedStyle(document.querySelector('.result-row')).transitionDuration.split(',').map(Number.parseFloat));
  assert.ok(transitionDurations.every(seconds => seconds <= 0.001),
    'Persisted reduced motion must eliminate perceptible transitions before Preferences is opened');

  await openPreferences();
  await session.$('button=Set Up Shortcut').click();
  await session.waitUntil(async () => (await session.$('.preferences-view').getText()).includes('Shortcut configured'), { timeout: 5000 });
  await captureWindow(environment, path.join(evidence, 'native-shortcut-available.png'));
  const lockEvidence = [];
  for (const locks of [[], ['Caps_Lock'], ['Num_Lock'], ['Caps_Lock', 'Num_Lock']]) {
    await hideRoot();
    if (locks.length) await execute('xdotool', ['key', ...locks], { env: environment });
    try {
      const observed = JSON.parse((await execute('/usr/bin/python3', [path.join(here, 'keyboard-state.py')], { env: environment })).stdout);
      assert.equal(observed.caps, locks.includes('Caps_Lock'));
      assert.equal(observed.num, locks.includes('Num_Lock'));
      await activate('ctrl+space');
      lockEvidence.push({ locks, observed, activated: true });
    } finally {
      if (locks.length) await execute('xdotool', ['key', ...locks], { env: environment });
    }
  }
  await writeFile(path.join(evidence, 'native-lock-modifiers.json'), JSON.stringify(lockEvidence, null, 2));

  const competitor = await competingGrab(environment, 12);
  try {
    await openPreferences();
    await session.$('.preference-field input').setValue('Control+Alt+Space');
    await session.$('button=Save').click();
    await session.waitUntil(async () => (await session.$('.preference-error').getText()).includes('unavailable'), { timeout: 5000 });
    assert.match(await readFile(settingsPath, 'utf8'), /shortcut = "Control\+Space"/);
    await captureWindow(environment, path.join(evidence, 'native-shortcut-conflict.png'));
    await hideRoot();
    await activate('ctrl+space');
  } finally {
    await releaseGrab(competitor);
  }
  await openPreferences();
  await session.$('.preference-field input').setValue('Control+Alt+Space');
  await session.$('button=Save').click();
  await session.waitUntil(async () => (await readFile(settingsPath, 'utf8')).includes('shortcut = "Control+Alt+Space"'), { timeout: 5000 });
  const releasedOldGrab = await competingGrab(environment, 4);
  await releaseGrab(releasedOldGrab);
  await hideRoot();
  await activate('ctrl+alt+space');
  await restart();
  await session.waitUntil(async () => {
    const preferences = await invoke(session, 'get_preferences');
    return preferences.ok && preferences.value.shortcutStatus.state === 'available';
  }, { timeout: 5000 });
  assert.equal(await session.$('.shortcut-offer').isExisting(), false, 'Requested setup must restore without another first-run offer');
  await hideRoot();
  await activate('ctrl+alt+space');

  const future = 'schemaVersion = 2\nfutureSetting = "preserve this file"\n';
  await restart(future);
  await session.keys(['Control', ',']);
  await session.keys('NULL');
  await session.$('.preference-warning').waitForDisplayed({ timeout: 5000 });
  assert.equal(await session.$('button=Save').isEnabled(), false);
  assert.equal(await session.$('button=Reset preferences').isEnabled(), false);
  const forbidden = await invoke(session, 'save_preferences', { request: { reduceMotion: true } });
  assert.equal(forbidden.ok, false);
  assert.equal(forbidden.error.code, 'settings_failed');
  assert.equal(await readFile(settingsPath, 'utf8'), future, 'Unknown versions must remain byte-for-byte preserved');
  await captureWindow(environment, path.join(evidence, 'native-preferences-read-only.png'));

  const malformed = 'not = [valid TOML\n';
  await restart(malformed);
  await session.$('button=Not Now').click();
  await session.waitUntil(async () => !(await session.$('.shortcut-offer').isExisting()), { timeout: 5000 });
  assert.equal(await readFile(settingsPath, 'utf8'), malformed, 'Session changes must not overwrite malformed settings');
  await openPreferences();
  await session.$('button=Reset preferences').click();
  await session.$('button=Cancel').click();
  assert.equal(await readFile(settingsPath, 'utf8'), malformed, 'Cancelling Reset must preserve malformed settings');
  await session.$('button=Reset preferences').click();
  await session.$('button=Confirm reset').click();
  await session.waitUntil(async () => !(await session.$('.preference-warning').isExisting()), { timeout: 5000 });
  await restart();
  await session.$('.shortcut-offer').waitForDisplayed({ timeout: 5000 });
  await session.$('button=Not Now').click();
  await session.waitUntil(async () => !(await session.$('.shortcut-offer').isExisting()), { timeout: 5000 });
  const releasedNewGrab = await competingGrab(environment, 12);
  await releaseGrab(releasedNewGrab);
}
