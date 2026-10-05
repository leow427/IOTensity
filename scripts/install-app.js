import { execFileSync } from 'node:child_process';
import {
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  renameSync,
  rmSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

// Custom/test bundles and cross-compilation must never replace the everyday app.
export function shouldInstallApp(platform, args, env = {}) {
  return (
    platform === 'darwin' &&
    !env.CI &&
    args[0] === 'build' &&
    args.slice(1).every((arg) => ['--verbose', '-v'].includes(arg))
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
  try {
    mkdirSync(lock);
  } catch (error) {
    if (error.code === 'EEXIST')
      throw new Error(
        `Another IOTensity installation is in progress. Lock: ${lock}`,
      );
    throw error;
  }
  let stage;
  let preserveStage = false;
  try {
    stage = mkdtempSync(join(parent, '.iotensity-install-'));
    const incoming = join(stage, 'IOTensity.app');
    const previous = join(stage, 'previous.app');
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
            `Installation and rollback failed; previous app preserved at ${previous}`,
            { cause: rollbackError },
          );
        }
      }
      throw error;
    }
  } finally {
    if (stage && !preserveStage)
      rmSync(stage, { recursive: true, force: true });
    rmSync(lock, { recursive: true });
  }
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
