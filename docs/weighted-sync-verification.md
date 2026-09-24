# Weighted screen-sync verification

Verified locally on macOS for the weighted-sampling update. Earlier prototype results remain in [screen-sync-verification.md](screen-sync-verification.md); they are historical and are not evidence for this update.

## Implemented behavior

- Color weighting is applied to linear source pixels before area reduction. The sampler preserves both weighted color sums and their weights, then applies a smooth spatial falloff around each saved light position.
- Vivid pixels receive a bounded preference over neutral backgrounds. Equal-area red/white detail produces RGB8 `[255, 124, 124]`, compared with `[255, 188, 188]` for the previous unweighted average. Black, white, grayscale and uniform dim colors retain their colors.
- Analysis dimensions are derived from each frame's aspect ratio, bounded to 256 pixels on the longest side. Native capture sizing uses the selected display's oriented content dimensions and backing scale. The display status exposes the resulting sample size.
- Existing persistence, native output events, four smoothing presets, brightness and saved-position mapping remain in use.

## Automated checks observed

| Check                                                                                     | Result                                                              |
| ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| `npm run check`                                                                           | Passed: formatting, lint, types, 34 frontend tests, 4 tooling tests |
| `npm run test:e2e`                                                                        | 4 passed using project-local Chromium                               |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`                               | Passed                                                              |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings` | Passed                                                              |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked`                                | 22 passed: 13 processing/service, 9 persistence                     |
| `npm run tauri -- build`                                                                  | Frontend production build and signed macOS bundle passed            |
| `codesign --verify --deep --strict --verbose=2 …/IOTensity.app`                           | Valid on disk; satisfies its designated requirement                 |

Rust tests include actual BGRA buffers with row padding, fractional reduction, colorful details against white, spatial preference and continuity, single-pixel axes, corner normalization, dark scenes, bounded highlight influence, invalid buffers, and repeated frame-size changes through one processor. Aspect coverage includes 16:9, 16:10, 4:3, ultrawide, 32:9, portrait, square and smaller-than-analysis images. Existing elapsed-time smoothing and single brightness application tests pass.

The browser runner needed permission to bind its local server. Its default browser cache lacked the required Chromium, so verification used `PLAYWRIGHT_BROWSERS_PATH="$PWD/.tooling/playwright"`. Signature verification required access to the system trust store. No remote CI run was triggered or observed.

## Native app observations

The signed release bundle used for these checks is `src-tauri/target/release/bundle/macos/IOTensity.app`. Executable SHA-256:

```text
522e6b5a040dce30b52fe131ad32ac9c88a5a55e51ccd1b0cd5581129e7b0641
```

- The generated source reached Running; the saved lower-left light appeared blue and upper-right light green in both cards and orbs. Stop returned to Stopped and held output.
- Main-display capture reached Running without a new permission grant. Its status reported **256×166 sampling**, demonstrating a non-16:9 live source. Native cards and orbs displayed the captured desktop colors. The longer status wrapped within the control panel.
- In a follow-up check, the user opened `docs/screen-pattern.html` in Safari and authorized inspection. The same signed executable above captured the actual test page with the Main macOS display source. With the red/green/blue/white palette, Desk left appeared blue and Corner lamp green. Clicking Swap colors changed the page to cyan/magenta/yellow/black; Desk left changed to yellow and Corner lamp to magenta in both native orbs and cards. Capture remained Running at 256×166 throughout. The original palette and full-screen page were restored afterward, with sync and the mini room left active for the user. This completes the previously blocked controlled browser-pattern check.
- The mini room opened and closed while capture ran. Capture then stopped cleanly before editor checks.
- A temporary 3D Location drag changed one draft light from `(-1.70, 1.20, 1.00)` to `(-1.35, 1.20, 1.84)`, retaining height and marking the room dirty. Command-Q opened the Save/Discard/Stay guard. Discard closed the app without saving the temporary drag.
- Relaunch restored the two saved lights, 27% brightness and Punch intensity, with sync stopped and the overlay closed. The saved configuration's SHA-256 was identical before and after verification:

  ```text
  c49ff02dc7c0766d149542ecadf108290bdf1cbac59068b4fbe34e2605b2dca3
  ```

## Processing cost

A temporary `rustc -O` harness compared the original and updated processing code on the same deterministic BGRA buffers. Each result is the median of 12 calls, including reduction and one light sample, excluding buffer generation and capture. This is an isolated CPU measurement, not an end-to-end frame-rate guarantee.

| Source pixels | Previous uniform sampler | Weighted sampler |
| ------------- | ------------------------ | ---------------- |
| 1920×1080     | 2.21 ms                  | 4.69 ms          |
| 3440×1440     | 5.20 ms                  | 8.28 ms          |
| 3840×2160     | 7.49 ms                  | 12.23 ms         |

## Remaining coverage

- The controlled browser-pattern check establishes regional color matching and response to palette changes through real capture. The weighted-versus-unweighted comparison and dark/neutral edge cases are established by the deterministic pixel tests; no subjective comparison using movie or game footage was performed.
- Physical ultrawide/portrait monitors and the older macOS capture-size fallback were not available for interactive verification; aspect behavior is covered by deterministic pixel-buffer tests and native compilation.
- Stop and restart capture after display configuration changes. Live stream reconfiguration, HDR, black-bar detection, other capture platforms and sustained capture frame-rate measurements remain outside this change.
