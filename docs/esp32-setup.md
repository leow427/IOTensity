# ESP32 RGB prototype

The initial target is an ESP32 DevKit / ESP32-WROOM board and one ordinary three-channel RGB LED. The ESP32 needs **2.4 GHz Wi-Fi**; the computer must be on the same local network and can use another Wi-Fi band or Ethernet. The pinned PlatformIO environment uses Arduino-ESP32 2.0.17. One firmware image works on every board: identity comes from the factory eFuse MAC, never a build-time serial number.

## Wiring

Default PWM pins are red **GPIO 25**, green **GPIO 26**, blue **GPIO 27**. Use a separate current-limiting resistor for each LED channel (330 Ω to 1 kΩ is a suitable low-current starting range; confirm the LED's forward-voltage/current specifications). With a common-cathode LED, connect the common lead to GND. The firmware defaults to common cathode.

For a common-anode LED, use a 3.3 V common connection and set `IOT_COMMON_ANODE=1` in `firmware/esp32/platformio.ini`. The firmware then inverts PWM duty. Adjust the three pin defines to match the actual board and wiring. Do not connect a high-power light or LED strip directly to the GPIOs; this prototype targets one low-current RGB LED. PWM is 20 kHz at 8-bit resolution, independent of the network's maximum 30 FPS.

## Build and flash

Run from the repository root with Python 3.9+ and a C++ compiler (the compiler is for host tests/emulation, not flashing):

```sh
python3 -m venv .tooling/firmware-env
.tooling/firmware-env/bin/python -m pip install platformio==6.1.18
.tooling/firmware-env/bin/pio run --project-dir firmware/esp32
.tooling/firmware-env/bin/pio device list
.tooling/firmware-env/bin/pio run --project-dir firmware/esp32 --target upload --upload-port /dev/cu.usbserial-REPLACE
.tooling/firmware-env/bin/pio device monitor --port /dev/cu.usbserial-REPLACE --baud 115200
```

On Windows use the virtual environment's `Scripts` directory and the appropriate `COM` port. Some DevKits need the BOOT button held while connecting. Firmware products are ignored under `firmware/esp32/.pio`; no credentials belong in source files. CI compiles the same pinned environment and uploads the build artifacts.

## Development provisioning

At 115200 baud send **one JSON line** with the Wi-Fi SSID and password, followed by a newline:

```json
{ "ssid": "YOUR_2_4_GHZ_NETWORK", "password": "YOUR_WIFI_PASSWORD" }
```

Enter this in the serial monitor, not a shell command/history or repository file. The ESP32 stores it in its NVS `wifi` namespace and restarts. It does not echo credentials. Sending `{"reset":true}` erases those Wi-Fi settings and restarts; it does not change hardware identity. Credentials are development provisioning data, not encrypted user accounts. A future provisioning flow can replace `provision_serial()` without changing identity, discovery, or the receiver.

The boot log prints `esp32-<12 lowercase hex digits>` and a services-ready message. The short recognition name is `IOT-<last six hex digits in uppercase>`. Short names can collide; the app always binds the full ID.

## Desktop workflow

1. Run the native IOTensity app from `~/Applications/IOTensity.app`; ordinary local release builds update that copy automatically. Allow local network access if macOS requests it. The browser editor does not access physical devices.
2. Open **Your Rooms → + Physical light**. An online device should appear automatically. Both machines must be on the same multicast-capable LAN; guest-network/AP client isolation and some VPNs block discovery.
3. Choose **Identify**. The LED blinks white/off three times over about 900 ms, then resumes the current stream or off state.
4. Choose **Add light** to create a new logical room light. Alternatively, select an existing virtual room light, choose **Bind physical light**, and select the ESP32. Its logical light ID and room position are retained. Selecting or moving it lights the physical LED in the orb's placement color: green–orange for Location, cyan–violet for Height. This works before saving and at zero Sync brightness. Two seconds after the last edit, it resumes Sync or turns off; leaving the editor also clears it.
5. Press **Save Room**. Only acknowledged bindings participate in Sync. The virtual and physical trays remain separate. **Use virtual output** removes a binding in the draft; save to apply it.
6. In **Sync**, choose **Animated colors** and Start. The physical LED and its preview receive the same final RGB values. For a static test choose **Test image**; screen capture remains macOS-only.
7. Stop returns the LED to off. After app restart the binding is restored but Sync is intentionally stopped. Start again to stream. An ESP32 restart, brief outage, or DHCP change while Sync remains requested reconnects automatically.

An offline light is never deleted from the room. The app resolves its current address by full ID and retries control with bounded backoff. It never persists a remembered IP. Allow a few seconds for network/discovery recovery. Only one computer may stream to a device at once; a second is reported busy until the first stops or its one-second stream lease expires.

If the list remains empty, confirm both devices are on the same LAN and that IOTensity is allowed under macOS Privacy & Security → Local Network, then choose **Retry discovery**. The app also recreates stalled discovery sockets automatically. Avoid launching older build/worktree copies with the same application identity.

## Emulator and repeatable verification

The emulator uses real mDNS, HTTP and UDP, and compiles **the exact firmware receiver header** into a small host library. It verifies protocol behavior without pretending to test electrical wiring, PWM waveform, RF conditions, or a physical DHCP server.

```sh
.tooling/firmware-env/bin/python -m pip install -r scripts/emulator-requirements.txt
# Replace the address with this computer's private LAN IPv4 address:
.tooling/firmware-env/bin/python scripts/esp32-emulator.py --bind 192.168.1.100 --test-api
```

Keep it running in one terminal. The app discovers `IOT-A1B2C3` / `esp32-020000a1b2c3`. Identify is logged to the terminal. The optional `--test-api` is only an emulator testing aid and does not exist in firmware. The default HTTP port is 8099, UDP is 49600; `--port` and `--udp-port` can change them. The firmware uses HTTP 80 and UDP 49600.

In another terminal:

```sh
npm run test:firmware
cargo run --manifest-path src-tauri/Cargo.toml --locked --example esp32_probe -- http://192.168.1.100:8099
```

The probe uses a temporary configuration file and production native discovery, control, and UDP code. It checks Identify, exact RGB, the packet rate, static keepalives, receiver reboot, offline retention/recovery, service restart with persisted binding, explicit stop, and source timeout. Run it while the native app is **not streaming** to the emulator. To test changed endpoints manually, stop the emulator and restart it with the same ID and another `--port`; DHCP/address changes are also covered by injected controller tests.

Desktop setup, architecture and the wire contract are in [AGENTS.md](../AGENTS.md). Arduino 3.x LEDC APIs differ; build against the pinned 2.0.17 environment. Emulator results do not replace physical flashing, wiring/polarity, PWM, power-cycle, DHCP and latency checks on a board.
