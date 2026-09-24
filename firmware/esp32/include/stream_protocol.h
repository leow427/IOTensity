#pragma once
#include <array>
#include <cstddef>
#include <cstdint>

namespace iotensity {
constexpr uint32_t kTimeoutMs = 1000;
constexpr size_t kPacketSize = 29;
using Token = std::array<uint8_t, 16>;
using Rgb = std::array<uint8_t, 3>;
inline bool newer(uint32_t next, uint32_t previous) {
  const uint32_t distance = next - previous;
  return distance != 0 && distance < 0x80000000U;
}
struct Frame { Token session; uint32_t sequence; Rgb rgb; };
inline bool decode(const uint8_t* bytes, size_t size, Frame& frame) {
  if (size != kPacketSize || bytes[0] != 'I' || bytes[1] != 'O' ||
      bytes[2] != 'T' || bytes[3] != 'L' || bytes[4] != 1 || bytes[5] != 0) return false;
  for (size_t i = 0; i < 16; ++i) frame.session[i] = bytes[6 + i];
  frame.sequence = (uint32_t(bytes[22]) << 24) | (uint32_t(bytes[23]) << 16) |
                   (uint32_t(bytes[24]) << 8) | bytes[25];
  frame.rgb = {bytes[26], bytes[27], bytes[28]};
  return true;
}

// Network-independent receiver. Callers serialize access; all time is injected.
class Stream {
 public:
  bool active = false;
  bool has_sequence = false;
  Token session{};
  Rgb rgb{}; // Safe static state: off, also used on stop, timeout, reboot and Wi-Fi loss.
  uint32_t sequence = 0;
  uint32_t accepted = 0;
  uint32_t last_valid = 0;
  uint32_t owner_ip = 0;
  Token client{};
  Token request{};

  void expire(uint32_t now) {
    if (active && uint32_t(now - last_valid) >= kTimeoutMs) stop();
  }
  void stop() { active = false; has_sequence = false; rgb = {0, 0, 0}; }
  bool start(uint32_t ip, Token client_id, Token request_id, Token fresh_session, uint32_t now) {
    expire(now);
    if (active) {
      if (ip != owner_ip || client_id != client) return false;
      if (request_id == request) return true; // Lost HTTP response: return existing association.
    }
    owner_ip = ip; client = client_id; request = request_id; session = fresh_session;
    active = true; has_sequence = false; last_valid = now; rgb = {0, 0, 0};
    return true;
  }
  bool receive(const uint8_t* bytes, size_t size, uint32_t ip, uint32_t now) {
    expire(now);
    Frame frame{};
    if (!active || ip != owner_ip || !decode(bytes, size, frame) || frame.session != session ||
        (has_sequence && !newer(frame.sequence, sequence))) return false;
    sequence = frame.sequence; has_sequence = true; rgb = frame.rgb; last_valid = now; ++accepted;
    return true;
  }
};
} // namespace iotensity
