# First milestone: virtual light instrument

## State boundaries

`AppStore` exposes immutable snapshots through `useSyncExternalStore`. `saved` is the last native-acknowledged configuration; `draft` is a separate copy of the first room. Temporary selection, mode, navigation and errors belong to the editor. Simulation colors/time live in `Simulation`, outside React state and outside the document.

Brightness/intensity controls update preview preferences immediately and debounce a save by 400 ms. Preference saves are queued through the same `commit` transaction as room saves. Each transaction starts with the latest acknowledged configuration; it applies only its intended change. Room edits freeze during Save Room, preventing an older acknowledgement from marking a newer edit saved. Native revision comparison also rejects stale writers.

## One interaction, end to end

1. `RoomEditor` calls `AppStore.updateLight`, `moveLight` or `addLight`. These replace the draft, preserving IDs and unrelated axes. `LightCards` and `RoomScene` render that same draft; one `selectedLightId` links them.
2. **Save Room** / keyboard shortcut calls `AppStore.saveRoom`. It captures the intended draft, freezes editing and enqueues a transaction in `commit`.
3. `commit` clones the latest committed document and replaces its room with the snapshot. `NativePersistence.save` validates the TypeScript contract and invokes `save_config` with the configuration and `expectedRevision`.
4. `src-tauri/src/lib.rs::save_config` calls `ConfigStore::save`. Under one mutex it validates the data, reads the existing file and compares revisions. It writes a same-directory `NamedTempFile`, flushes the file to disk, and atomically persists it over `configuration.json`.
5. Only after acknowledgement does `commit` update `saved`. Sync now sees the new saved lights. If any step fails, saved remains unchanged, the draft remains dirty and the editor offers retry.
6. On the next launch, `load_config` → `ConfigStore::load` validates the stored document. `AppStore.load` restores saved and a fresh draft, clears selection, and leaves simulation stopped. Card name and appearance come directly from each persisted light.

## Coordinates, gradients and response

The floor below the monitor's centre is `(0, 0, 0)`. One unit is one metre. X = right, Y = height above floor, positive Z = into the room. Supported ranges are X `[-3, 3]`, Y `[0.15, 3]`, Z `[-0.7, 4]`. Every interactive input calls `moveLight` and the same clamp. Camera transforms never enter the document. Location dragging uses a horizontal plane at the light's Y; Height dragging uses a vertical camera-facing plane and changes Y only.

Location calibration linearly interpolates sRGB green `(0.25, 0.91, 0.53)` → orange `(1, 0.35, 0.12)` by X only. Height interpolates cyan `(0.24, 0.78, 1)` → violet `(0.78, 0.43, 1)` by Y. A single `resolveColor` provides the source for both orbs and card accents. Cards mix a small portion into a light neutral background for legible names at every brightness; selection has an outline and checkmark.

Synthetic color targets are deterministic sine functions of elapsed simulation time and stable light ID. Response uses `alpha = 1 - exp(-dt / tau)` with Subtle `1.8 s`, Balanced `0.8 s`, Vivid `0.3 s`, Punch `0.08 s`. Brightness scales displayed RGB output, not persisted color. Stop cancels the single scheduled callback and holds the last raw RGB. Navigation leaves the service alive. Reduced-motion preferences slow target changes and use at least a 2.5-second response.

The scene uses simple meshes, ambient/directional illumination and inexpensive unlit orb halos. React Three Fiber reads animation output in its frame loop. A single row-level paint loop updates all card accents from those same colors; cards do not simulate independently. Furniture, SVG silhouettes and the application mark are bundled, with no network assets.

## Implementation references

The command boundary follows [Tauri's Rust command API](https://v2.tauri.app/develop/calling-rust/). The main window's event/close behavior follows [Tauri's window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/). Pointer capture and ray intersections follow [React Three Fiber events](https://r3f.docs.pmnd.rs/api/events). None of these boundaries is a screen-sampling contract or hardware interface.
