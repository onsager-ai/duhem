/**
 * publish.mjs — publish the npm packages.
 *
 * Ordering matters: platform packages MUST be published (and propagated)
 * before the main package, which references them as optionalDependencies.
 * The release workflow runs `--target platforms` then `--target main`.
 *
 * Usage:
 *   node npm/scripts/publish.mjs --target platforms [--tag latest] [--dry-run]
 *   node npm/scripts/publish.mjs --target main      [--tag latest] [--dry-run]
 *
 * `--dry-run` runs `npm publish --dry-run` (packs + validates, never uploads).
 * Real publishes require NODE_AUTH_TOKEN in the environment (set by the
 * workflow from secrets.NPM_TOKEN); the workflow gates this on a real tag.
 */

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { config, mainDir, platformDir } from './lib.mjs';

const args = process.argv.slice(2);
const dryRun = args.includes('--dry-run');
const tIdx = args.indexOf('--target');
const target = tIdx >= 0 ? args[tIdx + 1] : 'platforms';
const tagIdx = args.indexOf('--tag');
const tag = tagIdx >= 0 ? args[tagIdx + 1] : 'latest';

/** The `version` field of the package.json in `dir`. */
function pkgVersion(dir) {
  return JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8')).version;
}

/**
 * True if `name@version` is already on the registry. `npm view` prints
 * the version for a published spec and exits non-zero (404) otherwise, so
 * a thrown error means "not published". This is the guard against the
 * failure that motivated #261: republishing a *different build* under a
 * version that already exists on npm, leaving two artifacts indistinguish-
 * able by `--version`. The fix is always to bump, never to reuse.
 */
function alreadyPublished(name, version) {
  try {
    const out = execFileSync('npm', ['view', `${name}@${version}`, 'version'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
    return out === version;
  } catch {
    return false;
  }
}

function publish(dir, name) {
  const version = pkgVersion(dir);
  if (!dryRun && alreadyPublished(name, version)) {
    console.error(
      `refusing to publish ${name}@${version}: that version is already on npm. ` +
        `A different build must never reuse a published version — bump the ` +
        `workspace version (Cargo.toml + the schema_version! macro), re-run ` +
        `npm/scripts/sync-versions.mjs, and tag the new version.`,
    );
    process.exit(1);
  }
  const argv = ['publish', '--access', 'public', '--tag', tag];
  if (dryRun) argv.push('--dry-run');
  console.log(`  ${dryRun ? '[dry-run] ' : ''}npm ${argv.join(' ')}  (${name})`);
  execFileSync('npm', argv, { cwd: dir, stdio: 'inherit' });
}

if (target === 'platforms') {
  for (const p of config.platforms) {
    publish(platformDir(p), p.package);
  }
  console.log(`\npublished ${config.platforms.length} platform package(s)${dryRun ? ' (dry-run)' : ''}`);
} else if (target === 'main') {
  publish(mainDir(), config.mainPackage.name);
  console.log(`\npublished ${config.mainPackage.name}${dryRun ? ' (dry-run)' : ''}`);
} else {
  console.error(`unknown --target "${target}" (expected: platforms | main)`);
  process.exit(1);
}
