# IOTensity

IOTensity is a desktop app that lights your room to match your screen. You lay out lights in a 3D model of the room. The app samples the colors on your display near each light's position and streams them to RGB LEDs driven by ESP32 boards on your local Wi-Fi.

It is meant to stay lightweight. It runs entirely on your own network, with no accounts, cloud services or separate hub. It is built with React, TypeScript, Tauri, Rust and React Three Fiber.

> **Status:** working prototype (v0.1). Screen capture runs on macOS only. The hardware target is one low-current RGB LED per ESP32.

## What it does

- **Your Rooms**: place lights in a 3D room model. The room is measured in metres, with the monitor at the front. A light can be _virtual_ (shown on screen only) or _physical_ (bound to an ESP32). While you position a physical light, its LED shows a calibration color so you can tell which one you are moving.
- **Sync**: choose a color source and press **Start Sync**:
  - **Main display**: captures your screen (macOS, with Screen Recording permission) and gives each light the color of the screen region nearest to it.
  - **Animated colors**: a generated color animation, with an adjustable intensity.
  - **Test image**: a fixed pattern for checking that wiring and placement are right.
- **Mini room**: a small always-on-top preview of the room's live colors, which keeps showing while you use other apps.
- **ESP32 devices** are found automatically on the LAN, can blink on request (**Identify**) and reconnect on their own after a reboot or network drop.

## How it works

```
 Screen ──► Rust: capture, downscale, sample color at each light ──► brightness ──► RGB
                                                                                    │
                     ┌──────────────────────────────────────────────────────────────┤
                     ▼                                                              ▼
       React preview (virtual lights)                       UDP packets, up to 30 FPS ──► ESP32 PWM ──► LED
```

- The **React frontend** (`src/`) handles the room editor, controls and previews. It never handles captured pixels or network traffic.
- The **Rust backend** (`src-tauri/`) handles screen capture, color processing, saving the configuration, device discovery (mDNS), HTTP control and UDP streaming.
- The **ESP32 firmware** (`firmware/esp32/`) is the same image on every board. Each board takes its identity from its factory MAC address. It receives one 29-byte color packet per frame and turns the LED off if packets stop for one second.

Room layouts are saved to `configuration.json` in the app's data directory. Sync always uses the last _saved_ room, never unsaved edits.

## Getting started

**Requirements:** Node 22.12+, stable Rust and, on macOS 12.3+, the Xcode command-line tools. On Windows you need MSVC and WebView2.

```sh
npm ci
npm run tauri -- dev      # run the desktop app in development mode
```

To build and install the release app on macOS, quit any running copy first:

```sh
npm run tauri -- build    # builds, signs and installs ~/Applications/IOTensity.app
```

Release builds must be signed with an Apple Development or Developer ID identity from your Keychain. The [engineering guide](AGENTS.md#setup) explains how the identity is chosen and pinned. Always launch the installed copy, not the bundle in `src-tauri/target`.

`npm run dev` starts a browser-only room editor at `http://127.0.0.1:1420`. It does not save to disk, sync or talk to hardware.

### First run

1. Open **Your Rooms** and add a light with **+ Virtual light** (or **+ Physical light** if an ESP32 is online). Drag it into place, then **Save Room**.
2. Open **Sync**, pick **Animated colors** and press **Start Sync**. The orbs in the preview should start changing color.
3. For screen sync, choose **Main display**. macOS will ask for Screen Recording permission; allow it in System Settings → Privacy & Security and relaunch if asked.

To set up a real LED (wiring, flashing, Wi-Fi provisioning) or try the hardware flow without a board using the emulator, see the [ESP32 setup guide](docs/esp32-setup.md).

## Project layout

| Path              | Contents                                                                    |
| ----------------- | --------------------------------------------------------------------------- |
| `src/`            | React app: domain types and validation, app store, room scene, UI           |
| `src-tauri/`      | Rust backend: configuration files, sync sources, capture, hardware control  |
| `firmware/esp32/` | PlatformIO firmware for the ESP32 receiver                                  |
| `scripts/`        | Build/install wrapper, signing checks, ESP32 emulator, firmware test runner |
| `tests/`          | Unit and component tests, Playwright end-to-end tests, shared fixtures      |
| `docs/`           | Hardware guide                                                              |

## Testing

```sh
npm run check                                          # format, lint, types, unit tests
npm run test:e2e                                       # browser tests (run `npx playwright install chromium` once)
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run test:firmware                                  # firmware logic on the host
```

The [engineering guide](AGENTS.md#verification-and-delivery) has the full list, including clippy and the firmware build. CI runs the frontend and browser tests, native checks on macOS and Windows, and the ESP32 build.

## Documentation

- **[AGENTS.md](AGENTS.md)**: the engineering guide for contributors and coding agents. It covers setup and signing, architecture, configuration and save rules, the color pipeline, the ESP32 HTTP/UDP protocol, UI rules and verification.
- **[docs/esp32-setup.md](docs/esp32-setup.md)**: wiring, flashing, Wi-Fi provisioning, the desktop workflow with a real board, and the LAN emulator.

## Known limitations

- Screen capture works on macOS only and captures the main display in SDR. DRM-protected video may capture as black. Full-screen or system windows can cover the mini room.
- After you change the main display or its resolution, stop Sync and start it again so capture picks up the new display.
- There is no device authentication. Use it only on a trusted home LAN that allows multicast; guest networks and some VPNs block discovery.
- The app supports at most 64 physical lights. Only one computer can stream to a given device at a time.
