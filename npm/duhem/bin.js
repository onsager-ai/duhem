#!/usr/bin/env node

/**
 * bin.js — entry point for the `duhem` CLI installed from npm.
 *
 * The npm `duhem` package is a thin JS launcher. The actual Rust binary
 * ships in a per-platform optional dependency (`@duhem/cli-<os>-<arch>`);
 * npm installs only the one matching the host at install time. This
 * launcher resolves that package, locates the binary inside it, and execs
 * it with all CLI arguments forwarded and the exit code preserved.
 *
 * Architecture: rust-npm-publish skill (optionalDependencies platform
 * packages, same pattern as esbuild / swc / turbo).
 */

const { execFileSync } = require('node:child_process');
const { join } = require('node:path');

// Map Node's `${process.platform}-${process.arch}` to the platform package.
// NOTE: `process.platform` is `win32` (not `windows`); the package name uses
// `windows`. Keep this table in sync with npm/platforms/* and the
// optionalDependencies in npm/duhem/package.json.
const PLATFORM_PACKAGES = {
  'linux-x64': '@duhem/cli-linux-x64',
  'linux-arm64': '@duhem/cli-linux-arm64',
  'darwin-x64': '@duhem/cli-darwin-x64',
  'darwin-arm64': '@duhem/cli-darwin-arm64',
  'win32-x64': '@duhem/cli-windows-x64',
};

function resolveBinary() {
  const key = `${process.platform}-${process.arch}`;
  const pkg = PLATFORM_PACKAGES[key];

  if (!pkg) {
    const supported = Object.keys(PLATFORM_PACKAGES)
      .map((k) => `  - ${k}`)
      .join('\n');
    console.error(
      `duhem: unsupported platform "${key}".\n\n` +
        `Prebuilt binaries are published for:\n${supported}\n\n` +
        'Build from source instead: https://github.com/onsager-ai/duhem',
    );
    process.exit(1);
  }

  try {
    // require.resolve the platform package's manifest, then read `main`
    // to find the binary file shipped alongside it.
    const manifestPath = require.resolve(`${pkg}/package.json`);
    const manifest = require(`${pkg}/package.json`);
    return join(manifestPath, '..', manifest.main);
  } catch {
    console.error(
      `duhem: the platform package "${pkg}" for ${key} is not installed.\n\n` +
        'This usually means the optional dependency was skipped at install\n' +
        'time (e.g. --no-optional, --omit=optional, or an offline mirror).\n\n' +
        'Try reinstalling:\n  npm install -g duhem\n\n' +
        'or install the platform package directly:\n' +
        `  npm install ${pkg}`,
    );
    process.exit(1);
  }
}

const binary = resolveBinary();

try {
  execFileSync(binary, process.argv.slice(2), { stdio: 'inherit' });
} catch (error) {
  // execFileSync throws on a non-zero exit — forward the child's exit code.
  if (error && typeof error.status === 'number') {
    process.exit(error.status);
  }
  // No status (e.g. binary missing exec bit, ENOENT) — surface and fail.
  console.error(`duhem: failed to execute ${binary}: ${error && error.message}`);
  process.exit(1);
}
