import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  chooseIdentity,
  isMacBundle,
  parseIdentities,
  requestedIdentity,
  tauriSubcommand,
} from './signing.js';

const development = {
  fingerprint: 'A'.repeat(40),
  name: 'Apple Development: Example (TEAM)',
};
const distribution = {
  fingerprint: 'B'.repeat(40),
  name: 'Developer ID Application: Example (TEAM)',
};

test('selects valid app-signing identities without using a certificate for another purpose', () => {
  assert.deepEqual(
    parseIdentities(`
    1) ${development.fingerprint} "${development.name}"
    2) ${distribution.fingerprint} "${distribution.name}"
    3) ${'C'.repeat(40)} "Developer ID Installer: Example (TEAM)"
    3 valid identities found
  `),
    [development, distribution],
  );
  assert.deepEqual(chooseIdentity([development]), development);
});

test('keeps the pinned identity when another certificate is installed', () => {
  assert.equal(
    chooseIdentity([development, distribution], development.fingerprint),
    development,
  );
  assert.equal(
    chooseIdentity([development, distribution], distribution.name),
    distribution,
  );
});

test('never silently replaces a missing identity or falls back to ad hoc signing', () => {
  assert.throws(
    () => chooseIdentity([distribution], development.fingerprint),
    /unavailable/,
  );
  assert.throws(
    () => chooseIdentity([development], '-'),
    /Unsigned fallback is disabled/,
  );
  assert.throws(() => chooseIdentity([]), /stable signing identity/);
  assert.throws(
    () => chooseIdentity([development, distribution]),
    /more than one/,
  );
});

test('treats an empty signing override as unset so the pin still applies', () => {
  for (const override of ['', '   ', undefined]) {
    const requested = requestedIdentity(override, development.fingerprint);
    assert.equal(requested, development.fingerprint);
    assert.equal(
      chooseIdentity([development, distribution], requested),
      development,
    );
    // A single other installed certificate must not replace a missing pin.
    assert.throws(
      () => chooseIdentity([distribution], requested),
      /unavailable/,
    );
  }
  assert.equal(
    requestedIdentity(` ${distribution.name} `, development.fingerprint),
    distribution.name,
  );
  assert.equal(requestedIdentity('', undefined), undefined);
});

test('requires signing for macOS bundles while preserving portable CI compilation and help', () => {
  assert.equal(isMacBundle('darwin', ['build']), true);
  assert.equal(isMacBundle('darwin', ['bundle', '--bundles', 'app']), true);
  assert.equal(isMacBundle('darwin', ['build', '--no-bundle']), false);
  assert.equal(isMacBundle('darwin', ['build', '--help']), false);
  assert.equal(isMacBundle('win32', ['build']), false);
  assert.equal(isMacBundle('linux', ['build']), false);
  assert.equal(isMacBundle('darwin', ['dev']), false);
});

test('finds the Tauri subcommand after global options but never after --', () => {
  assert.equal(tauriSubcommand(['build']), 'build');
  assert.equal(tauriSubcommand(['-v', 'build']), 'build');
  assert.equal(tauriSubcommand(['-vv', 'build', '--debug']), 'build');
  assert.equal(tauriSubcommand(['--verbose', 'dev']), 'dev');
  assert.equal(tauriSubcommand(['--', 'build']), undefined);
  assert.equal(tauriSubcommand(['--help']), undefined);
  assert.equal(tauriSubcommand([]), undefined);
});

test('requires signing when global options precede the bundle subcommand', () => {
  for (const args of [
    ['-v', 'build'],
    ['-vv', 'build'],
    ['--verbose', 'build'],
    ['--verbose', 'bundle'],
    ['build', '--', '--foo'],
    // Runner arguments cannot disable Tauri bundling or show Tauri help.
    ['build', '--', '--no-bundle'],
    ['build', '--', '--help'],
  ]) {
    assert.equal(isMacBundle('darwin', args), true, args.join(' '));
  }
  for (const args of [
    ['-v', 'dev'],
    ['-v', 'build', '--no-bundle'],
    ['--help'],
    ['-h', 'build'],
    ['-v', 'build', '--help'],
    ['--', 'build'],
  ]) {
    assert.equal(isMacBundle('darwin', args), false, args.join(' '));
  }
});
