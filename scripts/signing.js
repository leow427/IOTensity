// Public certificate fingerprints only. Private keys stay in the macOS keychain.
export function parseIdentities(output) {
  return [...output.matchAll(/^\s*\d+\) ([A-Fa-f0-9]{40}) "([^"]+)"/gm)]
    .map(([, fingerprint, name]) => ({
      fingerprint: fingerprint.toUpperCase(),
      name,
    }))
    .filter(({ name }) =>
      /^(Apple Development|Developer ID Application): /.test(name),
    );
}

// An empty or blank override is treated as unset, so it cannot bypass the pin.
export function requestedIdentity(override, pinnedFingerprint) {
  const explicit = override?.trim();
  return explicit || pinnedFingerprint;
}

export function chooseIdentity(identities, requested) {
  if (requested) {
    const matches = identities.filter(
      ({ fingerprint, name }) =>
        fingerprint === requested.toUpperCase() || name === requested,
    );
    if (matches.length === 1) return matches[0];
    throw new Error(
      'The configured macOS signing identity is unavailable or ambiguous. Restore that certificate and its private key in Keychain Access, or explicitly select a replacement with APPLE_SIGNING_IDENTITY. Unsigned fallback is disabled.',
    );
  }
  if (identities.length !== 1)
    throw new Error(
      'A stable signing identity is required for macOS app bundles. Install an Apple Development or Developer ID Application certificate, then select it with APPLE_SIGNING_IDENTITY if more than one is available. CI can compile with --no-bundle.',
    );
  return identities[0];
}

// Tauri's only global options (-v/--verbose, -h/--help, -V/--version) take no
// value. Skip options to find the subcommand; tokens after `--` belong to the
// runner, so they never select one.
export function tauriSubcommand(args) {
  for (const arg of args) {
    if (arg === '--') return undefined;
    if (!arg.startsWith('-')) return arg;
  }
  return undefined;
}

// Tauri's own options; anything after `--` is passed through to the runner.
function tauriOptions(args) {
  const end = args.indexOf('--');
  return end === -1 ? args : args.slice(0, end);
}

export function isMacBundle(platform, args) {
  const options = tauriOptions(args);
  return (
    platform === 'darwin' &&
    ['build', 'bundle'].includes(tauriSubcommand(args)) &&
    !options.includes('--no-bundle') &&
    !options.includes('--help') &&
    !options.includes('-h')
  );
}
