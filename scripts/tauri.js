import { spawn, execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chooseIdentity, isMacBundle, parseIdentities } from './signing.js';
import {
  appInstallation,
  assertNotRunning,
  installBundle,
  shouldInstallApp,
} from './install-app.js';

const root = fileURLToPath(new URL('../', import.meta.url));
const args = process.argv.slice(2);
const env = { ...process.env };

try {
  if (isMacBundle(process.platform, args)) {
    const pinPath = join(root, '.tooling', 'macos-signing.json');
    const pin = existsSync(pinPath)
      ? JSON.parse(readFileSync(pinPath, 'utf8'))
      : null;
    if (pin && !/^[A-F0-9]{40}$/.test(pin.fingerprint))
      throw new Error(`Invalid signing identity file: ${pinPath}`);
    const identities = parseIdentities(
      execFileSync(
        '/usr/bin/security',
        ['find-identity', '-v', '-p', 'codesigning'],
        {
          encoding: 'utf8',
        },
      ),
    );
    const identity = chooseIdentity(
      identities,
      env.APPLE_SIGNING_IDENTITY ?? pin?.fingerprint,
    );
    // Pin once, so adding another certificate cannot silently change app identity.
    // An explicit environment override deliberately selects and pins a replacement.
    if (pin?.fingerprint !== identity.fingerprint) {
      mkdirSync(dirname(pinPath), { recursive: true });
      writeFileSync(
        pinPath,
        `${JSON.stringify({ fingerprint: identity.fingerprint }, null, 2)}\n`,
        { mode: 0o600 },
      );
    }
    env.APPLE_SIGNING_IDENTITY = identity.name;
    console.log(
      'Using the pinned macOS signing identity; unsigned fallback is disabled.',
    );
  }
  if (
    process.platform === 'darwin' &&
    (args[0] === 'dev' || args.includes('--debug'))
  ) {
    // Tauri dev runs an unbundled executable. Its permissions and configuration
    // must never replace those belonging to the signed release app.
    args.push(
      '--config',
      JSON.stringify({
        identifier: 'com.iotensity.desktop.dev',
        productName: 'IOTensity Dev',
      }),
    );
  }
  const installation = shouldInstallApp(process.platform, args, env)
    ? appInstallation(root, env)
    : null;
  if (installation) {
    assertNotRunning(installation.source);
    assertNotRunning(installation.destination);
  }
  const require = createRequire(import.meta.url);
  const cli = join(
    dirname(require.resolve('@tauri-apps/cli/package.json')),
    'tauri.js',
  );
  const child = spawn(process.execPath, [cli, ...args], {
    cwd: root,
    env,
    stdio: 'inherit',
  });
  for (const signal of ['SIGINT', 'SIGTERM'])
    process.on(signal, () => child.kill(signal));
  child.on('error', (error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
  child.on('exit', (code) => {
    process.exitCode = code ?? 1;
    if (code === 0 && installation) {
      try {
        console.log(`Installed current build: ${installBundle(installation)}`);
      } catch (error) {
        console.error(
          `Build completed, but installation failed: ${error.message}`,
        );
        process.exitCode = 1;
      }
    }
  });
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
