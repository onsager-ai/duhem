/**
 * lib.mjs — shared helpers for the npm distribution pipeline.
 *
 * Pure Node (ESM), no third-party deps, so the release workflow only needs
 * `actions/setup-node` — no `npm install`, no pnpm, no TS toolchain. This is
 * the rust-npm-publish skill's pipeline, trimmed to plain JSON config + .mjs.
 */

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));

/** Repo root (npm/scripts → npm → root). */
export const ROOT = resolve(HERE, '..', '..');

/** Parsed npm/publish.config.json. */
export const config = JSON.parse(
  readFileSync(join(ROOT, 'npm', 'publish.config.json'), 'utf8'),
);

/**
 * Read the workspace version from Cargo.toml's `[workspace.package]` table.
 * This is the single source of truth all npm versions must track.
 */
export function cargoVersion() {
  const toml = readFileSync(join(ROOT, config.cargoWorkspace), 'utf8');
  const section = toml.split(/^\[/m).find((s) => s.startsWith('workspace.package'));
  const match = section && section.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error('could not find [workspace.package] version in Cargo.toml');
  }
  return match[1];
}

/** Absolute path to a platform package directory. */
export function platformDir(platform) {
  return join(ROOT, config.platformDir, platform.dir);
}

/** Absolute path to the main package directory. */
export function mainDir() {
  return join(ROOT, config.mainPackage.dir);
}

/** Binary filename for a platform (handles the Windows .exe extension). */
export function binaryName(platform) {
  return `${config.binaryName}${platform.ext}`;
}
