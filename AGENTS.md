# IOTensity engineering guide

## Setup

Use Node 22.12+ and stable Rust with rustfmt/clippy. macOS 12.3+ needs Xcode command-line tools; Windows needs MSVC and WebView2. macOS is the primary interactive verification target; display capture is macOS-only.

```sh
npm ci
npm run tauri -- dev
# Quit the installed app, then build and install the standalone application:
npm run tauri -- build
```

Ordinary local macOS `npm run tauri -- build` builds, verifies and installs **`~/Applications/IOTensity.app`**. **After every successful implementation, build and install the newest app, then launch and verify that installed copy before declaring the work complete.** Updating source or pushing to GitHub alone is not completion; the user's installed app must also be current. Quit normally before building: the wrapper refuses to replace a running app, stages and verifies the new bundle before replacement, and rolls back installation failures. Build artifacts remain at `src-tauri/target/release/bundle/macos/IOTensity.app` (or Cargo's configured target directory); do not launch these or old worktree copies. Custom builds, CI, `--debug`, and `--no-bundle` do not auto-install.

The npm wrapper requires an Apple Development or Developer ID Application identity in Keychain and pins its public fingerprint in ignored `.tooling/macos-signing.json`. If selection is ambiguous, use `security find-identity -v -p codesigning`, then `APPLE_SIGNING_IDENTITY="FINGERPRINT" npm run tauri -- build`. Missing/expired pinned identities must fail; never silently switch certificates or use unsigned fallback. Private keys stay in Keychain. Distribution notarization is not configured; `--no-bundle` compilation needs no signing identity.

macOS dev and debug bundles use `com.iotensity.desktop.dev` / **IOTensity Dev**, with separate configuration and permissions. Verify capture using the signed release bundle: allow Screen Recording in System Settings → Privacy & Security and relaunch if requested. Keep the bundle and signing identity stable across rebuilds; do not automatically reset permissions. Allow local network access for ESP32 discovery.

`npm run dev` serves the volatile browser editor at `http://127.0.0.1:1420`; it has no disk persistence or native sync. If using an existing local Rust bootstrap, set `CARGO_HOME="$PWD/.tooling/cargo"`, `RUSTUP_HOME="$PWD/.tooling/rustup"`, and prepend `$CARGO_HOME/bin` to `PATH`. Never hardcode user-specific paths. Keep `.tooling`, credentials, build products and test output out of Git.

For wiring, flashing, provisioning and emulator commands, use [ESP32 setup](docs/esp32-setup.md).

## Architecture and scope

Support local ESP32 RGB outputs, virtual previews and native screen sync. Do not add accounts, cloud services, DDP or a separate physical hub. Discovery, HTTP, UDP, sessions, capture and color processing belong in Rust; captured pixels never enter React.

- `src/domain`: portable configuration types, bounds, validation and preview color math.
- `src/state/store.ts`: one `useSyncExternalStore` store owns saved configuration, room draft, selection, guards and the serialized save queue.
- `src/persistence`: typed Tauri commands; `src-tauri/src/config.rs`: native validation and atomic file transactions, without a live editor model.
- `src-tauri/src/sync`: application-owned screen/test/synthetic sources; page navigation does not own their lifetime. Synthetic math lives in `hardware/engine.rs` and takes injected elapsed time.
- `src-tauri/src/hardware`: mDNS discovery, identity-verified HTTP control and UDP scheduling. Consume final RGB8 without extra brightness, gamma or smoothing.
- `src/sync/output.ts` and `src/hardware/client.ts`: passive native output/discovery clients. React handles controls and binding; no hardware transmission timers.
- `src/scene` and `src/ui`: room rendering, interactions and bundled SVGs. Cards derive from draft lights keyed by stable IDs. `MiniRoom` is a read-only preview of saved positions and the same output; closing it does not stop sync. macOS uses a nonactivating `NSPanel`, Windows an always-on-top Tauri window.
- `firmware/esp32`: identical firmware per board, eFuse identity, replaceable serial provisioning, separate HTTP/serial and UDP/PWM tasks. PWM is 20 kHz/8-bit; powered-prototype Wi-Fi sleep is disabled.

## Configuration and saves

Only `saved` drives Sync. Room edits change `draft`; selection is one `selectedLightId`. Compare the draft with the saved room for dirty state, including reversal. Selection, camera, tabs and preview colors never dirty it.

Every save goes through `AppStore.commit`, building from the latest acknowledged configuration when its queue entry executes. Preference saves are debounced 400 ms and must never include the draft. Freeze room edits during room saves, reject duplicate submissions, mark saved only after acknowledgement, and preserve the draft after failure. Native services accept only acknowledged, non-regressing revisions.

Native writes validate, read the current document, check its revision, sync a same-directory temporary file and atomically replace the target. `configuration.json` lives in Tauri's app-data directory: `~/Library/Application Support/com.iotensity.desktop/` on macOS, the `com.iotensity.desktop` directory under roaming application data on Windows. A missing file means an empty Studio; broken, unreadable or unsupported files are errors, never silently reset. Restore a valid backup and retry loading. A revision conflict preserves the draft; note unsaved edits before reloading.

Schema 2 preserves logical `VirtualLight.id` and adds required `output: { kind: 'virtual' } | { kind: 'esp32'; deviceId: string }`. Persist full eFuse IDs (`esp32-` + 12 lowercase MAC hex digits), never recognition-only `IOT-A1B2C3` names. Reject duplicate hardware bindings across rooms; support at most 64 physical outputs. Schema 1 migrates to virtual outputs in memory; only an acknowledged save writes schema 2. Keep TypeScript/Rust strict shapes aligned with `tests/fixtures/configuration*.json`. Never persist IPs, ports, online state, sessions, sequences, RGB frames, selection, camera, timers, source or running state.

## Color output

ScreenCaptureKit supplies the latest complete BGRA/sRGB/SDR frame, excluding the entire app, cursor and audio. No recordings or application frame queues. Decode to linear RGB, reduce once with aspect preserved (longest side ≤256, no upscaling), and retain weighted color moments. A bounded `1 + 3 × linear chroma` weight favors vivid pixels without discarding neutrals. Sample an integrated tent around each saved X/Y position: `u=(x+3)/6`, `v=1-(y-0.15)/2.85`, radius 0.1, clipped and normalized. Z and camera transforms do not affect sampling. Apply brightness once, encode to final sRGB RGB8, then feed both preview events and physical output.

Screen/test-image colors update directly. Synthetic colors depend on elapsed time and stable light ID, with Subtle/Balanced/Vivid/Punch response times of 1.8/0.8/0.3/0.08 seconds; reduced motion slows this source. Never apply that smoothing again in transport. Passive webview caches reject older snapshots; scene and card-row paint loops read colors without per-frame React state or per-card simulation.

Native output targets up to 30 FPS, repeats static colors and avoids catch-up bursts. Stop releases capture, holds virtual preview colors and turns physical LEDs off. App restart starts stopped with the mini room closed. Restart capture after changing the main display/resolution. SDR main-display capture is the current limit; DRM content may be blank, and exclusive full-screen/system windows may cover the overlay.

## ESP32 protocol and reconnects

Use trusted private/link-local IPv4 LANs; this prototype has no cryptographic device authentication. Browse `_iotensity._tcp.local.`: SRV gives the HTTP port; TXT contains `id`, `model=esp32-rgb`, `pv=1`. Coalesce by full identity, retain saved offline entries, and rediscover current addresses. Refresh browsing every ten seconds. Recreate the entire mDNS daemon/socket on interface changes or Retry discovery, and after 5/10/20/30-second delays with no verified online device. Preserve healthy streams and bindings during recovery. Reopening only the browser cannot recover a stale multicast socket; [Apple documents problems with multiple app copies](https://developer.apple.com/forums/thread/809211). Verify identity and capabilities over HTTP before control; reject an old IP occupied by another device. No frontend URLs, proxies or redirects.

HTTP JSON API: GET `/v1/info` or `/v1/status`; POST `/v1/identify` with `deviceId`; POST `/v1/stream/start` with `deviceId`, `protocol=1`, `clientId`, `requestId`; POST `/v1/stream/stop` with `deviceId`, `sessionId`. Status includes identity/model/protocol, UDP port, `maxFps=30` and nullable session ID. Tokens are 128 bits encoded as 32 lowercase hex characters. Start retries retain the request ID; a new request creates a fresh session bound to the controller's IP. Only one active controller is allowed (otherwise HTTP 409). Identify blinks for 900 ms. Per-device control workers use bounded requests (300 ms connect, 600 ms total, 8 KiB response), 500 ms status probes and failure backoff capped at four seconds; they never block UDP.

UDP v1 is exactly **29 bytes**: `IOTL` (0–3), version 1 (4), flags 0 (5), session token (6–21), big-endian uint32 sequence (22–25), RGB8 (26–28). Validate length, marker, version, flags, owner IP and session. After the first frame, accept only `0 < (next - previous) mod 2^32 < 2^31`; ignore duplicates, old frames and old sessions. Each packet is a complete color; no ACKs, retransmissions or deltas.

Repeat static colors with fresh sequences. Firmware returns to off after 1,000 ms without a valid frame, or on stop/reboot/Wi-Fi loss; invalid traffic cannot refresh the timeout. Native output stops if its source stalls for 250 ms. Binding changes discard obsolete targets and late control responses. Reconnect/reboot negotiates a new session only while Sync remains requested; explicit Stop suppresses streaming. Best-effort shutdown control is backed by the firmware watchdog.

## Editor and UI rules

One scene unit is one metre; origin is the floor below the monitor centre. X increases right (−3..3), Y up (0.15..3), Z into the room (−0.7..4). All inputs share the same clamp. Location changes X/Z only; Height changes Y only. Camera transforms are not room coordinates. Orbs/cards share `resolveColor`: selected-editor calibration overrides ordinary output even at zero brightness; Location uses X-only green → orange, Height cyan → violet. Calibration is preview-only; use Identify for physical recognition.

Keep separate virtual/physical add buttons and lists. Binding an existing light retains its logical ID; binding edits take effect only after Save Room. Offline devices keep assignments. Preserve keyboard access, focus restoration, reduced motion, horizontal card scrolling without page overflow, warm surfaces, charcoal displays and restrained orange accents. Use local SVG icons, no emoji/branded assets, per-orb dynamic lighting or shadows.

Keep native capabilities narrow: event listening, titlebar dragging and confirmed window destruction; no shell, broad filesystem permissions or frontend network access. Keep macOS purpose strings explicit. Guard navigation, window close and Quit with Save/Discard/Stay; forced termination is not protected. Keep transactions/domain logic portable to Windows.

## Verification and delivery

```sh
npm run check
npx playwright install chromium # Once per environment
npm run test:e2e
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run test:firmware
pio run --project-dir firmware/esp32 # Install PlatformIO as in the ESP32 guide
npm run tauri -- build
```

Run checks appropriate to the change. Use meaningful domain/store/component/IPC/Rust/browser tests and injected clocks for simulation. Rust/C++ share `tests/fixtures/udp-v1.hex`; the LAN emulator uses the actual firmware receiver. Emulator evidence cannot establish wiring, PWM, real DHCP/power-cycle behavior or optical latency. Verify native dragging, shortcuts, close guards, restart persistence, capture and reconnects in the actual app. For display checks, show [the color-pattern fixture](tests/fixtures/screen-pattern.html) on the captured display. Browser/mock checks cannot establish native behavior. Check macOS signatures with `codesign --verify --deep --strict --verbose=2` against the built bundle; notarization is separate.

CI covers frontend/browser, macOS/Windows native checks and ESP32 compilation. Report configured CI separately from observed results. Preserve unrelated work, run checks before committing and pushing to `main`, never force push or bypass safeguards. Keep development history and old verification reports in Git, not additional documentation files.
