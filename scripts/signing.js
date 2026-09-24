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

export function isMacBundle(platform, args) {
  return (
    platform === 'darwin' &&
    ['build', 'bundle'].includes(args[0]) &&
    !args.includes('--no-bundle') &&
    !args.includes('--help') &&
    !args.includes('-h')
  );
}
