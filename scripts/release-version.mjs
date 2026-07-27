import fs from 'node:fs';

const [, , mode, rawVersion, compareVersion] = process.argv;
const version = rawVersion?.replace(/^v/, '');
const stableSemver = /^\d+\.\d+\.\d+$/;

function fail(message) {
  console.error(message);
  process.exit(1);
}

function requireVersion(value, label) {
  if (!value || !stableSemver.test(value)) {
    fail(`${label} must be a stable SemVer value such as 1.4.0`);
  }
}

function readJson(path) {
  return JSON.parse(fs.readFileSync(path, 'utf8'));
}

function writeJson(path, value) {
  fs.writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function cargoVersion() {
  const cargo = fs.readFileSync('src-tauri/Cargo.toml', 'utf8');
  const packageSection = cargo.match(/\[package\]([\s\S]*?)(?=\n\[|$)/)?.[1];
  return packageSection?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
}

function cargoLockVersion() {
  const lock = fs.readFileSync('src-tauri/Cargo.lock', 'utf8');
  return lock.match(/\[\[package\]\]\s*\nname = "ts-music"\s*\nversion = "([^"]+)"/)?.[1];
}

function projectVersions() {
  const packageJson = readJson('package.json');
  const lock = readJson('package-lock.json');
  const tauri = readJson('src-tauri/tauri.conf.json');
  return {
    'package.json': packageJson.version,
    'package-lock.json': lock.version,
    'package-lock root package': lock.packages?.['']?.version,
    'src-tauri/Cargo.toml': cargoVersion(),
    'src-tauri/Cargo.lock': cargoLockVersion(),
    'src-tauri/tauri.conf.json': tauri.version,
  };
}

function compareStableSemver(left, right) {
  const a = left.split('.').map(Number);
  const b = right.split('.').map(Number);
  for (let i = 0; i < 3; i += 1) {
    if (a[i] !== b[i]) return a[i] > b[i] ? 1 : -1;
  }
  return 0;
}

if (mode === '--check') {
  requireVersion(version, 'Release tag');
  const mismatches = Object.entries(projectVersions()).filter(([, value]) => value !== version);
  if (mismatches.length) {
    fail(
      `Release version ${version} does not match:\n${mismatches
        .map(([file, value]) => `- ${file}: ${value ?? '<missing>'}`)
        .join('\n')}`
    );
  }
  console.log(`All project manifests match ${version}`);
} else if (mode === '--set') {
  requireVersion(version, 'Release version');

  const packageJson = readJson('package.json');
  packageJson.version = version;
  writeJson('package.json', packageJson);

  const lock = readJson('package-lock.json');
  lock.version = version;
  if (lock.packages?.['']) lock.packages[''].version = version;
  writeJson('package-lock.json', lock);

  const tauri = readJson('src-tauri/tauri.conf.json');
  tauri.version = version;
  writeJson('src-tauri/tauri.conf.json', tauri);

  const cargoPath = 'src-tauri/Cargo.toml';
  const cargo = fs.readFileSync(cargoPath, 'utf8');
  const updatedCargo = cargo.replace(
    /(\[package\][\s\S]*?^version\s*=\s*")[^"]+(")/m,
    `$1${version}$2`
  );
  if (updatedCargo === cargo) fail('Could not update package version in src-tauri/Cargo.toml');
  fs.writeFileSync(cargoPath, updatedCargo);

  const cargoLockPath = 'src-tauri/Cargo.lock';
  const cargoLock = fs.readFileSync(cargoLockPath, 'utf8');
  const updatedCargoLock = cargoLock.replace(
    /(\[\[package\]\]\s*\nname = "ts-music"\s*\nversion = ")[^"]+(")/,
    `$1${version}$2`
  );
  if (updatedCargoLock === cargoLock) fail('Could not update ts-music in src-tauri/Cargo.lock');
  fs.writeFileSync(cargoLockPath, updatedCargoLock);

  console.log(`Project version set to ${version}`);
} else if (mode === '--assert-newer') {
  const current = compareVersion?.replace(/^v/, '');
  requireVersion(version, 'Rollback release version');
  requireVersion(current, 'Current release version');
  if (compareStableSemver(version, current) <= 0) {
    fail(`Rollback release ${version} must be newer than current release ${current}`);
  }
  console.log(`${version} is newer than ${current}`);
} else {
  fail(
    'Usage: release-version.mjs --check <tag> | --set <version> | --assert-newer <next> <current>'
  );
}
