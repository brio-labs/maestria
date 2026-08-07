import http from 'node:http';
import { existsSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(process.argv[2] ?? 'dist');
const port = Number(process.env.PORT ?? 4173);
const mimeTypes = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.wasm': 'application/wasm',
};

function assetPath(urlPath) {
  const decoded = decodeURIComponent(urlPath.split('?')[0]);
  const relative = path.normalize(decoded.replace(/^\/+/, ''));
  if (relative.startsWith('..') || path.isAbsolute(relative)) {
    return null;
  }
  return path.join(root, relative);
}

const server = http.createServer((request, response) => {
  let requested;
  try {
    requested = assetPath(request.url ?? '/');
  } catch {
    response.writeHead(400);
    response.end('Bad request');
    return;
  }
  const fallback = path.join(root, 'index.html');
  const isAsset = requested?.includes(`${path.sep}assets${path.sep}`);
  const candidate = requested && existsSync(requested) && statSync(requested).isFile()
    ? requested
    : isAsset ? null : fallback;
  if (!candidate || !existsSync(candidate)) {
    response.writeHead(404);
    response.end('Not found');
    return;
  }
  const extension = path.extname(candidate);
  response.writeHead(200, { 'content-type': mimeTypes[extension] ?? 'application/octet-stream' });
  response.end(readFileSync(candidate));
});

server.listen(port, '127.0.0.1');

process.on('SIGTERM', () => server.close());
process.on('SIGINT', () => server.close());

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.stdout.write(`Static Studio server listening on ${port}\n`);
}
