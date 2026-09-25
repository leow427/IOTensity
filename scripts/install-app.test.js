import assert from 'node:assert/strict';
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
import { installBundle, shouldInstallApp } from './install-app.js';

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
    verify: () => {},
    checkRunning: () => {},
    copy: (from, to) => cpSync(from, to, { recursive: true }),
  };
}
const version = (path) => readFileSync(join(path, 'version'), 'utf8');

test('installs only ordinary local release builds', () => {
  assert.equal(shouldInstallApp('darwin', ['build']), true);
  assert.equal(shouldInstallApp('darwin', ['build', '-v']), true);
  for (const args of [
    ['dev'],
    ['bundle'],
    ['build', '--debug'],
    ['build', '--no-bundle'],
    ['build', '--config', 'test.json'],
    ['build', '--target', 'aarch64-apple-darwin'],
    ['build', '--help'],
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
  assert.equal(version(join(parent, stage, 'previous.app')), 'old');
});

test('does not overwrite another install or follow a destination symlink', (t) => {
  const options = fixture(t);
  const lock = join(options.destination, '..', '.iotensity-install.lock');
  mkdirSync(lock);
  assert.throws(() => installBundle(options), /in progress/);
  assert.equal(existsSync(lock), true);
  rmSync(lock, { recursive: true });
  rmSync(options.destination, { recursive: true });
  symlinkSync(options.source, options.destination, 'dir');
  assert.throws(() => installBundle(options), /not a link/);
  assert.equal(version(options.source), 'new');
});
