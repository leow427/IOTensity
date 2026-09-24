# IOTensity engineering conventions

## Scope and architecture

This milestone supports local ESP32 RGB outputs and native screen sync. Do not add accounts, cloud services, DDP or a separate physical hub. Keep discovery, HTTP control, UDP transmission, session state, capture and color processing in Rust; no captured pixel buffers belong in React.

- `src/domain`: portable configuration types, bounds, validation and color math.
- `src/state/store.ts`: one external store consumed with React `useSyncExternalStore`. Owns saved configuration, room draft, selection, navigation guards and the serialized save queue.
- `src-tauri/src/sync`: application-owned native screen/test/synthetic color sources with injected elapsed time for deterministic tests. Navigation must not own their lifetime.
- `src-tauri/src/hardware`: mDNS discovery, identity-verified control, sessions and bounded UDP output. Consume final RGB8 from sync without additional brightness or smoothing.
- `firmware/esp32`: identical firmware per board, eFuse identity, serial development provisioning and a separately scheduled UDP/PWM receiver.
- `src/scene`: React Three Fiber rendering and pointer interactions. Camera transforms are not room coordinates.
- `src/ui`: controls and bundled vector silhouettes. Generate cards directly from draft lights, keyed by stable IDs.
- `src/persistence`: typed Tauri command client. Browser preview is volatile and must never claim disk persistence.
- `src-tauri/src/config.rs`: native validation and atomic local file transactions. Rust does not maintain a live editor model.

## State and saves

Only `saved` is used by Sync. Editing changes `draft`; selection is one `selectedLightId`. Selection, camera movement, tabs and preview colors must not dirty the draft. Equality against the saved room determines dirty state, including reversal of changes.

Every save goes through `AppStore.commit`. Build each transaction from the latest acknowledged configuration when its queue entry executes. Preference transactions must never include the draft. Freeze room edits during a room save; reject duplicate submissions. Mark saved only after a native acknowledgement. Preserve the draft after failure.

Native writes validate inputs, read the existing document, check its revision, write and sync a same-directory temporary file, and atomically replace the target. A missing file means first use; broken, unreadable and unsupported files are errors. Do not silently reset them. Keep Rust and TypeScript shapes aligned using `tests/fixtures/configuration.json` and boundary tests. Never persist runtime colors, selection, camera, timers or running state.

## Coordinates and rendering

One scene unit is one metre. Origin is the floor below the monitor centre. X increases right, Y up, Z from the monitor into the room. Bounds: X −3..3, Y 0.15..3, Z −0.7..4. Location changes X/Z only; Height changes Y only. All input paths use the same clamp. Location calibration depends only on X (green → orange); Height uses cyan → violet. Calibration overrides ordinary output only for the selected editor light and remains visible at zero brightness.

Orbs and cards call the same `resolveColor`. Do not add per-card simulation loops. Scene `useFrame` and one card-row paint loop read the simulation without putting every animation frame in React state. Avoid per-orb dynamic lighting or shadows.

## Commands and verification

Use Node 22.12+ and current stable Rust with rustfmt/clippy. On macOS, Xcode command-line tools are required. Windows needs MSVC and WebView2.

```sh
npm ci
npm run tauri -- dev
npm run check
npm run test:e2e
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run test:firmware
pio run --project-dir firmware/esp32
npm run tauri -- build
```

Install Chromium once with `npx playwright install chromium`. `npm run dev` is browser preview, not proof of native persistence. Keep `.tooling`, build products and test output out of Git. The optional local Rust bootstrap in `.tooling` needs its own `CARGO_HOME`, `RUSTUP_HOME` and `PATH`; do not bake user-specific paths into source.

Run meaningful domain, store, component, mocked IPC, Rust and browser tests. Use an injected clock, not real sleeps, for simulation. Verify the actual macOS app for 3D dragging, shortcuts, close guards and restart persistence. Browser or mocked IPC checks cannot establish native verification. Report configured CI separately from observed remote results.

## UI and platform considerations

Keep the desktop instrument aesthetic: quiet warm surfaces, charcoal displays, legible labels, restrained orange accents and tactile controls. Icons are local SVGs, never emoji or branded assets. Preserve keyboard access, focus visibility, reduced-motion behavior and a horizontally scrolling light row without page overflow.

Native capabilities are deliberately narrow: event listening, titlebar dragging and confirmed window destruction. Do not add shell or broad filesystem permissions or frontend network access. Native screen capture and local ESP32 discovery/control are the only network/capture scope; keep their macOS purpose strings explicit. macOS is the first verification target; keep file transactions and domain logic portable for Windows. Guard ordinary window closing; do not claim protection against forced termination.

Preserve unrelated changes. Run checks before committing and pushing to `main`; never force push or bypass safeguards. Keep development history in Git, not this file.
