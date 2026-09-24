#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .tooling
${CXX:-c++} -std=c++17 -Wall -Wextra -Werror -Ifirmware/esp32/include firmware/esp32/test/protocol_test.cpp -o .tooling/firmware-protocol-test
.tooling/firmware-protocol-test tests/fixtures/udp-v1.hex
