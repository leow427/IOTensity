#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .tooling
${CXX:-c++} -std=c++17 -Wall -Wextra -Werror ${CXXFLAGS:-} -Ifirmware/esp32/include firmware/esp32/test/protocol_test.cpp -o .tooling/firmware-protocol-test
.tooling/firmware-protocol-test tests/fixtures/udp-v1.hex
# Build the LAN emulator's receiver bridge the same way scripts/esp32-emulator.py does.
${CXX:-c++} -std=c++17 -Wall -Wextra -Werror ${CXXFLAGS:-} -shared -fPIC -Ifirmware/esp32/include firmware/esp32/test/emulator_bridge.cpp -o .tooling/firmware-emulator-bridge.so
