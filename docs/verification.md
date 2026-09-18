# Verification record

Verified locally on 2026-09-18, on an Apple Silicon Mac. Browser checks, mocked IPC and real native interaction checks are separate evidence.

## Automated checks

- `npm run check`: passed Prettier, ESLint, TypeScript and **31 frontend tests**. Includes domain bounds/gradients, draft ownership, failed saves, queued preference/room saves, editing freeze, navigation/close guards, card identity/appearance, deterministic simulation and mocked Tauri IPC.
- `npm run test:e2e`: **4 passed** in Chromium. Covers the real WebGL renderer, save/discard/navigation flow, keyboard card selection, horizontal overflow at 1280 and 1000 px, zero-brightness calibration and automated WCAG A/AA checks on both screens.
- `cargo test --locked`: **9 persistence tests passed** with temporary directories. Includes disk round-trip/reopen, native input validation, malformed/unsupported files, stale concurrent saves and a real unwritable-directory failure that preserves the old bytes.
- `cargo fmt -- --check` and `cargo clippy --locked --all-targets -- -D warnings`: passed.
- Frontend production build, native macOS debug app bundle, and native macOS **release app bundle**: passed.
- npm dependency audit after installation: no known vulnerabilities reported.

## Actual macOS Tauri interaction checks

The bundled application was launched from `src-tauri/target/release/bundle/macos/IOTensity.app`, displaying `tauri://localhost`, not the browser preview.

- First use opened an empty Studio with Start Sync disabled. Adding a light created its orb and card immediately.
- Named a light “Desk left”, selected Light Bar, and saved with Cmd+S. Added “Corner lamp” with the Lamp appearance. Save Room wrote the actual application-data file.
- Native verification exposed a JSON-property-order dirty-state bug. The fix compares domain values; regression coverage and the in-memory persistence mock now include Rust's serialization order. Retesting confirmed **Saved to disk**, a disabled Save button and cleared dirty state after acknowledgement.
- Numeric Location editing preserved Y. Dragging Height changed Y from 1.20 to 2.77 while preserving X/Z at 2.00/2.80. Location dragging then clamped X to 3.00, changed Z to 2.30, and kept Y at 2.77. The camera stayed stationary during light drags.
- Orbiting changed the view without changing coordinates or dirty state. Reset restored the camera. Clicking the “Desk left” orb selected its card and opened the matching editor.
- Start/Stop, Punch intensity and brightness visibly affected the synthetic output. Navigation to Your Rooms and back retained the running service and elapsed time. At zero brightness, selected calibration, selection ring and card text remained visible. Stop retained the last output.
- A normal window-close request with a dirty room showed **Save / Discard / Stay**. Stay returned to the intact draft; Discard closed the window without replacing the saved room.
- Fully closed and reopened the application. Both light names, appearances and positions returned, along with brightness **42%** and **Punch** intensity. The timer reset to `00:00`, simulation was stopped and selection was empty. The native JSON file was also inspected independently.

The local application-data document contains the two virtual lights used for these checks; it is outside the repository. New installations still start empty.

## CI and limits

[GitHub Actions](https://github.com/leow427/IOTensity/actions) is configured for pushes to `main` and pull requests: frontend/browser checks, plus macOS and Windows native formatting, lint, tests and compilation. **Configured does not mean passed**; inspect the run associated with the relevant pushed commit. Local test results above do not establish remote CI status.

Windows interactive behavior and signing/notarization have not been verified locally. Forced process termination does not protect unsaved drafts. Vite emits a bundle-size advisory for the embedded Three.js renderer; the build succeeds and all assets ship locally. Automated accessibility checks supplement keyboard/native inspection and are not a complete accessibility audit.

No hardware, screen capture, streaming or real light control is implemented or verified by this milestone.
