# Stable local macOS signing

Use `npm run tauri -- build` for the native app used for screen sync. The wrapper selects one valid Apple Development or Developer ID Application identity from the macOS keychain and pins its public fingerprint in the ignored `.tooling/macos-signing.json` file. Subsequent builds use that same identity. Private keys remain in Keychain Access; no certificate or private key is exported or committed.

If there is no suitable certificate, or more than one at first setup, the build stops with instructions. Install an Apple Development certificate for local development, or a Developer ID Application certificate for distribution. To explicitly select an installed identity, obtain its fingerprint using `security find-identity -v -p codesigning`, then run:

```sh
APPLE_SIGNING_IDENTITY="YOUR_CERTIFICATE_FINGERPRINT" npm run tauri -- build
```

An explicit override updates the local pin. Without an override, an unavailable or expired pinned certificate causes a clear failure instead of silently switching to an unsigned build. Restore the certificate and its private key or deliberately choose a replacement. The wrapper does not weaken signature requirements, reset macOS permissions, or modify keychain access rules.

The first transition from the old ad hoc build to the signed app requires a fresh Screen Recording grant. Enable the exact release bundle in System Settings → Privacy & Security → Screen & System Audio Recording, then quit and reopen it if macOS requests this. If an old grant remains stuck, quit IOTensity and reset **only** its obsolete grant before adding the signed bundle again:

```sh
tccutil reset ScreenCapture com.iotensity.desktop
```

This is a one-time recovery action, not part of the build or app startup. Subsequent builds retain the app identifier `com.iotensity.desktop` and signing identity. Apple describes how the [designated requirement lets macOS recognize updated code and retain privacy authorization](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements). This fixes the changing-ad-hoc-identity problem; it cannot remove initial authorization, user revocation, certificate/channel changes or reauthorization required by macOS.

`npm run tauri -- dev` uses `com.iotensity.desktop.dev` and the product name `IOTensity Dev` on macOS. Tauri dev launches an unbundled executable, so it has separate permission and configuration identity. It opens its own room configuration. Use the signed release bundle for persistent screen-recording verification. Direct `cargo run` or bypassing the npm wrapper is not the supported permission-stable workflow.

Windows/Linux commands remain unchanged. `--no-bundle` compilation and help commands do not require a signing certificate; existing native CI uses `--no-bundle`. Distribution notarization is separate and has not been configured for this local prototype. Tauri's [macOS signing guide](https://v2.tauri.app/distribute/sign/macos/) documents certificate and notarization setup.

Check a built app with:

```sh
codesign --verify --deep --strict --verbose=2 src-tauri/target/release/bundle/macos/IOTensity.app
codesign -dr - src-tauri/target/release/bundle/macos/IOTensity.app
```

A signed build has an app identifier and certificate requirement; it must not be identified only by a `cdhash` requirement. Run verification where macOS can access its trust store. See [the native verification report](screen-sync-verification.md) for the actual rebuild-and-capture results.
