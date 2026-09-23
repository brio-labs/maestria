import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { captureWindow, visibleWindow } from './scenario.mjs';
const execute = promisify(execFile);

export async function visualScenario(session, { application, environment, evidence }) {
  const input = await session.$('input[aria-label="Search apps and commands"]');
  const capture = (name) => captureWindow(environment, path.join(evidence, name));
  const resize = async (width, height) => {
    await execute('xdotool', ['windowsize', await visibleWindow(environment), String(width), String(height)], { env: environment, timeout: 5000 });
    await writeFile(path.join(evidence, 'native-resize.json'), JSON.stringify({
      requested: { width, height }, mapped: await visibleWindow(environment),
      viewport: await session.execute(() => ({ width: innerWidth, height: innerHeight, dpr: devicePixelRatio })),
      geometry: (await execute('xdotool', ['getwindowgeometry', '--shell', await visibleWindow(environment)], { env: environment })).stdout,
    }, null, 2));
    await session.waitUntil(async () => session.execute((w, h) => innerWidth === w && innerHeight === h, width, height), { timeout: 5000 });
  };
  await execute(application, ['--activate'], { env: environment, timeout: 5000 });
  await session.waitUntil(async () => (await visibleWindow(environment)) !== '' && (await input.getValue()) === '', { timeout: 5000 });
  await resize(720, 480);
  assert.notEqual(await visibleWindow(environment), '', 'Native resize must not dismiss the launcher');
  await input.setValue('Fixture');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length >= 3, { timeout: 5000 });
  assert.notEqual(await visibleWindow(environment), '', 'Native search must remain mapped before capture');
  await capture('native-normal-100.png');
  const selected = () => input.getAttribute('aria-activedescendant');
  const first = await selected();
  await session.keys('ArrowDown');
  const second = await selected();
  assert.notEqual(second, first);
  assert.equal(await session.execute(() => document.activeElement?.getAttribute('aria-label')), 'Search apps and commands');
  await session.keys('Home');
  assert.equal(await session.execute(() => document.querySelector('input').selectionStart), 0);
  assert.equal(await selected(), second);
  await session.keys('End');
  assert.equal(await session.execute(() => document.querySelector('input').selectionStart), 'Fixture'.length);

  // Exercise the renderer's composition guard inside the actual WebKit DOM.
  await session.execute(() => {
    const entry = document.querySelector('input');
    entry.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    for (const key of ['ArrowDown', 'Enter', 'Escape']) entry.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, isComposing: true }));
  });
  assert.equal(await selected(), second);
  assert.notEqual(await visibleWindow(environment), '');
  await session.execute(() => document.querySelector('input').dispatchEvent(new CompositionEvent('compositionend', { bubbles: true })));
  // Pointer selection is explicit, while a stationary pointer must not steal
  // the keyboard's subsequent selection.
  const fixtureRows = await session.$$('[role="option"]');
  assert.ok(fixtureRows.length >= 3, 'Fixture search must expose enough rows for pointer checks');
  await fixtureRows[2].click();
  const clickedRowId = await fixtureRows[2].getAttribute('id');
  const rowAboveId = await fixtureRows[1].getAttribute('id');
  assert.equal(await selected(), clickedRowId, 'Clicking a result row must select it');
  await fixtureRows[2].moveTo();
  await session.execute(() => document.querySelector('input')?.focus());
  await session.keys('ArrowUp');
  assert.equal(await selected(), rowAboveId, 'A stationary hover must not undo keyboard selection');
  assert.equal(await session.execute(() => document.activeElement?.getAttribute('aria-label')), 'Search apps and commands');

  // Double-click a safe built-in command, then exercise both footer controls
  // with the same Preferences navigation instead of launching an application.
  await input.setValue('preferences');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length === 1, { timeout: 5000 });
  await (await session.$('[id="result-host.preferences"]')).doubleClick();
  await (await session.$('.preferences-view')).waitForDisplayed({ timeout: 5000 });
  let preferenceFooter = await session.$$('.footer-button');
  assert.equal(preferenceFooter.length, 1);
  await preferenceFooter[0].click();
  await input.waitForEnabled({ timeout: 5000 });

  await input.setValue('preferences');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length === 1, { timeout: 5000 });
  preferenceFooter = await session.$$('.footer-button');
  assert.equal(preferenceFooter.length, 2);
  await preferenceFooter[0].click();
  await (await session.$('.preferences-view')).waitForDisplayed({ timeout: 5000 });
  await (await session.$('.footer-button')).click();
  await input.waitForEnabled({ timeout: 5000 });

  await input.setValue('preferences');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length === 1, { timeout: 5000 });
  const footerWithActions = await session.$$('.footer-button');
  await footerWithActions[1].click();
  await (await session.$('.action-panel')).waitForDisplayed({ timeout: 5000 });
  await (await session.$('.action-panel button')).click();
  await (await session.$('.preferences-view')).waitForDisplayed({ timeout: 5000 });
  await (await session.$('.footer-button')).click();
  await input.waitForEnabled({ timeout: 5000 });
  await input.setValue('Fixture');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length >= 3, { timeout: 5000 });


  await session.keys(['Control', 'k']);
  await session.keys('NULL');
  await (await session.$('.action-panel')).waitForDisplayed({ timeout: 5000 });
  assert.equal(await session.execute(() => document.activeElement === document.querySelector('.action-panel button')), true);
  await session.keys('ArrowDown');
  assert.equal(await session.execute(() => document.activeElement === document.querySelectorAll('.action-panel button')[1]), true);
  await session.keys('Escape');
  await session.waitUntil(async () => (await session.$$('.action-panel')).length === 0, { timeout: 5000 });
  assert.equal(await session.execute(() => document.activeElement?.getAttribute('aria-label')), 'Search apps and commands');

  await input.setValue('日本語');
  await session.waitUntil(async () => (await session.$$('[role="option"]')).length === 1, { timeout: 5000 });
  await resize(480, 320);
  await capture('native-narrow-unicode.png');
  const baseline = await session.execute(() => parseFloat(getComputedStyle(document.querySelector('input')).fontSize));
  await session.execute(() => { document.documentElement.style.fontSize = '200%'; });
  assert.equal(await session.execute(() => parseFloat(getComputedStyle(document.querySelector('input')).fontSize)), baseline * 2);
  await capture('native-narrow-text-200.png');
  await session.keys(['Control', 'k']);
  await session.keys('NULL');
  await (await session.$('.action-panel')).waitForDisplayed({ timeout: 5000 });
  await capture('native-narrow-actions-200.png');
  const panel = await session.execute(() => {
    const bounds = document.querySelector('.action-panel').getBoundingClientRect();
    return { left: bounds.left, right: bounds.right, bottom: bounds.bottom, width: innerWidth, height: innerHeight };
  });
  await writeFile(path.join(evidence, 'native-layout-measurements.json'), JSON.stringify({ textScale: 2, panel }, null, 2));
  assert.ok(panel.left >= 0 && panel.right <= panel.width && panel.bottom <= panel.height, 'Action panel must remain inside the narrow viewport at 200% text');
  await session.keys('Escape');
  await session.keys(['Control', ',']);
  await session.keys('NULL');
  await (await session.$('.preferences-view')).waitForDisplayed({ timeout: 5000 });
  await session.$('.preference-field input[type="text"]').waitForEnabled({ timeout: 5000 });
  await session.waitUntil(async () => session.execute(() => document.activeElement?.classList.contains('back-button')), { timeout: 5000 });
  await session.keys('Tab');
  assert.equal(await session.execute(() => document.activeElement?.matches('.preference-field input[type="text"]')), true,
    'Tab must focus the shortcut preference field');
  await session.keys('Tab');
  assert.equal(await session.execute(() => document.activeElement?.matches('.preference-switch input[type="checkbox"]')), true,
    'Tab must focus the reduce-motion preference field');
  await capture('native-preferences-fields-200.png');
  const preferenceViewport = await session.execute(() => {
    const surface = document.querySelector('.result-surface');
    return {
      scrollHeight: surface?.scrollHeight ?? 0,
      clientHeight: surface?.clientHeight ?? 0,
      scrollTop: surface?.scrollTop ?? 0,
    };
  });
  assert.ok(preferenceViewport.scrollHeight > preferenceViewport.clientHeight, '200% Preferences must scroll instead of clipping fields');
  for (let index = 0; index < 8; index += 1) {
    await session.keys('Tab');
    if (await session.execute(() => document.activeElement?.matches('.preference-actions .primary-button'))) break;
  }
  assert.equal(await session.execute(() => document.activeElement?.matches('.preference-actions .primary-button')), true,
    'Tab must reach the visible Save control after scrolling Preferences');
  assert.ok((await session.execute(() => document.querySelector('.result-surface')?.scrollTop ?? 0)) > 0,
    'Tabbing through 200% Preferences must scroll the result surface');
  await capture('native-preferences-200.png');
  await session.execute(() => { document.documentElement.style.fontSize = ''; });
  await resize(720, 480);
  await capture('native-preferences-100.png');
  await session.keys('Escape');
  await input.waitForEnabled({ timeout: 5000 });
  assert.notEqual(await visibleWindow(environment), '', 'Preferences Escape must not dismiss root');
  await input.setValue('no-results-qzx');
  await (await session.$('button=Clear query')).waitForDisplayed({ timeout: 5000 });
  await (await session.$('button=Clear query')).click();
  assert.equal(await input.getValue(), '');
  assert.equal(await session.execute(() => document.activeElement?.getAttribute('aria-label')), 'Search apps and commands');
}
