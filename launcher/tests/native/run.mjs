import { spawn, execFile } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, rm, symlink, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { createServer } from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { remote } from 'webdriverio';
import { promisify } from 'node:util';
import { boundaryScenario, quitScenario, visibleWindow, captureWindow } from './scenario.mjs';
import { createFixtures } from './fixtures.mjs';
import { catalogScenario } from './catalog-scenario.mjs';
import { visualScenario } from './visual-scenario.mjs';
import { fileScenario } from './file-scenario.mjs';
import { shortcutScenario } from './shortcut-scenario.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, '../../..');
const application = path.join(repository, 'target/release/maestria-launcher');
const execute = promisify(execFile);

if (!process.argv.includes('--isolated')) {
  const root = await mkdtemp(path.join(tmpdir(), 'maestria-launcher-native-'));
  try {
    for (const directory of ['data/applications', 'system/applications', 'config', 'cache', 'runtime']) {
      await mkdir(path.join(root, directory), { recursive: true, mode: 0o700 });
    }
    // Keep GTK's read-only resources without admitting installed desktop entries.
    for (const resource of ['glycin-loaders', 'icons', 'mime', 'themes']) {
      await symlink(path.join('/usr/share', resource), path.join(root, 'system', resource));
    }
    // Xvfb precedes D-Bus so every activated service receives the private display.
    const child = spawn('xvfb-run', ['-a', '-s', '-screen 0 1280x1024x24', 'dbus-run-session', '--', process.execPath, fileURLToPath(import.meta.url), '--isolated'], {
      stdio: 'inherit',
      env: {
        ...process.env, GDK_BACKEND: 'x11', WAYLAND_DISPLAY: '',
        MAESTRIA_NATIVE_TEST_ROOT: root,
        XDG_DATA_HOME: path.join(root, 'data'), XDG_DATA_DIRS: path.join(root, 'system'),
        XDG_CONFIG_HOME: path.join(root, 'config'), XDG_CACHE_HOME: path.join(root, 'cache'),
        XDG_RUNTIME_DIR: path.join(root, 'runtime'), XDG_CURRENT_DESKTOP: 'GNOME',
        GSETTINGS_SCHEMA_DIR: process.env.GSETTINGS_SCHEMA_DIR ?? '/usr/share/glib-2.0/schemas',
      },
    });
    const [code] = await once(child, 'exit');
    process.exitCode = code ?? 1;
  } finally {
    // Private portal services release their FUSE mounts when their bus exits.
    await rm(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
  }
} else {
  const root = process.env.MAESTRIA_NATIVE_TEST_ROOT;
  if (!root) throw new Error('The native suite must be launched through its private-session wrapper');
  for (const port of [4444, 4445]) {
    const probe = createServer();
    probe.listen(port, '127.0.0.1');
    await once(probe, 'listening');
    await new Promise((resolve, reject) => probe.close((error) => error ? reject(error) : resolve()));
  }
  const evidence = path.join(repository, 'target/launcher-evidence');
  await mkdir(evidence, { recursive: true });
  const fixtures = await createFixtures(root);
  const environment = { ...process.env, MAESTRIA_LAUNCHER_ARGV_RECORD: fixtures.record,
    AT_SPI_BUS_ADDRESS: process.env.DBUS_SESSION_BUS_ADDRESS, LANG: 'C.UTF-8' };
  const registryPath = ['/usr/libexec/at-spi2-registryd', '/usr/lib/at-spi2-registryd'].find(existsSync);
  if (!registryPath) throw new Error('Install at-spi2-core for native GTK accessibility automation');
  const registry = spawn(registryPath, [], { env: environment, stdio: 'inherit' });
  let registryError;
  registry.on('error', error => { registryError = error; });
  const wmConfig = path.join(root, 'jwm.xml');
  await writeFile(wmConfig, '<JWM><FocusModel>click</FocusModel></JWM>');
  const windowManager = spawn(process.env.X11_WINDOW_MANAGER ?? 'jwm', ['-f', wmConfig], { env: environment, stdio: 'inherit' });
  let managerError;
  windowManager.on('error', (error) => { managerError = error; });
  const args = ['--port', '4444'];
  if (process.env.WEBKIT_WEBDRIVER) args.push('--native-driver', process.env.WEBKIT_WEBDRIVER);
  const driver = spawn('tauri-driver', args, { env: environment, stdio: 'inherit', detached: true });
  let driverError;
  driver.on('error', (error) => { driverError = error; });
  let session;
  try {
    let registryReady = false;
    for (let attempt = 0; attempt < 100; attempt++) {
      if (registryError) throw registryError;
      if (registry.exitCode !== null) throw new Error('The private accessibility registry exited');
      const response = await execute('gdbus', ['call', '--session', '--dest', 'org.freedesktop.DBus', '--object-path', '/org/freedesktop/DBus', '--method', 'org.freedesktop.DBus.NameHasOwner', 'org.a11y.atspi.Registry'], { env: environment });
      if (response.stdout.includes('true')) { registryReady = true; break; }
      await delay(50);
    }
    if (!registryReady) throw new Error('The private accessibility registry did not become ready');
    let managerReady = false;
    for (let attempt = 0; attempt < 100; attempt++) {
      if (managerError) throw managerError;
      if (windowManager.exitCode !== null) throw new Error('The isolated X11 window manager exited');
      const { stdout } = await execute('xprop', ['-root', '_NET_SUPPORTING_WM_CHECK'], { env: environment });
      if (/window id # 0x[1-9a-f]/i.test(stdout)) { managerReady = true; break; }
      await delay(50);
    }
    if (!managerReady) throw new Error('The isolated X11 window manager did not become ready');
    let ready = false;
    for (let attempt = 0; attempt < 100; attempt++) {
      if (driverError) throw driverError;
      if (driver.exitCode !== null) throw new Error(`tauri-driver exited: ${driver.exitCode}`);
      try {
        if ((await fetch('http://127.0.0.1:4444/status')).ok) { ready = true; break; }
      } catch { /* Driver socket is not listening yet. */ }
      await delay(100);
    }
    if (!ready) throw new Error('tauri-driver did not become ready');
    session = await remote({
      hostname: '127.0.0.1', port: 4444, logLevel: 'warn',
      connectionRetryCount: 0, connectionRetryTimeout: 20000,
      capabilities: { 'tauri:options': { application } },
    });
    const context = { application, environment, evidence, root, fixtures };
    const resident = await boundaryScenario(session, context);
    console.log('Native boundary passed');
    await catalogScenario(session, context);
    console.log('Native catalog and clipboard passed');
    await visualScenario(session, context);
    console.log('Native visual and keyboard checks passed');
    await fileScenario(session, context);
    console.log('Native file chooser and error checks passed');
    await shortcutScenario(session, context, resident);
    console.log('Native shortcuts, lock modifiers and persistence passed');
    await quitScenario(session, context, resident);
    console.log('Native acceptance passed');
  } catch (error) {
    await writeFile(path.join(evidence, 'native-failure.txt'), String(error.stack ?? error));
    if (session) {
      const state = await session.execute(() => ({ text: document.body.innerText, visibility: document.visibilityState })).catch(() => null);
      await writeFile(path.join(evidence, 'native-failure-state.json'), JSON.stringify(state, null, 2));
      const active = await execute('xdotool', ['getactivewindow'], { env: environment }).catch(() => null);
      if (active) await execute('/usr/bin/python3', [path.join(here, 'capture.py'), active.stdout.trim(), path.join(evidence, 'native-active-failure.png')], { env: environment, timeout: 5000 }).catch(() => {});
    }
    if (session && await visibleWindow(environment)) await captureWindow(environment, path.join(evidence, 'native-failure.png')).catch((captureError) => console.error('Native surface capture failed:', captureError));
    throw error;
  } finally {
    if (session) await session.deleteSession().catch(() => {});
    if (driver.pid && driver.exitCode === null) {
      process.kill(-driver.pid, 'SIGTERM');
      await Promise.race([once(driver, 'exit'), delay(3000)]);
      if (driver.exitCode === null) process.kill(-driver.pid, 'SIGKILL');
    }
    if (windowManager.pid && windowManager.exitCode === null) {
      windowManager.kill('SIGTERM');
      await Promise.race([once(windowManager, 'exit'), delay(3000)]);
      if (windowManager.exitCode === null) windowManager.kill('SIGKILL');
    }
    if (registry.pid && registry.exitCode === null) {
      registry.kill('SIGTERM');
      await Promise.race([once(registry, 'exit'), delay(3000)]);
      if (registry.exitCode === null) registry.kill('SIGKILL');
    }
  }
}
