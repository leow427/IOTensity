import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  installBundle,
  shouldInstallApp,
  verifyBundle,
} from './install-app.js';

const FINGERPRINT = '0123456789ABCDEF0123456789ABCDEF01234567';

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'iotensity-install-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const source = join(root, 'build', 'IOTensity.app');
  const destination = join(root, 'Applications', 'IOTensity.app');
  for (const [path, version] of [
    [source, 'new'],
    [destination, 'old'],
  ]) {
    mkdirSync(path, { recursive: true });
    writeFileSync(join(path, 'version'), version);
  }
  return {
    source,
    destination,
    fingerprint: FINGERPRINT,
    verify: () => {},
    checkRunning: () => {},
    copy: (from, to) => cpSync(from, to, { recursive: true }),
  };
}
const version = (path) => readFileSync(join(path, 'version'), 'utf8');

test('installs only ordinary local release builds', () => {
  assert.equal(shouldInstallApp('darwin', ['build']), true);
  assert.equal(shouldInstallApp('darwin', ['build', '-v']), true);
  assert.equal(shouldInstallApp('darwin', ['-v', 'build']), true);
  assert.equal(shouldInstallApp('darwin', ['-vv', 'build', '-v']), true);
  assert.equal(shouldInstallApp('darwin', ['--verbose', 'build']), true);
  for (const args of [
    ['dev'],
    ['bundle'],
    ['build', '--debug'],
    ['build', '--no-bundle'],
    ['build', '--config', 'test.json'],
    ['build', '--target', 'aarch64-apple-darwin'],
    ['build', '--help'],
    ['--help'],
    ['-v', 'dev'],
    ['-v', 'build', '--no-bundle'],
    ['-v', 'build', '--debug'],
    ['build', '--', '--foo'],
    ['build', 'build'],
    ['--', 'build'],
  ]) {
    assert.equal(shouldInstallApp('darwin', args), false);
  }
  assert.equal(shouldInstallApp('darwin', ['build'], { CI: 'true' }), false);
  assert.equal(shouldInstallApp('win32', ['build']), false);
});

test('updates the stable app and cleans staging without changing the build', (t) => {
  const options = fixture(t);
  assert.equal(installBundle(options), options.destination);
  assert.equal(version(options.destination), 'new');
  assert.equal(version(options.source), 'new');
  assert.deepEqual(readdirSync(join(options.destination, '..')), [
    'IOTensity.app',
  ]);
});

test('refuses running apps and failed signatures before replacing the installation', (t) => {
  const options = fixture(t);
  assert.throws(
    () =>
      installBundle({
        ...options,
        checkRunning: () => {
          throw new Error('running');
        },
      }),
    /running/,
  );
  assert.throws(
    () =>
      installBundle({
        ...options,
        verify: (path) => {
          if (path.includes('.iotensity-install-'))
            throw new Error('invalid staged signature');
        },
      }),
    /invalid staged signature/,
  );
  assert.equal(version(options.destination), 'old');
});

test('restores the previous app when replacement fails', (t) => {
  const options = fixture(t);
  assert.throws(
    () =>
      installBundle({
        ...options,
        move: (from, to) => {
          if (
            from.includes('.iotensity-install-') &&
            from.endsWith('IOTensity.app')
          )
            throw new Error('replacement failed');
          renameSync(from, to);
        },
      }),
    /replacement failed/,
  );
  assert.equal(version(options.destination), 'old');
});

test('preserves recoverable files if rollback also fails', (t) => {
  const options = fixture(t);
  assert.throws(
    () =>
      installBundle({
        ...options,
        move: (from, to) => {
          if (from.includes('.iotensity-install-'))
            throw new Error('filesystem unavailable');
          renameSync(from, to);
        },
      }),
    /previous app preserved/,
  );
  const parent = join(options.destination, '..');
  const stage = readdirSync(parent).find((name) =>
    name.startsWith('.iotensity-install-'),
  );
  assert.equal(version(join(parent, stage, 'previous')), 'old');
});

const lockPath = (options) =>
  join(options.destination, '..', '.iotensity-install.lock');
const writeLock = (options, pid) =>
  writeFileSync(
    lockPath(options),
    JSON.stringify({ pid, createdAt: new Date().toISOString(), token: 'x' }),
  );

test('does not overwrite another install or follow a destination symlink', (t) => {
  const options = fixture(t);
  const lock = lockPath(options);
  writeLock(options, process.pid);
  assert.throws(() => installBundle(options), /in progress/);
  assert.equal(existsSync(lock), true);
  rmSync(lock);
  // A lock without a recorded owner cannot be proven stale.
  mkdirSync(lock);
  assert.throws(() => installBundle(options), /in progress/);
  assert.equal(version(options.destination), 'old');
  rmSync(lock, { recursive: true });
  rmSync(options.destination, { recursive: true });
  symlinkSync(options.source, options.destination, 'dir');
  assert.throws(() => installBundle(options), /not a link/);
  assert.equal(version(options.source), 'new');
});

test('requires the pinned signer for the build and staged copy', (t) => {
  const options = fixture(t);
  const calls = [];
  installBundle({
    ...options,
    verify: (path, fingerprint) => calls.push([path, fingerprint]),
  });
  assert.equal(calls.length, 3);
  assert.deepEqual(calls[0], [options.source, FINGERPRINT]);
  // A previously installed app may predate an explicit identity replacement.
  assert.deepEqual(calls[1], [options.destination, undefined]);
  assert.match(calls[2][0], /\.iotensity-install-.*IOTensity\.app$/);
  assert.equal(calls[2][1], FINGERPRINT);
  for (const fingerprint of [undefined, '', 'not-a-fingerprint']) {
    assert.throws(
      () => installBundle({ ...options, fingerprint }),
      /fingerprint is missing or invalid/,
    );
  }
  assert.equal(version(options.destination), 'new');
});

test('checks the code signature against the pinned leaf certificate', () => {
  const calls = [];
  const exec = (file, args) => {
    calls.push([file, args]);
    return file === '/usr/libexec/PlistBuddy' ? 'com.iotensity.desktop\n' : '';
  };
  verifyBundle('/build/IOTensity.app', FINGERPRINT, exec);
  assert.deepEqual(calls.slice(1), [
    [
      '/usr/bin/codesign',
      ['--verify', '--deep', '--strict', '/build/IOTensity.app'],
    ],
    [
      '/usr/bin/codesign',
      [
        '--verify',
        '--strict',
        '--test-requirement',
        `=certificate leaf = H"${FINGERPRINT}"`,
        '/build/IOTensity.app',
      ],
    ],
  ]);
  calls.length = 0;
  verifyBundle('/installed/IOTensity.app', undefined, exec);
  assert.equal(calls.length, 2);
  assert.throws(
    () => verifyBundle('/build/IOTensity.app', 'abc', exec),
    /fingerprint is missing or invalid/,
  );
});

test('refuses ad-hoc or differently signed staged apps without replacing', (t) => {
  const options = fixture(t);
  // codesign exits non-zero when the leaf certificate does not match.
  const exec = (file, args) => {
    if (file === '/usr/libexec/PlistBuddy') return 'com.iotensity.desktop\n';
    if (
      args.includes('--test-requirement') &&
      args.at(-1).includes('.iotensity-install-')
    )
      throw new Error('test-requirement: code failed to satisfy requirement');
    return '';
  };
  assert.throws(
    () =>
      installBundle({
        ...options,
        verify: (path, fingerprint) => verifyBundle(path, fingerprint, exec),
      }),
    /not signed by the pinned macOS signing identity/,
  );
  assert.equal(version(options.destination), 'old');
  assert.deepEqual(readdirSync(join(options.destination, '..')), [
    'IOTensity.app',
  ]);
});

test('recovers a lock left by an installer that was killed', (t) => {
  const options = fixture(t);
  const { pid } = spawnSync(process.execPath, ['-e', '']);
  writeLock(options, pid);
  assert.equal(installBundle(options), options.destination);
  assert.equal(version(options.destination), 'new');
  assert.deepEqual(readdirSync(join(options.destination, '..')), [
    'IOTensity.app',
  ]);
});

test('reports the installation failure when lock removal also fails', (t) => {
  const options = fixture(t);
  const warn = t.mock.method(console, 'warn', () => {});
  const lock = lockPath(options);
  assert.throws(
    () =>
      installBundle({
        ...options,
        verify: (path) => {
          if (!path.includes('.iotensity-install-')) return;
          rmSync(lock);
          mkdirSync(join(lock, 'blocked'), { recursive: true });
          throw new Error('invalid staged signature');
        },
      }),
    /invalid staged signature/,
  );
  assert.equal(warn.mock.callCount(), 1);
  assert.equal(version(options.destination), 'old');
});
