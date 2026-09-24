# Native physical light output

## Identity and persisted configuration

`VirtualLight.id` remains the logical room object ID. Schema 2 adds a required output union:

```typescript
type LightOutput = { kind: 'virtual' } | { kind: 'esp32'; deviceId: string };
```

The factory identity is `esp32-` plus all six eFuse factory MAC bytes, in conventional byte order as 12 lowercase hex digits. `IOT-A1B2C3` is display-only. Rust and TypeScript reject malformed full IDs, duplicate hardware bindings (including across rooms), unknown output fields, and transient fields. At most 64 physical bindings are supported per configuration. The shared v2 fixture contains both output variants; the v1 fixture preserves the old strict shape.

Legacy schema 1 is validated by a strict migration to explicit virtual outputs. Logical IDs, names, appearance, positions, preferences and revision are preserved. Loading migrates in memory without rewriting the file. A subsequent successful room/preference transaction writes v2 through the existing revision check, synced temporary file, and atomic replacement. Invalid/unknown versions are never reset. There is no downgrade migration.

Only user configuration and full bindings reach disk. IPs, ports, discovery state, online state, stream IDs, client IDs, request IDs, sequence counters, current RGB, timers and running state do not.

## Discovery and control

The service is `_iotensity._tcp.local.`. DNS-SD SRV provides the HTTP control port; TXT contains `id`, `model=esp32-rgb`, and `pv=1`. The hostname includes the full ID. `mdns-sd` continually browses, resolves changes, checks host interfaces, and verifies unreachable advertisements. Browsing refreshes every ten seconds and on interface changes so missed announcements cannot leave discovery in an hour-long query backoff. Discovered instances are coalesced by full hardware ID. A saved binding creates an offline entry even before any announcement arrives.

This milestone supports private/link-local **IPv4** LAN addresses. Global, loopback, multicast and unspecified addresses are not accepted from discovery. There is no frontend URL/network API, proxy use, redirect following, or cloud endpoint. Before controlling an address, the HTTP response must match the advertised full identity, model, protocol and capabilities; a DHCP address reused by a different light is rejected.

HTTP JSON endpoints (no CORS):

| Method/path                  | Request                                           | Response                                                                                                  |
| ---------------------------- | ------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| GET `/v1/info`, `/v1/status` | None                                              | `deviceId`, `shortId`, `model`, `protocol`, `udpPort`, `maxFps=30`, `sessionId` or null, `acceptedFrames` |
| POST `/v1/identify`          | `deviceId`                                        | Status; 900 ms blink overrides PWM temporarily                                                            |
| POST `/v1/stream/start`      | `deviceId`, `protocol=1`, `clientId`, `requestId` | Status including a new random 128-bit `sessionId`                                                         |
| POST `/v1/stream/stop`       | `deviceId`, `sessionId`                           | Status; safe output is off                                                                                |

All tokens are 32 lowercase hex characters. A desktop instance generates a random client ID at launch. Each new stream operation generates a request ID. Retrying a lost start response with the same client/request returns the current session, so unreliable control responses cannot continuously replace it. A different request establishes a fresh session. Firmware generates sessions with `esp_fill_random` and binds them to the HTTP peer's IPv4 address. Another active client gets HTTP 409; it cannot silently steal the stream. Stop requires the current session and owner IP. Expired streams may be acquired by another client.

Control runs on one bounded worker per discovered device (the discovery table is capped at 128 entries). Each has 300 ms connect / 600 ms request deadlines and a maximum 8 KiB response. Status is checked every 500 ms when healthy; failures back off to at most four seconds. Device failures do not block another device or the UDP scheduler. Fresh endpoint changes interrupt the backoff and cause re-verification/session establishment. Identify errors appear in the room UI.

This is a **trusted-LAN development protocol**, not an authenticated pairing/security boundary. Identity checks avoid accidental IP reuse; they are not cryptographic device authentication. A hostile LAN participant can spoof services or call control APIs. Use a trusted LAN. Internet exposure, TLS, user accounts, cryptographic provisioning, DDP, and a hub are outside this milestone.

## UDP v1 wire format

Exactly **29 bytes**, network byte order:

| Offset | Bytes | Meaning                              |
| ------ | ----- | ------------------------------------ |
| 0      | 4     | ASCII `IOTL`                         |
| 4      | 1     | Version = 1                          |
| 5      | 1     | Reserved flags = 0                   |
| 6      | 16    | Verified HTTP stream session token   |
| 22     | 4     | Unsigned 32-bit sequence, big endian |
| 26     | 3     | Complete RGB8 value                  |

The verified session association identifies the target, so each frame need not repeat the hardware ID. Length, marker, version, flags, sender IP and current session must match. The first valid packet accepts any sequence. Thereafter, `distance = (next - previous) mod 2^32` must be in `1..2^31-1`. Duplicates, older packets, and the exactly-half-range ambiguity are rejected. Invalid traffic does not refresh the timeout. A new session resets the sequence boundary, making delayed packets from old sessions harmless.

There are no RGB ACKs, retransmits, deltas or application frame queues. The receiver drains a bounded batch of UDP packets, selects the newest valid value, and writes its PWM duties. After **1,000 ms without a valid new frame**, the stream expires and RGB becomes zero. The same unsigned subtraction handles `millis()` wrap. Repeated current colors use fresh sequence numbers and act as keepalives. Stop, reboot, and Wi-Fi loss also return to off.

## Color and scheduling ownership

`src-tauri/src/sync` owns synthetic simulation, deterministic image sampling, and native screen capture. The synthetic source uses injected elapsed time and retains the original response presets; reduced motion slows it. Screen/test-image output directly samples the latest analysis image without those response constants. Brightness/color processing occurs in the source exactly once, yielding final RGB8.

Each final native frame feeds both the passive `sync-output` preview event cache and `HardwareService.publish`. The output layer copies those bytes without brightness changes, gamma changes, interpolation or temporal smoothing. Editor calibration is explicitly a preview-only placement guide; Identify is the physical recognition action.

The native UDP worker samples the latest final colors at no more than 30 FPS. Slow ticks are dropped rather than caught up. There is no per-device React timer, network loop in a card, or per-frame React state update. Control negotiation cannot block UDP. Static colors continue to send. If the native source stops publishing for 250 ms, hardware output stops rather than replaying stale frames forever; firmware independently enforces its one-second watchdog. This latest-frame handoff can add up to one output interval; it does not queue older frames.

Binding edits remain in the draft until native save acknowledgement. Saved changes remove obsolete targets immediately, and late control results are discarded if their binding/run/endpoint request no longer matches. App restart restores configuration, rediscovers addresses, and starts **stopped**. Device restart/outage during requested Sync automatically creates a new session; explicit Stop suppresses reconnect streaming. Closing stops the sender; if a best-effort HTTP Stop cannot complete during shutdown, the firmware watchdog supplies the final safety bound.

The firmware's HTTP/serial loop is separate from its UDP/PWM FreeRTOS task. HTTP delays cannot hold the LED indefinitely. PWM is 20 kHz / 8-bit; Wi-Fi modem sleep is disabled on this powered prototype. Provisioning is isolated in one function so a future onboarding system can replace it without changing output logic.

## Tests and scope

Host C++ tests execute the firmware receiver and share a golden packet with Rust. Rust controller tests inject time and transport to cover session replacement, idempotent retries, IP reuse, DHCP changes, unavailable devices, saved/offline binding and explicit stop. Native persistence tests cover migration without writes, atomic upgraded saves, and strict boundaries. Frontend tests cover plus-button separation, stable IDs, binding/discard/failure, Identify, offline display and passive final-color caching. The LAN emulator/probe exercises real network I/O using production desktop code and the actual firmware receiver.

The first electrical target is one ESP32 and one RGB LED. The data and worker boundaries permit multiple distinct devices, but multi-device RF capacity, native Windows interactive behavior, IPv6, and physical latency/jitter/PWM quality require further hardware measurements. See [verification evidence](esp32-verification.md).
