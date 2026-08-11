#!/usr/bin/env node
'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const pkg = require('../package.json');
const { installLatest } = require('../lib/install-macos');

const args = process.argv.slice(2);
const command = args.find((arg) => !arg.startsWith('-')) || 'install';
const quiet = args.includes('--quiet');
const noLaunch = args.includes('--no-launch');

function usage() {
  process.stdout.write(`PBI Lens installer\n\nUsage:\n  npx @seg4lt/pbi-lens [install|update|launch|uninstall] [--no-launch] [--quiet]\n`);
}

async function main() {
  if (args.includes('--help') || args.includes('-h')) return usage();
  if (args.includes('--version') || args.includes('-v')) return process.stdout.write(`${pkg.version}\n`);
  if (command === 'install' || command === 'update') return installLatest({ launch: !noLaunch, quiet });

  const candidates = ['/Applications/PBI Lens.app', path.join(os.homedir(), 'Applications/PBI Lens.app')];
  const app = candidates.find(fs.existsSync);
  if (command === 'launch') {
    if (!app) throw new Error('PBI Lens is not installed. Run npx @seg4lt/pbi-lens first.');
    spawnSync('open', [app], { stdio: 'inherit' });
    return;
  }
  if (command === 'uninstall') {
    if (!app) return process.stderr.write('PBI Lens is not installed.\n');
    fs.rmSync(app, { recursive: true, force: true });
    process.stderr.write(`Removed ${app}\n`);
    return;
  }
  throw new Error(`Unknown command: ${command}`);
}

main().catch((error) => {
  process.stderr.write(`pbi-lens: ${error.message || error}\n`);
  process.exitCode = 1;
});
