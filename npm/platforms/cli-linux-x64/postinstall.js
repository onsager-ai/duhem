#!/usr/bin/env node

/**
 * postinstall.js — ensure the shipped binary is executable on Unix.
 *
 * npm does not preserve the exec bit inside a published tarball, so the
 * binary arrives 0644. chmod it to 0755 on install. No-op / ignored on
 * Windows.
 */

const { existsSync, chmodSync } = require('node:fs');
const { join } = require('node:path');

const pkg = require('./package.json');
const binaryPath = join(__dirname, pkg.main);

if (existsSync(binaryPath)) {
  try {
    chmodSync(binaryPath, 0o755);
  } catch {
    // chmod is unsupported / unnecessary on Windows — ignore.
  }
}
