import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import {
  existsSync,
  linkSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { tauriSubcommand } from './signing.js';

// CI providers set CI=true; an explicit CI=false or CI=0 is a local build.
function isCi(env) {
  const value = String(env.CI ?? '')
    .trim()
    .toLowerCase();
  return value !== '' && value !== 'false' && value !== '0';
}

// Custom/test bundles and cross-compilation must never replace the everyday app.
export function shouldInstallApp(platform, args, env = {}) {
  return (
    platform === 'darwin' &&
    !isCi(env) &&
    tauriSubcommand(args) === 'build' &&
    args.filter((arg) => !/^(-v+|--verbose)$/.test(arg)).length === 1
  );
}

const FINGERPRINT = /^[A-F0-9]{40}$/;

// `security find-identity` prints the certificate's SHA-1 hash, which is what
// a `certificate leaf = H"..."` code requirement matches. Ad-hoc and other
// identities' signatures fail it.
export function signerRequirement(fingerprint) {
  if (!FINGERPRINT.test(fingerprint ?? ''))
    throw new Error(
      'The pinned macOS signing fingerprint is missing or invalid; refusing to install an unverified app.',
    );
  return `=certificate leaf = H"${fingerprint}"`;
}

// Without a fingerprint only integrity is checked: an installed app signed by a
// previously pinned identity may still be replaced by a correctly signed build.
export function verifyBundle(path, fingerprint, exec = execFileSync) {
  const identity = exec(
    '/usr/libexec/PlistBuddy',
    ['-c', 'Print :CFBundleIdentifier', join(path, 'Contents', 'Info.plist')],
    { encoding: 'utf8' },
  ).trim();
  if (identity !== 'com.iotensity.desktop') {
    throw new Error(`Refusing to replace an unrelated application: ${path}`);
  }
  exec('/usr/bin/codesign', ['--verify', '--deep', '--strict', path], {
    stdio: 'pipe',
  });
  if (fingerprint === undefined) return;
  const requirement = signerRequirement(fingerprint);
  try {
    exec(
      '/usr/bin/codesign',
      ['--verify', '--strict', '--test-requirement', requirement, path],
      { stdio: 'pipe' },
    );
  } catch (error) {
    throw new Error(
      `The app is not signed by the pinned macOS signing identity: ${path}`,
      { cause: error },
    );
  }
}

export function assertNotRunning(destination) {
  const executable = join(
    resolve(destination),
    'Contents',
    'MacOS',
    'iotensity',
  );
  const processes = execFileSync('/bin/ps', ['-axo', 'comm='], {
    encoding: 'utf8',
  });
  if (processes.split('\n').some((line) => line.trim() === executable)) {
    throw new Error(
      'Quit IOTensity normally to preserve unsaved changes, then build again. The installed app is still running and has not been replaced.',
    );
  }
}

function processAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code !== 'ESRCH';
  }
}

// Remove a lock only after confirming the recorded installer has exited. The
// lock is moved aside first, so a lock another installer just took over is put
// back instead of being deleted.
function removeStaleLock(lock) {
  let observed;
  try {
    observed = readFileSync(lock, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT') return true;
    return false; // Unreadable or an older directory lock: owner unknown.
  }
  let pid;
  try {
    ({ pid } = JSON.parse(observed));
  } catch {
    return false;
  }
  if (!Number.isSafeInteger(pid) || pid <= 0 || processAlive(pid)) return false;
  const stale = `${lock}.${randomUUID()}.stale`;
  try {
    renameSync(lock, stale);
  } catch (error) {
    if (error.code === 'ENOENT') return true;
    throw error;
  }
  const removed = readFileSync(stale, 'utf8') === observed;
  if (!removed) {
    try {
      linkSync(stale, lock);
    } catch {
      // A third installer already holds the lock.
    }
  }
  rmSync(stale, { force: true });
  return removed;
}

// The lock records its owner so an install killed mid-way cannot block later
// ones. It is published by hard link, so it never appears without contents.
function acquireLock(lock) {
  const pending = `${lock}.${randomUUID()}`;
  writeFileSync(
    pending,
    JSON.stringify({
      pid: process.pid,
      createdAt: new Date().toISOString(),
      token: randomUUID(),
    }),
    { flag: 'wx' },
  );
  try {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      try {
        linkSync(pending, lock);
        return;
      } catch (error) {
        if (error.code !== 'EEXIST') throw error;
      }
      if (!removeStaleLock(lock)) break;
    }
  } finally {
    rmSync(pending, { force: true });
  }
  throw new Error(
    `Another IOTensity installation is in progress. If none is running, remove the lock: ${lock}`,
  );
}

// Stage and verify on the destination filesystem before replacing anything.
// Inject the platform operations so rollback behavior is also tested on CI.
export function installBundle({
  source,
  destination,
  fingerprint,
  verify = verifyBundle,
  checkRunning = assertNotRunning,
  copy = (from, to) => execFileSync('/usr/bin/ditto', [from, to]),
  move = renameSync,
}) {
  signerRequirement(fingerprint);
  source = resolve(source);
  destination = resolve(destination);
  if (source === destination)
    throw new Error('Build and installed app paths must differ.');
  if (lstatSync(source).isSymbolicLink())
    throw new Error('The build must be a real app bundle.');
  const target = lstatSync(destination, { throwIfNoEntry: false });
  if (target && (!target.isDirectory() || target.isSymbolicLink())) {
    throw new Error(
      'The installed app path must be a real app bundle, not a link or file.',
    );
  }
  checkRunning(destination);
  verify(source, fingerprint);
  if (existsSync(destination)) verify(destination);
  const parent = dirname(destination);
  mkdirSync(parent, { recursive: true });
  const lock = join(parent, '.iotensity-install.lock');
  acquireLock(lock);
  let stage;
  let preserveStage = false;
  let failure;
  let cleanupFailure;
  try {
    // The hidden stage keeps the copy off the destination's visible listing.
    // The incoming copy keeps its .app name so codesign verifies exactly the
    // bundle that is renamed into place; the displaced app is not verified
    // again, so it drops the extension and stops looking like an app.
    stage = mkdtempSync(join(parent, '.iotensity-install-'));
    const incoming = join(stage, 'IOTensity.app');
    const previous = join(stage, 'previous');
    copy(source, incoming);
    verify(incoming, fingerprint);
    checkRunning(destination);
    const replaced = existsSync(destination);
    if (replaced) move(destination, previous);
    try {
      move(incoming, destination);
    } catch (error) {
      if (replaced) {
        try {
          move(previous, destination);
        } catch (rollbackError) {
          preserveStage = true;
          throw new Error(
            `Installation and rollback failed; previous app preserved at ${previous} (move it back to ${destination})`,
            { cause: rollbackError },
          );
        }
      }
      throw error;
    }
  } catch (error) {
    failure = error;
    throw error;
  } finally {
    // Cleanup failures must not hide why the installation itself failed. A
    // leftover lock names this process, so the next install recovers it.
    const cleanup = [];
    try {
      if (stage && !preserveStage)
        rmSync(stage, { recursive: true, force: true });
    } catch (error) {
      cleanup.push(error);
    }
    try {
      rmSync(lock);
    } catch (error) {
      cleanup.push(error);
    }
    if (cleanup.length > 0) {
      const message = `Could not remove installation files in ${parent}`;
      if (failure) console.warn(`${message}:`, ...cleanup);
      else cleanupFailure = new Error(message, { cause: cleanup[0] });
    }
  }
  if (cleanupFailure) throw cleanupFailure;
  return destination;
}

export function appInstallation(root, env) {
  const metadata = JSON.parse(
    execFileSync(
      'cargo',
      [
        'metadata',
        '--manifest-path',
        join(root, 'src-tauri', 'Cargo.toml'),
        '--format-version',
        '1',
        '--no-deps',
        '--locked',
      ],
      { encoding: 'utf8', env },
    ),
  );
  return {
    source: join(
      metadata.target_directory,
      'release',
      'bundle',
      'macos',
      'IOTensity.app',
    ),
    destination: join(homedir(), 'Applications', 'IOTensity.app'),
  };
}
