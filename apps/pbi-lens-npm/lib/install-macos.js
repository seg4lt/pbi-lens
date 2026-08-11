'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { download } = require('./download');
const { latestRelease } = require('./release');

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', ...options });
  if (result.status !== 0) throw new Error(`${command} failed: ${(result.stderr || result.stdout || '').trim()}`);
  return result.stdout || '';
}

async function installLatest({ launch = true, quiet = false } = {}) {
  if (process.platform !== 'darwin' || process.arch !== 'arm64') throw new Error('PBI Lens currently supports Apple Silicon Macs only');
  const release = await latestRelease();
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'pbi-lens-'));
  const dmg = path.join(temp, release.name);
  let mountPoint = '';
  try {
    if (!quiet) process.stderr.write(`Downloading PBI Lens ${release.tag}…\n`);
    await download(release.url, dmg);
    const plist = run('hdiutil', ['attach', dmg, '-nobrowse', '-readonly', '-plist']);
    mountPoint = plist.match(/<key>mount-point<\/key>\s*<string>([^<]+)<\/string>/)?.[1] || '';
    if (!mountPoint) throw new Error('The installer image mounted without a readable volume');
    const source = path.join(mountPoint, 'PBI Lens.app');
    if (!fs.existsSync(source)) throw new Error('PBI Lens.app was not found inside the installer image');

    let destinationDir = '/Applications';
    try { fs.accessSync(destinationDir, fs.constants.W_OK); }
    catch { destinationDir = path.join(os.homedir(), 'Applications'); fs.mkdirSync(destinationDir, { recursive: true }); }
    const destination = path.join(destinationDir, 'PBI Lens.app');
    if (fs.existsSync(destination)) fs.rmSync(destination, { recursive: true, force: true });
    run('ditto', [source, destination]);
    spawnSync('xattr', ['-dr', 'com.apple.quarantine', destination], { stdio: 'ignore' });
    if (!quiet) process.stderr.write(`Installed ${destination}\n`);
    if (launch) spawnSync('open', [destination], { stdio: quiet ? 'ignore' : 'inherit' });
  } finally {
    if (mountPoint) spawnSync('hdiutil', ['detach', mountPoint, '-quiet'], { stdio: 'ignore' });
    fs.rmSync(temp, { recursive: true, force: true });
  }
}

module.exports = { installLatest };
