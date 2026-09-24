#include "stream_protocol.h"
#include <algorithm>
// The LAN emulator executes the exact receiver compiled into the firmware.
extern "C" {
void* rgb_create() { return new iotensity::Stream(); }
void rgb_destroy(void* state) { delete static_cast<iotensity::Stream*>(state); }
void rgb_expire(void* state, uint32_t now) { static_cast<iotensity::Stream*>(state)->expire(now); }
int rgb_active(void* state) { return static_cast<iotensity::Stream*>(state)->active; }
uint32_t rgb_accepted(void* state) { return static_cast<iotensity::Stream*>(state)->accepted; }
void rgb_session(void* state, uint8_t* bytes) { const auto& s = static_cast<iotensity::Stream*>(state)->session; std::copy(s.begin(), s.end(), bytes); }
void rgb_color(void* state, uint8_t* bytes) { const auto& c = static_cast<iotensity::Stream*>(state)->rgb; std::copy(c.begin(), c.end(), bytes); }
void rgb_stop(void* state) { static_cast<iotensity::Stream*>(state)->stop(); }
int rgb_start(void* state, uint32_t ip, const uint8_t* client, const uint8_t* request, const uint8_t* session, uint32_t now) {
  iotensity::Token c{}, r{}, s{};
  std::copy(client, client + 16, c.begin()); std::copy(request, request + 16, r.begin()); std::copy(session, session + 16, s.begin());
  return static_cast<iotensity::Stream*>(state)->start(ip, c, r, s, now);
}
int rgb_receive(void* state, const uint8_t* bytes, size_t size, uint32_t ip, uint32_t now) {
  return static_cast<iotensity::Stream*>(state)->receive(bytes, size, ip, now);
}
}
