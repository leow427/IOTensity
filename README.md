# IOTensity

A desktop instrument for arranging lights, previewing screen-driven color, and controlling ESP32 RGB LEDs on your local Wi-Fi. Built with React, TypeScript, Tauri 2, Rust and React Three Fiber.

**Screen-sync prototype:** a generated pixel image or the main macOS display drives the existing virtual lights. Native processing uses weighted color averages around each light's saved X/Y room position, favoring nearby vivid colors. The low-resolution analysis image follows the captured display's aspect ratio, including ultrawide and portrait displays. An optional always-on-top mini room lets you watch the response over other content. Virtual previews and physical ESP32 outputs consume the same final RGB values. mDNS discovers devices by permanent eFuse identity; HTTP controls sessions and unicast UDP sends complete colors at up to 30 FPS. There are no accounts, cloud dependencies, DDP or separate hubs. Start with the [ESP32 setup guide](docs/esp32-setup.md).

## Run on macOS

Use macOS 12.3 or later. Install Node 22.12+ and stable Rust with `rustfmt` and `clippy`, plus Xcode command-line tools. Apple Silicon is the primary target.

```sh
npm ci
npm run tauri -- dev
```

For a standalone application:

```sh
npm run tauri -- build
```

The macOS bundle is `src-tauri/target/release/bundle/macos/IOTensity.app`. Local macOS bundles use a pinned signing certificate so rebuilding does not change their permission identity. A valid Apple Development or Developer ID Application identity must be available in Keychain Access; the build stops if it cannot use one. See [stable macOS signing](docs/macos-signing.md). Distribution notarization is outside this milestone. On Windows, install MSVC build tools and WebView2, then use `npm run tauri -- build --no-bundle` to compile the executable. Windows native compilation is included in CI; local interactive verification is macOS only.

On macOS, `tauri dev` uses a separate `IOTensity Dev` identity and room configuration. Use the signed release bundle for screen-sync verification with your saved room.

If this checkout has the project-local toolchain created during development, set these before running Cargo or Tauri:

```sh
export CARGO_HOME="$PWD/.tooling/cargo"
export RUSTUP_HOME="$PWD/.tooling/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
```

`npm run dev` opens a **volatile browser preview** on `http://127.0.0.1:1420`. It deliberately labels itself as not saved to disk. Use the native app to persist rooms across restarts and run the native sync sources. The browser preview deliberately does not emulate screen sync.

## Use

1. Open **Your Rooms**, click **+ Virtual light**, and choose a name and appearance: Bulb, Light Bar, LED Strip or Lamp.
2. Select any orb or its matching card. Drag it in **Location** or **Height** mode, or use the sliders/numeric fields. Drag empty space to orbit, scroll to zoom, and reset the view with the circular arrow.
3. Press **Save Room** or **Cmd+S** / **Ctrl+S**. Only successfully saved lights appear in Sync. Unsaved edits are guarded when navigating or closing the window; Discard restores the saved arrangement.
4. In **Sync**, choose **Synthetic color simulation**, **Deterministic test image** or **Main macOS display**, then **Start Sync**. The test image is red/green above and blue/white below. For display capture, approve Screen Recording in macOS System Settings and relaunch if requested.
5. Choose brightness; the four intensity/smoothing presets apply to the synthetic simulation. Screen and test-image output have no temporal smoothing. Preferences save automatically; native output uses the acknowledged preferences. **Stop Sync** stops capture, holds preview colors, and turns physical LEDs off. Start can then create a fresh capture session. Sync always starts stopped after an application restart.
6. Choose **Open mini room**. Drag its titlebar into a corner and watch the same native output while using other apps. Close it with its × button or **Close mini room** in Sync. Its positions update only when room changes are saved.

Use **+ Physical light** to add a discovered ESP32, or select an existing room light and choose **Bind physical light**. Identify blinks the LED; Save Room commits the binding. Virtual and physical lights have separate trays. Offline devices keep their assignments.

Cards use the same preview colors as their orbs. The selected light shows a calibration guide, even when sync is stopped or brightness is zero. Color changes and selection never modify the saved room.

## Check

```sh
npm run check
npm run build
npx playwright install chromium
npm run test:e2e
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run test:firmware
# With PlatformIO installed:
pio run --project-dir firmware/esp32
```

Frontend tests cover domain behavior, save serialization, errors, guards, cards, final native-color caching, overlay parity and mocked Tauri IPC. Playwright covers the real browser renderer, overflow and keyboard interactions. Rust tests use temporary directories to test disk persistence and failures, plus deterministic pixel-buffer tests for weighted color selection, grayscale fidelity, smooth spatial boundaries, multiple aspect ratios, position mapping, brightness, protocol/session handling, migration and reconnection. The firmware receiver is also compiled and tested on the host, and its exact C++ implementation powers the LAN emulator. These are distinct from actual native-app verification. GitHub Actions runs frontend/browser checks plus native macOS and Windows checks on pushes to `main` and pull requests.

## Configuration and architecture

The only persisted document is `configuration.json` in Tauri's application data directory. On macOS: `~/Library/Application Support/com.iotensity.desktop/configuration.json`. On Windows this is under the user's roaming application-data directory, in `com.iotensity.desktop`. There is no database.

A missing file opens a valid empty Studio room. The first successful room or preference save creates the file. Invalid or unsupported files stop loading and show an error without replacing data. Restore a valid backup at that path and choose **Retry loading**. If a revision conflict occurs, the draft remains visible; restart after noting any unsaved edits to load the externally saved version.

The frontend owns editing. Rust validates and saves snapshots through `load_config` and `save_config`, without a second live room model. A serialized frontend queue and native revision checks prevent stale saves; native writes use a synced temporary file and atomic replacement. Normal navigation, closing and Quit are guarded, but forced process termination cannot preserve unsaved work. Rust keeps a read-only acknowledged configuration for sampling, separately from the frontend room draft. It never owns a live editor model.

Schema 1 files migrate to explicit virtual outputs in memory; schema 2 reaches disk only through a successful atomic save. Corrupt/unsupported files are still errors. Only full hardware IDs are persisted, never IP addresses or runtime state.

See [ESP32 setup](docs/esp32-setup.md), [protocol and native architecture](docs/esp32-architecture.md), [current verification](docs/esp32-verification.md), [screen-sync architecture and usage](docs/screen-sync.md), [weighted-sync verification](docs/weighted-sync-verification.md), [earlier screen-sync verification](docs/screen-sync-verification.md), [original milestone design and save trace](docs/milestone.md), [original milestone verification](docs/verification.md), and [engineering conventions](AGENTS.md).

Open [the local display check](docs/screen-pattern.html) in a browser on the main display to check mapping and response. It provides quadrant colors, a palette swap and an animated swap. This page must actually be visible on the captured display; the generated source is a separate native test.

Display capture is SDR, main-display-only and macOS-only. The entire IOTensity process is excluded, including the mini room. Protected/DRM video may be blank. Full-screen games and unusual window levels may cover the overlay; ordinary desktop windows are the target. Stop and restart capture after display-resolution or main-display changes. The initial transition from an unsigned build to the signed app needs a fresh Screen Recording grant; subsequent local bundles reuse the pinned identity. macOS can still require reauthorization.
