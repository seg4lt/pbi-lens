import fs from 'node:fs';

const readJson = (path) => JSON.parse(fs.readFileSync(path, 'utf8'));
const writeJson = (path, value) => fs.writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);

const pkg = readJson('package.json');
const parts = pkg.version.split('.').map(Number);
if (parts.length !== 3 || parts.some(Number.isNaN)) throw new Error(`Invalid app version: ${pkg.version}`);
const version = `${parts[0]}.${parts[1]}.${parts[2] + 1}`;

pkg.version = version;
writeJson('package.json', pkg);

const lock = readJson('package-lock.json');
lock.version = version;
lock.packages[''].version = version;
writeJson('package-lock.json', lock);

const tauri = readJson('src-tauri/tauri.conf.json');
tauri.version = version;
writeJson('src-tauri/tauri.conf.json', tauri);

const cargoPath = 'src-tauri/Cargo.toml';
const cargo = fs.readFileSync(cargoPath, 'utf8').replace(
  /^(\[package\][\s\S]*?^version\s*=\s*)"[^"]+"/m,
  `$1"${version}"`,
);
fs.writeFileSync(cargoPath, cargo);

const cargoLockPath = 'src-tauri/Cargo.lock';
const cargoLock = fs.readFileSync(cargoLockPath, 'utf8').replace(
  /(\[\[package\]\]\nname = "pbi-lens"\nversion = )"[^"]+"/,
  `$1"${version}"`,
);
fs.writeFileSync(cargoLockPath, cargoLock);

process.stdout.write(`${version}\n`);
