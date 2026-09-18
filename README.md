# IOTensity

A small desktop instrument for arranging virtual lights and exploring synthetic color. Built with React, TypeScript, Tauri 2, Rust and React Three Fiber.

**This first milestone is simulated.** It does not capture your screen, stream pixels, discover devices or control physical lights. There are no hardware services, accounts or cloud dependencies.

## Run on macOS

Install Node 22.12+ and stable Rust with `rustfmt` and `clippy`, plus Xcode command-line tools. Apple Silicon is the primary target.

```sh
npm ci
npm run tauri -- dev
```

For a standalone application:

```sh
npm run tauri -- build
```

The macOS bundle is `src-tauri/target/release/bundle/macos/IOTensity.app`. This is a local unsigned build; distribution signing/notarization is outside this milestone. On Windows, install MSVC build tools and WebView2, then use `npm run tauri -- build --no-bundle` to compile the executable. Windows native compilation is included in CI; local interactive verification is macOS only.

If this checkout has the project-local toolchain created during development, set these before running Cargo or Tauri:

```sh
export CARGO_HOME="$PWD/.tooling/cargo"
export RUSTUP_HOME="$PWD/.tooling/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
```

`npm run dev` opens a **volatile browser preview** on `http://127.0.0.1:1420`. It deliberately labels itself as not saved to disk. Use the native app to persist rooms across restarts.

## Use

1. Open **Your Rooms**, click **+ Add light**, and choose a name and appearance: Bulb, Light Bar, LED Strip or Lamp.
2. Select any orb or its matching card. Drag it in **Location** or **Height** mode, or use the sliders/numeric fields. Drag empty space to orbit, scroll to zoom, and reset the view with the circular arrow.
3. Press **Save Room** or **Cmd+S** / **Ctrl+S**. Only successfully saved lights appear in Sync. Unsaved edits are guarded when navigating or closing the window; Discard restores the saved arrangement.
4. In **Sync**, start the simulated color animation, change brightness or intensity, and stop to hold the last output. Preferences save automatically. Simulation always starts stopped after a full restart.

Cards use the same preview colors as their orbs. The selected light shows a calibration guide, even when simulation is stopped or brightness is zero. Color changes and selection never modify the saved room.

## Check

```sh
npm run check
npm run build
npx playwright install chromium
npm run test:e2e
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Frontend tests cover domain behavior, save serialization, errors, guards, cards, deterministic animation and mocked Tauri IPC. Playwright covers the real browser renderer, overflow and keyboard interactions. Rust tests use temporary directories to test disk persistence and failures. These are distinct from actual native-app verification. GitHub Actions runs frontend/browser checks plus native macOS and Windows checks on pushes to `main` and pull requests.

## Configuration and architecture

The only persisted document is `configuration.json` in Tauri's application data directory. On macOS: `~/Library/Application Support/com.iotensity.desktop/configuration.json`. On Windows this is under the user's roaming application-data directory, in `com.iotensity.desktop`. There is no database.

A missing file opens a valid empty Studio room. The first successful room or preference save creates the file. Invalid or unsupported files stop loading and show an error without replacing data. Restore a valid backup at that path and choose **Retry loading**. If a revision conflict occurs, the draft remains visible; restart after noting any unsaved edits to load the externally saved version.

The frontend owns editing. Rust validates and saves snapshots through `load_config` and `save_config`, without a second live room model. A serialized frontend queue and native revision checks prevent stale saves; native writes use a synced temporary file and atomic replacement. Normal navigation/close is guarded, but forced process termination cannot preserve unsaved work.

See [milestone design and save trace](docs/milestone.md), [verification record](docs/verification.md), and [engineering conventions](AGENTS.md).
