# Screen-sync prototype

The room editor, save queue and logical light IDs are preserved. Rust owns screen, test-image and synthetic color output. Schema 2 adds optional physical outputs through an explicit virtual/ESP32 binding. Source, capture state, network addresses, colors, sessions and overlay state remain volatile. See [ESP32 architecture](esp32-architecture.md).

## Use

Save your room, choose a source in Sync, and press Start Sync. Main macOS display uses ScreenCaptureKit and requires Screen Recording access. Only the main display is captured. The generated source works without that permission and uses real BGRA pixels: red/green above, blue/white below.

Open mini room creates a 360×310 always-on-top window. Drag its titlebar into a corner. It contains a simplified room with the saved light positions and exactly the final colors applied in the main UI. It is read-only and has no animation engine. Closing it does not stop sync. Closing the main app stops the service and closes the mini room. Relaunch always starts stopped with the overlay closed.

Stop waits for the capture lifecycle to stop before exposing a stopped state. The virtual preview holds its last colors after Stop; physical LEDs return to off. Start creates a new capture session. Source switching is available only while stopped or after an error. Preferences become effective after successful persistence acknowledgment (normally after the existing 400 ms debounce).

## Native pipeline

`ScreenCaptureKit / generated BGRA pixels → sRGB decode + color weights → one aspect-preserving reduction → position-weighted averages → brightness → sRGB RGB8 → preview events + physical output`

- `src-tauri/src/sync/capture.m` is a small macOS-only ScreenCaptureKit bridge. It captures the main display at backing pixel resolution in BGRA/sRGB/SDR, with cursor and audio capture disabled. On macOS 14+, dimensions come from the selected content filter's `contentRect × pointPixelScale`, so the stream follows the display's orientation and backing scale. On older macOS versions, backing dimensions are aligned with the selected display's orientation. The entire current application is excluded with `SCContentFilter`, including windows created after capture starts. No screenshots or video recordings are written.
- Capture callbacks only retain the latest complete pixel buffer, releasing its predecessor. There is no application frame queue. The framework's own capture queue is bounded to three surfaces.
- A Rust output worker wakes on a 33.333 ms target interval independently of capture callbacks. It consumes at most one latest buffer, reduces it once, and keeps the analysis image for static screens. The current final colors repeat even without a new capture callback, keeping physical streams alive. Late ticks use actual elapsed time and do not create catch-up frame work.
- Each source component is decoded using the sRGB transfer function before it contributes to any average. Reduction uses an area-weighted box filter, including fractional edge coverage. Both analysis dimensions come from each incoming frame, with a longest side of at most 256 pixels and the other rounded to the nearest pixel (minimum one). Smaller images are not enlarged. The running display status shows the actual analysis size.

  | Capture dimensions | Analysis dimensions |
  | ------------------ | ------------------- |
  | 1920×1080 (16:9)   | 256×144             |
  | 2560×1600 (16:10)  | 256×160             |
  | 1600×1200 (4:3)    | 256×192             |
  | 3440×1440          | 256×107             |
  | 5120×1440 (32:9)   | 256×72              |
  | 1080×1920 portrait | 144×256             |
  | 2048×2048 square   | 256×256             |

  The deterministic quadrant source remains 640×360 by design; it does not determine the display source's dimensions.

- Before reduction, each source pixel receives `colorWeight = 1 + 3 × (max(linearRGB) - min(linearRGB))`. Bright, saturated colors have up to four times the influence of neutral pixels, helping colorful details survive white/gray backgrounds. Using linear chroma also limits the influence of dim saturated noise. Black, gray and white retain a nonzero weight, so all-black scenes stay black and grayscale stays neutral. Both `linearRGB × colorWeight` and `colorWeight` are area-averaged into the small image; normalizing only when a light is sampled preserves color information through downscaling.
- Screen coordinates start at the top-left. With the same bounds used by native configuration validation:

  ```text
  u = clamp((x - (-3)) / (3 - (-3)), 0, 1)
  v = clamp(1 - (y - 0.15) / (3 - 0.15), 0, 1)
  rectangle = [u - 0.1, u + 0.1] × [v - 0.1, v + 0.1], clipped to image bounds
  ```

  X maps left-to-right; increasing Y maps upward. Z and camera transforms never enter this calculation. Within the rectangle, a separable tent gives the mapped position the greatest weight and falls smoothly to zero at the perimeter. The tent is integrated over each analysis pixel, including partial coverage, so small position changes do not abruptly add a whole pixel. Clipped edges are normalized by their actual total weight, including on a one-pixel axis.

  ```text
  spatialWeight = integratedTentX × integratedTentY
  output = sum(spatialWeight × weightedRGB) / sum(spatialWeight × colorWeight)
  ```

  There is no winning-color cutoff or saturation multiplier after averaging. Equal red and blue contributions still blend in linear light; tiny highlights get only a bounded preference. Brightness remains controlled by the existing preference.

- Media frames update directly, without the synthetic animation response constants. The physical transport adds no smoothing. Synthetic color simulation retains the Subtle (1.8 s), Balanced (800 ms), Vivid (300 ms) and Punch (80 ms) responses. Reduced motion slows that synthetic source; the intensity selector is disabled for screen and test-image sources.
- Brightness multiplies sampled linear RGB exactly once. Native output encodes and rounds to sRGB RGB8. Frontend cards, 3D orbs, mini-room circles and ESP32 datagrams consume those final values. The 3D light materials bypass tone mapping. Selected editor calibration remains a visual-only override, including at zero brightness.

## Ownership and transactions

`load_config` and a successful `save_config` update the native service's acknowledged configuration. Failed or stale writes do not update sampling. Older acknowledgments cannot replace a newer native revision. The existing `AppStore.commit` queue still constructs room and preference transactions from the latest acknowledged configuration. Preferences never serialize a room draft.

The frontend native-output client is a passive color cache. It listens before requesting its initial snapshot and rejects older sequence numbers so a delayed response cannot overwrite newer output. Events contain final RGB8 values and basic status, never captured pixels or analysis buffers. React subscriptions publish status changes; the existing scene frame loop and one card-row paint loop read colors without putting every native frame into React state.

Each webview has its own passive cache of the same native output events. The mini room receives configuration-save events, accepts only non-regressing revisions, and projects saved XYZ positions into a small SVG room. Its one paint loop reads the same native colors. It never invokes save, start or stop from its UI.

On macOS, `src-tauri/src/overlay.m` hosts the mini-room webview in an actual nonactivating `NSPanel`. The hidden Tauri window retains ownership; its close notification returns the view and closes the panel before Tauri destroys the webview. No Objective-C class swapping is used. The panel has its own titlebar drag handling, joins full-screen Spaces, and does not change the main application's activation policy or Dock presence. Windows continues to use the ordinary always-on-top Tauri window.

The capture bridge serializes lifecycle operations, cancels startup when Stop arrives, releases retained frames on Stop, and reports permission/capture errors. The native worker outlives page navigation. Quit routes through the existing frontend Save/Discard/Stay flow; forced termination is not guarded.

## Platform and limits

- macOS 12.3+; capture code and frameworks are platform-gated. Windows can use the generated native source; Windows capture is not implemented.
- ScreenCaptureKit handles permission checks directly. If macOS denies capture, the UI explains where to enable it and relaunch. Local macOS bundles now use a pinned signing certificate through the npm Tauri wrapper; missing certificates fail clearly instead of falling back to ad hoc signing. See [stable macOS signing](macos-signing.md). The initial transition from the previous ad hoc app needs a fresh grant. macOS may still require reauthorization, and changing certificate/channel can require another grant. Distribution notarization is not configured.
- SDR only. Protected/DRM content may be blank. No window capture, display picker, HDR handling, black-bar detection, custom mappings. Local ESP32 output is described separately.
- The display is selected at Start. Stop and restart after changing the primary display or resolution. Display disconnection, sleep/wake, rotation, arbitrary multi-monitor arrangements and exclusive full-screen games require further native coverage.
- The macOS overlay uses a nonactivating panel, the screen-saver window level, full-screen auxiliary / all-Spaces behavior, and (on macOS 13+) `canJoinAllApplications`. See the [native verification report](screen-sync-verification.md) for observed stacking results; flags alone do not establish full-screen compatibility. System windows and exclusive full-screen applications may outrank it. Position and open state are not persisted.
- The 30 Hz cadence is a target, not a real-time guarantee. Full-resolution SDR decoding uses CPU; high-resolution displays and system load can reduce the attained rate. No performance dashboard or advanced diagnostics is included.
- Reduced-motion CSS and keyboard/focus behavior remain. Reduced motion applies to synthetic animation; media follows the selected source without a temporal filter.

Behavior references: [Govee's desktop guide](https://desktop.govee.com/user-manual/user-guide) describes screen-area mapping, saturation and sensitivity; [Hue's gaming overview](https://www.philips-hue.com/en-us/entertainment/gaming) describes positioned lights, screen color matching, intensity and brightness. These are references for the experience, not specifications of either vendor's internal algorithm. The bounded color preference and spatial weighting above are IOTensity's own implementation.

API references: [Apple ScreenCaptureKit capture sample](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos), [contentRect](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/contentrect), [pointPixelScale](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/pointpixelscale), [Tauri native-to-frontend events](https://tauri.app/develop/calling-frontend/).
