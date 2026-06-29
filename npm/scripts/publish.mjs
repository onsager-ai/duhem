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
import { config, mainDir, platformDir } from './lib.mjs';

const args = process.argv.slice(2);
const dryRun = args.includes('--dry-run');
const tIdx = args.indexOf('--target');
const target = tIdx >= 0 ? args[tIdx + 1] : 'platforms';
const tagIdx = args.indexOf('--tag');
const tag = tagIdx >= 0 ? args[tagIdx + 1] : 'latest';

function publish(dir, name) {
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
