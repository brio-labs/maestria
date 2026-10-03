import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

const linuxdeployRelease = '1-alpha-20251107-1';
const linuxdeployAssets = {
  x64: ['x86_64', 'c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d'],
  arm64: ['aarch64', '620095110d693282b8ebeb244a95b5e911cf8f65f76c88b4b47d16ae6346fcff'],
  arm: ['armhf', 'e359161979fa4bee50b92ce7102fb510299caebf34f711d983fba7a8f4bb1c2e'],
  ia32: ['i386', 'a2a88d142aac42db779483ca07c10dbf318b27f514691107fc88a202faae17b5'],
};
const gtkCommit = '596310ad881b83cabe8cb47b032232a250ebe5ec';
const gtkDigest = '7804c9eef13e59bf2783aad9882ef9db8f3f3f9e8d631874b1d348d550a3693f';

async function download(url, digest) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Packaging tool download failed (${response.status}): ${url}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (createHash('sha256').update(bytes).digest('hex') !== digest) {
    throw new Error(`Packaging tool checksum mismatch: ${url}`);
  }
  return bytes;
}

export async function prepareBundleTools(cacheDirectory) {
  const asset = linuxdeployAssets[process.arch];
  if (!asset) throw new Error(`No pinned linuxdeploy release for architecture ${process.arch}`);
  const [architecture, digest] = asset;
  const tools = path.join(cacheDirectory, 'tauri');
  await mkdir(tools, { recursive: true });
  const [linuxdeploy, gtk] = await Promise.all([
    download(`https://github.com/linuxdeploy/linuxdeploy/releases/download/${linuxdeployRelease}/linuxdeploy-${architecture}.AppImage`, digest),
    download(`https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/${gtkCommit}/linuxdeploy-plugin-gtk.sh`, gtkDigest),
  ]);
  const gtkSource = gtk.toString('utf8');
  const forcedBackend = /^export GDK_BACKEND=x11[^\n]*\n/m;
  if (!forcedBackend.test(gtkSource)) throw new Error('The pinned GTK packaging hook changed its backend policy');
  // Upstream forces XWayland, which prevents desktop-wide X11 grabs on Wayland.
  // Preserve GTK's native backend selection and any explicit user override.
  await Promise.all([
    writeFile(path.join(tools, `linuxdeploy-${architecture}.AppImage`), linuxdeploy, { mode: 0o700 }),
    writeFile(path.join(tools, 'linuxdeploy-plugin-gtk.sh'), gtkSource.replace(forcedBackend, ''), { mode: 0o700 }),
  ]);
}
