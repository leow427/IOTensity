# ESP32 milestone verification

Verified on macOS with an ESP32 emulator on the computer's private Wi-Fi interface. No physical ESP32 or LED was connected. The emulator advertises real mDNS/DNS-SD, serves HTTP, receives unicast UDP, and executes the same C++ receiver header compiled into the firmware.

## Observed checks

| Check                                                            | Observed result                                                                                                                                                                                                                                                         |
| ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| TypeScript, lint, formatting, domain/store/components/mocked IPC | `npm run check`: 41 Vitest tests and 4 signing-tool tests passed.                                                                                                                                                                                                       |
| Rust                                                             | 30 tests passed, including protocol, migration, persistence, identity, reconnect, and native color processing. Rustfmt and Clippy with warnings denied passed.                                                                                                          |
| Browser                                                          | Four Playwright tests passed: room workflow, WebGL/orbit, overflow/keyboard/calibration, and automated WCAG AA checks. Discovery-dialog focus restoration is included. Browser tests do not establish native persistence or networking.                                 |
| Firmware receiver                                                | `npm run test:firmware` passed. It executes the firmware C++ receiver, including the shared Rust/C++ golden packet, malformed frames, wrong source/session, out-of-order/duplicate frames, sequence wrap, one-owner behavior, idempotent start, keepalives and timeout. |
| ESP32 compilation                                                | PlatformIO `espressif32@6.12.0`, `esp32dev`, Arduino 2.0.17 built successfully: 47,740 bytes RAM and 831,225 bytes flash. This is a cross-compile, not a flash or electrical test.                                                                                      |
| Desktop build                                                    | Signed macOS application bundle built successfully. The development signature was verified locally; the app was not notarized.                                                                                                                                          |

## Real network emulator probe

The `esp32_probe` example uses production Rust discovery, HTTP/session management and UDP output, plus a temporary real configuration file. The final run passed:

- Automatic discovery and Identify by full permanent identity.
- Exact final RGB bytes, repeated static colors, and no session replacement during a static image.
- Receiver reboot creates a new session and resumes requested output.
- A two-second outage marks the device offline and retains its logical room assignment; requested output resumes when it returns.
- A fresh desktop service loads the saved binding, begins stopped, and streams after a new request.
- Explicit stop and a stalled native color source return the emulator to off.

Final probe rate was **26.4 FPS** on this Mac, below the 30 FPS cap. Earlier runs measured 26.9 and 27.3 FPS. These include real scheduler/network timing; they are not LED latency or Wi-Fi radio measurements.

## Signed macOS UI checks

An isolated application identity (`com.iotensity.hardware-check`) kept test configuration separate from the existing IOTensity installation. Observed through the actual native app:

1. The physical plus button discovered the emulator and Identify reached its receiver.
2. Adding a physical light left the virtual light in a separate tray. The room was saved through native persistence.
3. Synthetic simulation produced changing RGB on the emulator and showed Streaming in the app.
4. Stopping the emulator changed the saved light to Offline without deleting it.
5. Restarting the same full identity with HTTP **8099 → 8100** and UDP **49600 → 49601** resumed streaming without a duplicate or a new binding.
6. A physical binding was moved to an existing logical light. Reading the actual saved document confirmed that light's original UUID, position and separate virtual output were preserved, with no IP, session, online state or RGB in the file.
7. Dragging a light in the native 3D scene changed X/Z and retained its height. Cmd+S saved. Cmd+Q with dirty changes displayed the close guard; Save Room completed the save and quit.
8. Reopening the app retained the binding and coordinates and began stopped. Start resumed output automatically.
9. A second client attempting to start during an active stream received HTTP 409; the original session continued.
10. The deterministic test-image source held RGB `[0, 0, 225]` at 75% brightness, kept the same session, and delivered **27.4 FPS** over two seconds. Screen/test-image smoothing controls were disabled as intended.

During verification the Mac changed Wi-Fi networks, which required restarting the emulator on its current local address. This was not treated as proof of live physical DHCP recovery. Injected Rust controller tests cover changed IP addresses, re-verification, and an old IP occupied by a different identity. Discovery also refreshes its browse every ten seconds and on interface changes to bound recovery after missed announcements.

## Remaining physical/platform checks

The user requested emulator verification because no board was connected. Flashing, actual eFuse readout across boards, GPIO wiring/polarity, PWM waveform, real LED Identify behavior, power-cycle/DHCP tests on a board, Wi-Fi jitter, and end-to-end optical latency remain hardware acceptance checks. IPv6 and multi-device RF capacity are outside this first prototype. Native Windows interactive behavior was not observed locally.

The existing screen-capture and mini-room work is included in this delivery. Its native processing and lifecycle tests pass; historical screen-capture observations remain in the separate screen-sync verification documents. This milestone's new native UI checks used simulation and the deterministic test image.

CI is configured for frontend/browser checks, macOS and Windows native checks, and the pinned ESP32 firmware build. Local results above are observed evidence; remote run status is reported separately on the pull request.
