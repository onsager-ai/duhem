/**
 * prepare-platforms.mjs — copy CI-built binaries into the platform packages
 * and validate them before publish.
 *
 * Expects the release matrix to have downloaded artifacts laid out as:
 *   <artifacts>/binary-<nodeKey>/<duhem|duhem.exe>
 * (matching the `binary-<platform>` artifact names uploaded by the build job).
 *
 * Usage:
 *   node npm/scripts/prepare-platforms.mjs --artifacts artifacts
 *
 * Validates: file exists, non-empty, and the magic-byte header matches the
 * target OS (ELF / Mach-O / PE). chmod 0755 on Unix binaries.
 */

import { chmodSync, copyFileSync, existsSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { binaryName, config, platformDir, ROOT } from './lib.mjs';

const args = process.argv.slice(2);
const aIdx = args.indexOf('--artifacts');
const artifactsDir = resolve(ROOT, aIdx >= 0 ? args[aIdx + 1] : 'artifacts');

const HEADERS = {
  linux: [[0x7f, 0x45, 0x4c, 0x46]], // ELF
  darwin: [
    [0xcf, 0xfa, 0xed, 0xfe], // Mach-O 64 LE
    [0xfe, 0xed, 0xfa, 0xcf], // Mach-O 64 BE
  ],
  win32: [[0x4d, 0x5a]], // PE / MZ
};

function headerOk(filePath, os) {
  const expected = HEADERS[os];
  if (!expected) return true;
  const buf = readFileSync(filePath);
  return expected.some((sig) => sig.every((b, i) => buf[i] === b));
}

let errors = 0;

for (const p of config.platforms) {
  const file = binaryName(p);
  const src = join(artifactsDir, `binary-${p.platform}`, file);
  const dest = join(platformDir(p), file);

  if (!existsSync(src)) {
    console.error(`  MISSING artifact: ${src}`);
    errors++;
    continue;
  }
  copyFileSync(src, dest);
  if (p.os !== 'win32') {
    try {
      chmodSync(dest, 0o755);
    } catch {
      /* ignore */
    }
  }

  const size = statSync(dest).size;
  if (size === 0) {
    console.error(`  EMPTY: ${dest}`);
    errors++;
    continue;
  }
  if (!headerOk(dest, p.os)) {
    console.error(`  BAD HEADER: ${dest} (expected ${p.os} binary)`);
    errors++;
    continue;
  }
  console.log(`  ok ${p.package}: ${file} (${(size / 1024 / 1024).toFixed(1)} MB)`);
}

if (errors > 0) {
  console.error(`\n${errors} platform binary error(s)`);
  process.exit(1);
}
console.log('\nall platform binaries staged + validated');
