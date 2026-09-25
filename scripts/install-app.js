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

function bundleIdentity(path) {
  return execFileSync(
    '/usr/libexec/PlistBuddy',
    ['-c', 'Print :CFBundleIdentifier', join(path, 'Contents', 'Info.plist')],
    { encoding: 'utf8' },
  ).trim();
}

function verifyBundle(path) {
  if (bundleIdentity(path) !== 'com.iotensity.desktop') {
    throw new Error(`Refusing to replace an unrelated application: ${path}`);
  }
  execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', path], {
    stdio: 'pipe',
  });
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
  verify = verifyBundle,
  checkRunning = assertNotRunning,
  copy = (from, to) => execFileSync('/usr/bin/ditto', [from, to]),
  move = renameSync,
}) {
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
  verify(source);
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
    verify(incoming);
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
