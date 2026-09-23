import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { prepareBundleTools } from './bundle-tools.mjs';

const launcherPath = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const nativePath = path.resolve(launcherPath, '..', 'crates', 'apps', 'maestria-launcher');
const cli = path.resolve(launcherPath, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
const args = process.argv.slice(2);

const bundlesLinux = process.platform === 'linux'
  && ['build', 'bundle'].includes(args[0])
  && !args.some((argument) => ['--no-bundle', '--help', '-h'].includes(argument));
let cacheDirectory;
try {
  if (bundlesLinux) {
    cacheDirectory = await mkdtemp(path.join(tmpdir(), 'maestria-bundle-'));
    await prepareBundleTools(cacheDirectory);
  }
  const child = spawn(process.execPath, [cli, ...args], {
    cwd: launcherPath,
    env: {
      ...process.env,
      ...(cacheDirectory ? { XDG_CACHE_HOME: cacheDirectory } : {}),
      TAURI_APP_PATH: nativePath,
      TAURI_FRONTEND_PATH: launcherPath,
    },
    stdio: 'inherit',
  });
  const interrupt = () => child.kill('SIGINT');
  const terminate = () => child.kill('SIGTERM');
  process.on('SIGINT', interrupt);
  process.on('SIGTERM', terminate);
  try {
    const [code, signal] = await once(child, 'exit');
    process.exitCode = code ?? (signal ? 1 : 0);
  } finally {
    process.off('SIGINT', interrupt);
    process.off('SIGTERM', terminate);
  }
} catch (error) {
  console.error(`Unable to run Tauri CLI: ${error.message}`);
  process.exitCode = 1;
} finally {
  if (cacheDirectory) await rm(cacheDirectory, { recursive: true, force: true });
}
