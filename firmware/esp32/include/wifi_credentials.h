#pragma once
#include <cstddef>

namespace iotensity {
// Station credentials accepted by serial provisioning. SSIDs are 1-32 bytes.
// Passwords are empty (open network), an 8-63 character printable ASCII WPA
// passphrase, or a 64 hex digit raw PSK.
inline bool valid_wifi_ssid(size_t length) { return length >= 1 && length <= 32; }
inline bool valid_wifi_password(const char* password, size_t length) {
  if (length == 0) return true;
  if (!password) return false;
  const bool hex = length == 64;
  if (!hex && (length < 8 || length > 63)) return false;
  for (size_t i = 0; i < length; ++i) {
    const char c = password[i];
    if (hex ? !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'))
            : (c < 0x20 || c > 0x7e)) return false;
  }
  return true;
}
} // namespace iotensity
