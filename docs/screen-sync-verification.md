# Screen-sync verification — 18 September 2026

The final signed macOS build exercised real ScreenCaptureKit display capture after the Quit-guard fix. The user confirmed that the mini room remained visible and reacted to the independent Safari color pattern in **both normal and full-screen modes**. This is native application verification, not a browser mock or the generated image source.

## Final state

- App: `src-tauri/target/release/bundle/macos/IOTensity.app`.
- Final executable SHA-256: `f45f274ced159ff0deec30587b9228623f3c97bdd96552eb62f7b9e2daf45610`.
- IOTensity is open and **stopped**, with Main macOS display selected, the mini room closed, and no unsaved edits.
- **Desk left is restored to `(-1.7, 1.2, 1)`**, saved through the existing editor transaction and checked after quitting/relaunching. Corner lamp retains its saved position. Brightness remains **27%**, intensity **Punch**.
- Changes are uncommitted. No push or remote CI run was performed. Toolchains, build products, signing identity pin and test outputs remain ignored.

## Implementation and architecture

The generated BGRA quadrant image and macOS ScreenCaptureKit main-display capture feed the same Rust pipeline: sRGB decode before area downscaling, aspect-preserving analysis image (256×144 for 16:9), approximately 20% sampling rectangles from saved room X/Y, linear temporal smoothing, one linear brightness multiplication, and final sRGB RGB8 events. Z and camera movement do not enter sampling. Native output targets 30 Hz independently of capture callbacks; a single latest-frame slot replaces stale frames.

All four existing intensity presets use their original response constants: Subtle 1.8 s, Balanced 800 ms, Vivid 300 ms, Punch 80 ms. These implement the user's follow-up instead of the original single 60 ms request.

The frontend only caches final native colors and basic status. Existing orbs, cards and the optional 360×310 mini room consume that output without simulating sync or applying brightness again. On macOS, the mini-room webview is hosted in a real nonactivating NSPanel with full-screen/Spaces behavior. The hidden Tauri window retains ownership; closing it restores the content view and closes the panel. The main app keeps its normal Dock/menu behavior. No Objective-C class swapping or new panel dependency is used.

ScreenCaptureKit excludes the entire IOTensity application, including the mini room. Source/running/colors/overlay state are not persisted. The configuration schema, save queue and Save/Discard behavior remain unchanged. CI failure-artifact retention is three days.

Architecture and use: [screen-sync.md](screen-sync.md). Permission-stable build workflow: [macos-signing.md](macos-signing.md).

## Permission fix and rebuild proof

The earlier ad hoc executable was recognized by a hash-only code requirement. Rebuilding changed that identity and invalidated its old Screen Recording authorization. The macOS permission log had reported differing stored/current requirements.

The npm Tauri wrapper now selects a valid installed Apple Development or Developer ID Application identity, pins its public fingerprint in ignored `.tooling/macos-signing.json`, and uses it for macOS bundles. A missing/expired/ambiguous certificate fails clearly; no ad hoc fallback is allowed. Private keys remain in the keychain. No user-specific identity is committed. The macOS dev command has a separate `.dev` bundle/configuration identity so unbundled development does not replace the release app's identity.

One scoped transition was performed: quit the old app, reset only `com.iotensity.desktop` ScreenCapture authorization, enable the exact signed release bundle in System Settings, and Quit & Reopen. No other app's permissions or keychain access rules were changed, and no TCC database was edited.

After that one migration:

| Signed executable                                                  | Native observation                                                                                                                                                                                       |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `95eb5d79bc002defa3d668fe7c69270e616b078003a4bd5234c8ea5e34ec43e1` | Real capture ran, stopped and the app quit.                                                                                                                                                              |
| `5ef7217a325993744f338b8603e77b82e69a038fe57c98463137e9391fb43109` | A real source change and rebuild changed the executable hash. Relaunch and Start reached RUNNING without another permission prompt, reset or Settings visit. The designated requirement stayed the same. |
| `f45f274ced159ff0deec30587b9228623f3c97bdd96552eb62f7b9e2daf45610` | Final native-panel build retained authorization. Real capture, full-screen overlay observation, Stop/Restart and guarded closing were exercised. Strict code-signature verification passed.              |

This verifies the rebuild-related permission fix. It does not remove Apple's initial authorization, user revocation, or any future macOS reauthorization requirement. Changing signing identity/channel can require a fresh grant. This is an Apple Development-signed local build; distribution notarization has not been configured. See Apple's [designated requirement explanation](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).

## Checks actually run

Environment: macOS 26.6.2, Apple Silicon M5 Pro, Node 22.23.2, Rust 1.98.1, Xcode command-line tools installed. Repository-local Rust toolchain:

```sh
export CARGO_HOME="$PWD/.tooling/cargo"
export RUSTUP_HOME="$PWD/.tooling/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
```

| Exact command                                                                                       | Observed result                                                                                                                                                                                                    |
| --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `npm ci`                                                                                            | Passed earlier in this implementation session.                                                                                                                                                                     |
| `npm run check`                                                                                     | Passed: Prettier, ESLint, TypeScript, **34 frontend tests across 6 files**, and **4 Node signing-tool tests**. Run after the signing wrapper/source-help changes; subsequent application changes were native-only. |
| `PLAYWRIGHT_BROWSERS_PATH="$PWD/.tooling/playwright" npm run test:e2e`                              | **4 passed**, including browser WebGL, camera orbit without dirtying data, editor/keyboard/overflow, and automated WCAG AA checks. Run after the signing/source-help changes.                                      |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`                                         | Passed on final native-panel source.                                                                                                                                                                               |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings`           | Passed on final native-panel source.                                                                                                                                                                               |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked`                                          | **16 passed** on final source: 7 processing/service tests and 9 persistence integration tests.                                                                                                                     |
| `npm run tauri -- build`                                                                            | Passed, including TypeScript/Vite, native bridge compilation and signed release bundle creation.                                                                                                                   |
| `codesign --verify --deep --strict --verbose=2 src-tauri/target/release/bundle/macos/IOTensity.app` | Final app valid on disk and satisfies its designated requirement. Run with access to the macOS trust store.                                                                                                        |
| `codesign -dr - src-tauri/target/release/bundle/macos/IOTensity.app`                                | Certificate/app-identifier requirement, stable across the first two different signed builds.                                                                                                                       |
| `git diff --check`                                                                                  | Passed.                                                                                                                                                                                                            |

`npm run format:check` also passed after the final documentation updates.

The production build emits a large-JavaScript-chunk advisory (approximately 1.142 MB before gzip, primarily the existing 3D application) and notes that distribution notarization is not configured. Playwright emits an environment color-variable warning; tests pass. Browser tests required access to bind their local preview server. These are observed local results; configured GitHub Actions jobs are not observed remote results.

Rust tests exercise real generated pixel buffers, quadrant boundaries, ignored depth, linear averaging (black/white averages to sRGB 188), aspect ratio/stride, elapsed-time smoothing on static images, all four response constants, brightness exactly once, successful saved-position retargeting, failed/stale save isolation and initial stopped state. Signing tests cover certificate-purpose filtering, stable pin selection, unavailable/ad hoc identity rejection and portable CI compilation. Mocked IPC/component tests establish frontend behavior, not ScreenCaptureKit functionality.

## Native macOS verification actually performed

### Position sampling and generated image

- In the actual release app, generated image pixels drove Desk left blue and Corner lamp green according to their saved positions.
- A draft Desk left height change from 1.2 to 2.7 left ordinary output blue before Save. Cmd+S acknowledged the save and ordinary output changed to red.
- Orbiting the camera changed the view without changing output or dirtying the room.
- Native 3D dragging changed X/Z and dirtied the draft; that test drag was discarded.
- Selected editor calibration remained visual-only; orbs and cards displayed corresponding native output after deselection.

These position/camera checks were performed earlier in this implementation session. On the final build, Desk left was saved back to exactly 1.2 m with the overlay/capture running, and its original XYZ was confirmed again after quit/relaunch. A separate final-build visual top-to-bottom sample comparison during that restoration was not recorded.

### Real capture and overlay

- Real main-display ScreenCaptureKit capture reached RUNNING on the final build after the Quit-guard fix, with no renewed permission prompt.
- Earlier native checks compared the independent Safari page's red/green and cyan/magenta quadrants with corresponding room/card/mini-room output. These used real display capture, not the generated source.
- On the final native-panel build, the user watched the actual Safari color-test tab and confirmed **“Works in both”** when asked whether the mini room stayed visible and reacted in both normal-window and page Full screen modes.
- Early stacking checks were inconclusive/negative: the old overlay was not seen, and automation sometimes operated a background Safari window the user could not see. The final confirmation followed opening the pattern in the user's visible Safari window. Individual-window automation screenshots are not claimed as proof of desktop stacking.
- With the final overlay open, Stop reached STOPPED and Start returned to RUNNING. STOPPED is exposed only after ScreenCaptureKit's stop completion. No additional permission prompt occurred.
- Closing and reopening the final native panel through the main app were exercised while real capture remained RUNNING.
- All four intensity controls were selected during an earlier actual capture run without errors, then Punch was restored. Formula/timing differences are established by deterministic Rust tests; native response times and sustained 30 Hz throughput were not measured.

### Closing and persistence on the final build

- With real capture and the overlay active, an unsaved one-step height change (1.2 → 1.21) was made solely for the close-guard test.
- The ordinary main-window close button displayed Save / Discard / Stay. Stay preserved the draft.
- Cmd+Q displayed the same guard. Discard quit the app. A process check (`pgrep -x iotensity`, with process-list access) returned no matching process, so no IOTensity capture process was left running.
- Relaunch started STOPPED with the overlay closed. Desk left remained saved at `(-1.7, 1.2, 1)`, confirming the test draft had not overwritten the restoration. Brightness/intensity remained 27%/Punch.
- The mini room's own close button was exercised before the native-panel change; its own close button and titlebar dragging were not separately re-exercised after that change. Main-app overlay close/reopen and whole-app teardown were exercised on the final panel build.

## Changed files

- `.github/workflows/ci.yml`: three-day failure-artifact retention.
- `README.md`, `docs/screen-sync.md`, `docs/screen-sync-verification.md`, `docs/macos-signing.md`, `docs/screen-pattern.html`: use, architecture, signing, evidence and independent display pattern.
- `package.json`, `scripts/tauri.js`, `scripts/signing.js`, `scripts/signing.test.js`: permission-stable signed builds, separate macOS dev identity and tooling checks.
- `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/build.rs`, `src-tauri/Info.plist`, `src-tauri/tauri.conf.json`: native bridge build, frameworks, capture usage description and macOS deployment target.
- `src-tauri/capabilities/overlay.json`: narrow overlay event/titlebar capability.
- `src-tauri/src/config.rs`: shared native bounds; unchanged configuration shape.
- `src-tauri/src/lib.rs`, `src-tauri/src/overlay.rs`, `src-tauri/src/overlay.m`: commands, saved acknowledgments, guarded Quit, overlay ownership and native NSPanel.
- `src-tauri/src/sync/mod.rs`, `src-tauri/src/sync/processing.rs`, `src-tauri/src/sync/capture.rs`, `src-tauri/src/sync/capture.m`: native worker, processing/tests and ScreenCaptureKit bridge.
- `src/sync/output.ts`, `src/main.tsx`, `src/state/store.ts`: passive native output, overlay entry point and existing store integration.
- `src/domain/colors.ts`, `src/scene/RoomScene.tsx`, `src/ui/LightCards.tsx`: final native colors without duplicate brightness or frontend sync animation.
- `src/ui/SyncPage.tsx`, `src/ui/MiniRoom.tsx`, `src/ui/OverlayControl.tsx`, `src/styles.css`: source/status controls and mini-room view.
- `tests/app.test.tsx`, `tests/domain.test.ts`, `tests/helpers.ts`, `tests/store.test.ts`, `tests/sync-output.test.ts`, `tests/overlay.test.tsx`, `tests/e2e/workflow.spec.ts`: behavior and regression checks.
- Removed `src/simulation/engine.ts` and `tests/simulation.test.ts`: frontend synthetic sync engine replaced by native processing tests.

## Known limits and unverified coverage

- Main-display capture is macOS-only and SDR-only. No networking, physical devices, window capture, HDR, mapping controls, black-bar detection or advanced image analysis were added.
- Safari normal/full-screen color content was exercised. Arbitrary movies, DRM video, games and exclusive full-screen applications were not. Protected content may appear blank; system windows or exclusive applications may outrank the overlay.
- Main/overlay application exclusion is configured in ScreenCaptureKit. A dedicated move-the-overlay-across-a-sampled-region feedback comparison was not performed on the final panel build.
- Exceptional capture errors beyond permission denial, display disconnect/resolution changes, rotation, sleep/wake and multi-monitor arrangements remain unverified. Stop/restart after changing display configuration.
- macOS 12.3 is the deployment minimum; actual native verification was on macOS 26.6.2. Older macOS versions were not exercised.
- Windows compilation is configured in CI but was not run locally or observed remotely. No remote CI result is claimed.
- Sustained 30 Hz and CPU cost on high-resolution displays were not benchmarked. The pipeline uses CPU linear-light reduction at source resolution.
- The native Dock Quit path and forced termination were not exercised. Forced termination is not guarded.
- The supported local build workflow requires the pinned signing certificate. Initial/OS-required privacy authorization and distribution notarization remain separate from the rebuild-permission fix.
