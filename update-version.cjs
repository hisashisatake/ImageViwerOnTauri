const fs = require('fs');
const path = require('path');

const newVersion = process.argv[2];
if (!newVersion) {
  console.error('Usage: npm run version -- <new-version>');
  process.exit(1);
}

// package.json
const pkgPath = path.join(__dirname, 'package.json');
const pkgRaw = fs.readFileSync(pkgPath, 'utf8');
const pkgNewline = pkgRaw.includes('\r\n') ? '\r\n' : '\n';
const pkg = JSON.parse(pkgRaw);
pkg.version = newVersion;
const pkgString = JSON.stringify(pkg, null, 2).split('\n').join(pkgNewline);
fs.writeFileSync(pkgPath, pkgString, { encoding: 'utf8' });

// tauri.conf.json
const tauriPath = path.join(__dirname, 'src-tauri', 'tauri.conf.json');
const tauriRaw = fs.readFileSync(tauriPath, 'utf8');
const tauriNewline = tauriRaw.includes('\r\n') ? '\r\n' : '\n';
const tauri = JSON.parse(tauriRaw);
tauri.version = newVersion;
const tauriString = JSON.stringify(tauri, null, 2).split('\n').join(tauriNewline);
fs.writeFileSync(tauriPath, tauriString, { encoding: 'utf8' });

// Cargo.toml
const cargoPath = path.join(__dirname, 'src-tauri', 'Cargo.toml');
const cargo = fs.readFileSync(cargoPath, 'utf8');
const cargoNewline = cargo.includes('\r\n') ? '\r\n' : '\n';
const cargoUpdated = cargo.replace(
  /^version\s*=\s*"[^"]+"/m,
  `version = "${newVersion}"`
);
fs.writeFileSync(cargoPath, cargoUpdated.split(/\r?\n/).join(cargoNewline), { encoding: 'utf8' });

console.log(`Version updated to ${newVersion} in package.json, tauri.conf.json, and Cargo.toml`);
