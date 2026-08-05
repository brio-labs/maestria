import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const typeCheck = spawnSync(resolve(root, 'node_modules', '.bin', 'tsc'), ['--noEmit'], {
  cwd: root,
  stdio: 'inherit',
});
if (typeCheck.status !== 0) process.exit(typeCheck.status ?? 1);
