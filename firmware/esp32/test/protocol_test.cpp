#include "stream_protocol.h"
#include <cassert>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>
using namespace iotensity;

std::vector<uint8_t> packet(Token token, uint32_t sequence, Rgb rgb = {1, 128, 255}) {
  std::vector<uint8_t> bytes{'I', 'O', 'T', 'L', 1, 0};
  bytes.insert(bytes.end(), token.begin(), token.end());
  for (int shift : {24, 16, 8, 0}) bytes.push_back(sequence >> shift);
  bytes.insert(bytes.end(), rgb.begin(), rgb.end());
  return bytes;
}
int main(int argc, char** argv) {
  assert(argc == 2);
  Token session; session.fill(0x11);
  Token client; client.fill(0x22);
  Token request; request.fill(0x33);
  Token next; next.fill(0x44);
  auto bytes = packet(session, 0xfffffffe);
  std::ifstream fixture(argv[1]); std::string golden; fixture >> golden;
  std::string encoded;
  const char* digits = "0123456789abcdef";
  for (uint8_t b : bytes) { encoded += digits[b >> 4]; encoded += digits[b & 15]; }
  assert(encoded == golden);
  Stream stream;
  assert(!stream.receive(bytes.data(), bytes.size(), 7, 0));
  assert(stream.start(7, client, request, session, 0));
  assert(stream.receive(bytes.data(), bytes.size(), 7, 1));
  assert(stream.rgb == (Rgb{1, 128, 255}));
  assert(!stream.receive(bytes.data(), bytes.size(), 7, 2)); // duplicate
  assert(!stream.receive(bytes.data(), bytes.size(), 8, 2)); // wrong computer
  bytes = packet(session, 0xfffffffd);
  assert(!stream.receive(bytes.data(), bytes.size(), 7, 2)); // old sequence
  bytes = packet(session, 0);
  assert(stream.receive(bytes.data(), bytes.size(), 7, 3)); // wrap
  bytes = packet(session, 0x80000000);
  assert(!stream.receive(bytes.data(), bytes.size(), 7, 4)); // ambiguous half range
  bytes = packet(next, 1);
  assert(!stream.receive(bytes.data(), bytes.size(), 7, 4)); // old/foreign session
  assert(!stream.start(8, next, next, next, 5)); // one owner
  assert(stream.start(7, client, request, next, 5));
  assert(stream.session == session); // idempotent HTTP retry
  assert(stream.start(7, client, next, next, 6)); // new request = new session
  bytes = packet(session, 99);
  assert(!stream.receive(bytes.data(), bytes.size(), 7, 7));
  bytes = packet(next, 0);
  assert(stream.receive(bytes.data(), bytes.size(), 7, 10));
  for (size_t size = 0; size < kPacketSize; ++size) assert(!stream.receive(bytes.data(), size, 7, 20));
  bytes.push_back(0); assert(!stream.receive(bytes.data(), bytes.size(), 7, 20)); bytes.pop_back();
  for (int offset : {0, 4, 5}) { bytes[offset] ^= 1; assert(!stream.receive(bytes.data(), bytes.size(), 7, 20)); bytes[offset] ^= 1; }
  // Repeating a current color with fresh sequences is a keepalive.
  for (uint32_t i = 1; i < 40; ++i) { bytes = packet(next, i); assert(stream.receive(bytes.data(), bytes.size(), 7, 10 + i * 33)); }
  const uint32_t last = stream.last_valid;
  stream.expire(last + 999); assert(stream.active);
  stream.expire(last + 1000); assert(!stream.active && stream.rgb == (Rgb{0, 0, 0}));
  bytes = packet(next, 100); assert(!stream.receive(bytes.data(), bytes.size(), 7, last + 1001));
  assert(stream.start(8, next, next, session, 0xfffffff0));
  stream.expire(0x000003d7); assert(stream.active); // millis wrap: 999 ms
  stream.expire(0x000003d8); assert(!stream.active);
  assert(stream.start(7, client, request, next, 0)); stream.stop();
  assert(!stream.receive(bytes.data(), bytes.size(), 7, 1));
  // Exercise the exact color selector used by the PWM task, not just stream.rgb.
  stream.identify(10); // Discovery Identify works while normally stopped.
  assert(stream.output(10, true) == (Rgb{160, 160, 160}));
  assert(stream.output(160, true) == (Rgb{0, 0, 0}));
  assert(stream.output(310, true) == (Rgb{160, 160, 160}));
  assert(stream.output(910, true) == (Rgb{0, 0, 0}));
  assert(stream.start(7, client, request, session, 1000));
  bytes = packet(session, 0);
  assert(stream.receive(bytes.data(), bytes.size(), 7, 1001));
  stream.identify(1002);
  assert(stream.output(1002, true) == (Rgb{160, 160, 160}));
  stream.stop(); // Accepted HTTP Stop and main-loop Wi-Fi/IP changes use this.
  assert(stream.output(1003, true) == (Rgb{0, 0, 0}));
  assert(stream.output(1302, true) == (Rgb{0, 0, 0})); // No later blink resumes.
  stream.identify(2000);
  assert(stream.output(2000, false) == (Rgb{0, 0, 0})); // PWM task sees Wi-Fi loss.
  assert(stream.output(2300, true) == (Rgb{0, 0, 0})); // Reconnect stays off.
  assert(stream.start(7, client, request, session, 3000));
  stream.identify(3900);
  assert(stream.output(3999, true) == (Rgb{160, 160, 160}));
  stream.expire(4000); // HTTP status expiry must also cancel Identify.
  assert(stream.output(4200, true) == (Rgb{0, 0, 0}));
  std::cout << "Firmware protocol: golden packet, owner/session, ordering/wrap, keepalive, timeout and stop passed\n";
}
