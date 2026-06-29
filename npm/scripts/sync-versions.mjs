/**
 * sync-versions.mjs — keep every npm package version locked to the Cargo
 * workspace version (the single source of truth).
 *
 * Usage:
 *   node npm/scripts/sync-versions.mjs            # rewrite versions to match Cargo
 *   node npm/scripts/sync-versions.mjs --check    # assert in sync; exit 1 on drift
 *   node npm/scripts/sync-versions.mjs --version 0.2.0   # override (e.g. from a tag)
 *
 * Touches: npm/duhem/package.json (version + every optionalDependencies pin)
 * and each npm/platforms/<pkg>/package.json (version).
 */

import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { cargoVersion, config, mainDir, platformDir } from './lib.mjs';

const args = process.argv.slice(2);
const check = args.includes('--check');
const vIdx = args.indexOf('--version');
const version = vIdx >= 0 ? args[vIdx + 1] : cargoVersion();

const drift = [];

function reconcile(label, pkgPath, mutate) {
  const before = readFileSync(pkgPath, 'utf8');
  const pkg = JSON.parse(before);
  mutate(pkg);
  const after = JSON.stringify(pkg, null, 2) + '\n';
  if (after !== before) {
    if (check) {
      drift.push(label);
    } else {
      writeFileSync(pkgPath, after);
      console.log(`  updated ${label} -> ${version}`);
    }
  }
}

// Main package: version + every optionalDependencies pin.
reconcile(config.mainPackage.name, join(mainDir(), 'package.json'), (pkg) => {
  pkg.version = version;
  pkg.optionalDependencies ||= {};
  for (const p of config.platforms) {
    pkg.optionalDependencies[p.package] = version;
  }
});

// Platform packages: version only.
for (const p of config.platforms) {
  reconcile(p.package, join(platformDir(p), 'package.json'), (pkg) => {
    pkg.version = version;
  });
}

if (check) {
  if (drift.length > 0) {
    console.error(
      `version drift against ${version} (Cargo source of truth):\n` +
        drift.map((d) => `  - ${d}`).join('\n') +
        '\n\nRun: node npm/scripts/sync-versions.mjs',
    );
    process.exit(1);
  }
  console.log(`all npm package versions match ${version}`);
} else {
  console.log(`synced npm packages to ${version}`);
}
