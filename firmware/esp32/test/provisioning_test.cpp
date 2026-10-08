#include "wifi_credentials.h"
#include <cstdlib>
#include <iostream>
#include <string>
using namespace iotensity;

// Unlike assert(), always evaluates its argument exactly once, even with -DNDEBUG.
#define CHECK(expr) check((expr), #expr, __FILE__, __LINE__)
void check(bool ok, const char* expr, const char* file, int line) {
  if (ok) return;
  std::cerr << file << ':' << line << ": check failed: " << expr << '\n';
  std::exit(1);
}

bool password(const std::string& text) { return valid_wifi_password(text.data(), text.size()); }
int main() {
  CHECK(!valid_wifi_ssid(0));
  CHECK(valid_wifi_ssid(1) && valid_wifi_ssid(32));
  CHECK(!valid_wifi_ssid(33));
  CHECK(password("")); // Open network.
  CHECK(!valid_wifi_password(nullptr, 8));
  for (size_t length = 1; length < 8; ++length) CHECK(!password(std::string(length, 'a')));
  CHECK(password("12345678"));
  CHECK(password(" !~Pass word~! "));
  CHECK(password(std::string(63, 'x')));
  CHECK(!password(std::string(65, 'a')));
  CHECK(password(std::string(32, 'a') + std::string(16, 'F') + std::string(16, '9'))); // Raw PSK.
  CHECK(!password(std::string(63, 'a') + "g")); // 64 characters must all be hex.
  CHECK(!password(std::string(63, 'a') + " "));
  CHECK(!password("pass\tword")); // Control characters.
  CHECK(!password("pass\x7fword"));
  CHECK(!password("caf\xc3\xa9-password")); // Non-ASCII UTF-8.
  CHECK(!password(std::string("pass\0word", 9))); // Embedded NUL.
  std::cout << "Firmware provisioning: SSID length, open, passphrase and raw PSK password rules passed\n";
}
