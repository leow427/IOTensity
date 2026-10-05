#include "wifi_credentials.h"
#include <cassert>
#include <iostream>
#include <string>
using namespace iotensity;

bool password(const std::string& text) { return valid_wifi_password(text.data(), text.size()); }
int main() {
  assert(!valid_wifi_ssid(0));
  assert(valid_wifi_ssid(1) && valid_wifi_ssid(32));
  assert(!valid_wifi_ssid(33));
  assert(password("")); // Open network.
  assert(!valid_wifi_password(nullptr, 8));
  for (size_t length = 1; length < 8; ++length) assert(!password(std::string(length, 'a')));
  assert(password("12345678"));
  assert(password(" !~Pass word~! "));
  assert(password(std::string(63, 'x')));
  assert(!password(std::string(65, 'a')));
  assert(password(std::string(32, 'a') + std::string(16, 'F') + std::string(16, '9'))); // Raw PSK.
  assert(!password(std::string(63, 'a') + "g")); // 64 characters must all be hex.
  assert(!password(std::string(63, 'a') + " "));
  assert(!password("pass\tword")); // Control characters.
  assert(!password("pass\x7fword"));
  assert(!password("caf\xc3\xa9-password")); // Non-ASCII UTF-8.
  assert(!password(std::string("pass\0word", 9))); // Embedded NUL.
  std::cout << "Firmware provisioning: SSID length, open, passphrase and raw PSK password rules passed\n";
}
