#!/usr/bin/env node
// Downloads a prebuilt PDFium library from bblanchon/pdfium-binaries into
// src-tauri/resources/pdfium/. Run once before `npm run tauri dev|build`.
//
// Usage:
//   node scripts/fetch-pdfium.mjs              # current platform
//   node scripts/fetch-pdfium.mjs linux-x64    # a specific target (see ASSETS)
//   node scripts/fetch-pdfium.mjs --force      # re-download even if present
import { execFileSync } from 'node:child_process';
import {
  copyFileSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  readdirSync,
  rmSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pipeline } from 'node:stream/promises';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'src-tauri', 'resources', 'pdfium');

const ASSETS = {
  'win32-x64': 'pdfium-win-x64',
  'win32-arm64': 'pdfium-win-arm64',
  'darwin-x64': 'pdfium-mac-x64',
  'darwin-arm64': 'pdfium-mac-arm64',
  'darwin-universal': 'pdfium-mac-univ',
  'linux-x64': 'pdfium-linux-x64',
  'linux-arm64': 'pdfium-linux-arm64',
  'linux-x64-musl': 'pdfium-linux-musl-x64',
  'linux-arm64-musl': 'pdfium-linux-musl-arm64',
};

const LIBRARIES = { win32: 'pdfium.dll', darwin: 'libpdfium.dylib', linux: 'libpdfium.so' };

const force = process.argv.includes('--force');
const targetArg = process.argv.slice(2).find((a) => !a.startsWith('--'));
const target = targetArg ?? `${process.platform}-${process.arch}`;
const assetName = ASSETS[target];
if (!assetName) {
  console.error(`Unknown target "${target}". Known targets: ${Object.keys(ASSETS).join(', ')}`);
  process.exit(1);
}
const libraryName = LIBRARIES[target.split('-')[0]];

const headers = { 'User-Agent': 'bookshelf-pdf-fetch-script' };
if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;

const release = await fetch('https://api.github.com/repos/bblanchon/pdfium-binaries/releases/latest', {
  headers,
}).then((r) => {
  if (!r.ok) throw new Error(`GitHub API returned ${r.status}`);
  return r.json();
});

const asset = release.assets.find((a) => a.name === `${assetName}.tgz`);
if (!asset) throw new Error(`Asset ${assetName}.tgz not found in release ${release.tag_name}`);

mkdirSync(outDir, { recursive: true });
const libDest = join(outDir, libraryName);
if (existsSync(libDest) && !force) {
  console.log(`PDFium already present: ${libDest} (use --force to re-download)`);
  process.exit(0);
}

const archivePath = join(tmpdir(), `${assetName}-${release.tag_name.replace(/\//g, '_')}.tgz`);
if (!existsSync(archivePath) || force) {
  console.log(`Downloading ${asset.browser_download_url} ...`);
  const res = await fetch(asset.browser_download_url, { headers });
  if (!res.ok) throw new Error(`Download failed: HTTP ${res.status}`);
  await pipeline(res.body, createWriteStream(archivePath));
}

const extractDir = join(tmpdir(), `${assetName}-extract`);
rmSync(extractDir, { recursive: true, force: true });
mkdirSync(extractDir, { recursive: true });
try {
  // Pass only relative names to tar: GNU tar misreads `C:\...` archive paths
  // as remote `host:file` references on Windows.
  execFileSync(
    'tar',
    ['-xzf', basename(archivePath), '-C', basename(extractDir)],
    { cwd: dirname(archivePath), stdio: 'ignore' },
  );
} catch (err) {
  throw new Error(`Could not extract the archive with tar: ${err}`);
}

function findFile(dir, name) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) {
      const hit = findFile(p, name);
      if (hit) return hit;
    } else if (entry.name === name) {
      return p;
    }
  }
  return null;
}

const libFile = findFile(extractDir, libraryName);
if (!libFile) throw new Error(`${libraryName} not found inside the downloaded archive`);
copyFileSync(libFile, libDest);
const license = findFile(extractDir, 'LICENSE');
if (license) copyFileSync(license, join(outDir, 'LICENSE.pdfium.txt'));
rmSync(extractDir, { recursive: true, force: true });
console.log(`PDFium ${release.tag_name} installed at ${libDest}`);
