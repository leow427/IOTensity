#include <Arduino.h>
#include <ArduinoJson.h>
#include <ESPmDNS.h>
#include <Preferences.h>
#include <WebServer.h>
#include <WiFi.h>
#include <WiFiUdp.h>
#include <esp_system.h>
#include <esp_wifi.h>
#include <esp_efuse.h>
#include "stream_protocol.h"

using namespace iotensity;
constexpr uint16_t kHttpPort = 80;
constexpr uint16_t kUdpPort = 49600;
constexpr uint32_t kPwmHz = 20000;
constexpr uint32_t kServiceRetryMs = 5000;
const uint8_t kPins[] = {IOT_RED_PIN, IOT_GREEN_PIN, IOT_BLUE_PIN};
WebServer http(kHttpPort);
Preferences preferences;
SemaphoreHandle_t stream_lock;
iotensity::Stream stream;
String device_id, hostname, configured_ssid, configured_password;
bool connected = false;

class Guard {
 public:
  Guard() { xSemaphoreTake(stream_lock, portMAX_DELAY); }
  ~Guard() { xSemaphoreGive(stream_lock); }
};
String hex(const Token& token) {
  char text[33];
  for (size_t i = 0; i < token.size(); ++i) snprintf(text + 2 * i, 3, "%02x", token[i]);
  return String(text);
}
bool parse_token(const char* text, Token& token) {
  if (!text || strlen(text) != 32) return false;
  for (size_t i = 0; i < 16; ++i) {
    uint8_t value = 0;
    for (size_t j = 0; j < 2; ++j) {
      const char c = text[i * 2 + j];
      if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) return false;
      value = (value << 4) | (c <= '9' ? c - '0' : c - 'a' + 10);
    }
    token[i] = value;
  }
  return true;
}
// Bodies are read by ControlHandler, never by WebServer, which would heap-buffer any
// declared Content-Length and wait up to 5 s for it before a handler could reject it.
constexpr size_t kMaxBody = 512;
constexpr uint32_t kBodyWaitMs = 1000;
char request_body[kMaxBody + 1];
size_t request_length = 0;
bool request_valid = false;

void status() {
  StaticJsonDocument<512> json;
  json["deviceId"] = device_id;
  String short_id = String("IOT-") + device_id.substring(12); short_id.toUpperCase();
  json["shortId"] = short_id;
  json["model"] = "esp32-rgb"; json["protocol"] = 1;
  json["udpPort"] = kUdpPort; json["maxFps"] = 30;
  {
    Guard guard;
    stream.expire(millis());
    if (stream.active) json["sessionId"] = hex(stream.session); else json["sessionId"] = nullptr;
    json["acceptedFrames"] = stream.accepted;
  }
  String body; serializeJson(json, body);
  http.sendHeader("Cache-Control", "no-store");
  http.send(200, "application/json", body);
}
bool body(StaticJsonDocument<512>& json) {
  if (http.header("Origin").length() || http.header("Content-Type") != "application/json" ||
      !request_valid || deserializeJson(json, static_cast<const char*>(request_body), request_length) ||
      !json["deviceId"].is<const char*>() || json["deviceId"].as<String>() != device_id) {
    http.send(400, "application/json", "{\"error\":\"invalid request\"}"); return false;
  }
  return true;
}
void start_stream() {
  StaticJsonDocument<512> json; if (!body(json)) return;
  Token client, request, session;
  if (json["protocol"] != 1 || !parse_token(json["clientId"], client) || !parse_token(json["requestId"], request)) {
    http.send(400, "application/json", "{\"error\":\"invalid stream request\"}"); return;
  }
  esp_fill_random(session.data(), session.size());
  bool ok;
  {
    Guard guard;
    ok = stream.start(uint32_t(http.client().remoteIP()), client, request, session, millis());
  }
  if (!ok) { http.send(409, "application/json", "{\"error\":\"busy\"}"); return; }
  status();
}
void stop_stream() {
  StaticJsonDocument<512> json; if (!body(json)) return;
  Token session;
  if (!parse_token(json["sessionId"], session)) { http.send(400, "application/json", "{}"); return; }
  bool allowed;
  {
    Guard guard;
    stream.expire(millis());
    allowed = !stream.active || (stream.session == session && stream.owner_ip == uint32_t(http.client().remoteIP()));
    if (allowed) stream.stop();
  }
  if (!allowed) { http.send(409, "application/json", "{\"error\":\"busy\"}"); return; }
  status();
}
void identify() {
  StaticJsonDocument<512> json; if (!body(json)) return;
  { Guard guard; stream.identify(millis()); }
  status();
}

// Owns every method for which Arduino-ESP32 2.x WebServer reads a body. Its raw hook runs
// after the headers: an oversized or negative Content-Length is refused unread, and an
// accepted body has a total deadline instead of WebServer's per-byte waits.
class ControlHandler : public RequestHandler {
 public:
  bool canHandle(HTTPMethod method, String) override {
    return method == HTTP_POST || method == HTTP_PUT || method == HTTP_PATCH || method == HTTP_DELETE;
  }
  bool canRaw(String) override { return true; }
  void raw(WebServer& server, String, HTTPRaw& state) override {
    if (state.status != RAW_START) return;
    request_length = 0; request_valid = false;
    const int length = server.clientContentLength();
    if (length >= 0 && size_t(length) <= kMaxBody) {
      WiFiClient client = server.client();
      const uint32_t started = millis();
      while (request_length < size_t(length) && millis() - started < kBodyWaitMs) {
        const int read = client.available() > 0
            ? client.read(reinterpret_cast<uint8_t*>(request_body) + request_length, size_t(length) - request_length)
            : 0;
        if (read > 0) request_length += read;
        else if (!client.connected()) break;
        else delay(1);
      }
      request_valid = request_length == size_t(length);
    }
    request_body[request_length] = '\0';
    // WebServer keeps reading until totalSize reaches Content-Length; mark the body
    // consumed so it neither buffers nor waits for the remainder.
    state.totalSize = SIZE_MAX;
  }
  bool handle(WebServer& server, HTTPMethod method, String uri) override {
    if (method == HTTP_POST && uri == "/v1/identify") identify();
    else if (method == HTTP_POST && uri == "/v1/stream/start") start_stream();
    else if (method == HTTP_POST && uri == "/v1/stream/stop") stop_stream();
    else server.send(404, "application/json", "{}");
    request_valid = false;
    return true;
  }
};
ControlHandler control_handler;

// UDP/PWM has its own task, so a slow HTTP client or serial provisioning cannot
// block valid frames, timeout safety or the Identify animation.
void output_task(void*) {
  WiFiUDP udp;
  bool listening = false;
  Rgb previous = {255, 255, 255};
  for (;;) {
    const bool wifi = WiFi.status() == WL_CONNECTED;
    if (wifi && !listening) listening = udp.begin(kUdpPort) != 0;
    if (!wifi && listening) { udp.stop(); listening = false; }
    for (int count = 0; listening && count < 32; ++count) {
      const int size = udp.parsePacket(); if (!size) break;
      uint8_t bytes[kPacketSize + 1];
      const int received = udp.read(bytes, sizeof(bytes));
      const uint32_t ip = uint32_t(udp.remoteIP());
      udp.flush();
      if (size == int(kPacketSize) && received == int(kPacketSize)) {
        Guard guard; stream.receive(bytes, size, ip, millis());
      }
    }
    Rgb rgb;
    {
      Guard guard;
      rgb = stream.output(millis(), wifi);
    }
    if (rgb != previous) {
      for (uint8_t i = 0; i < 3; ++i) ledcWrite(i, IOT_COMMON_ANODE ? 255 - rgb[i] : rgb[i]);
      previous = rgb;
    }
    vTaskDelay(1);
  }
}

// Replace this development provisioning function later without changing identity,
// discovery, control or the realtime receiver. Never echo or compile credentials.
void provision_serial() {
  static String line;
  static bool overflow = false;
  for (int count = 0; Serial.available() && count < 512; ++count) {
    const char c = Serial.read();
    if (c == '\r') continue;
    if (c != '\n') {
      if (line.length() < 384 && !overflow) line += c; else overflow = true;
      continue;
    }
    StaticJsonDocument<512> json;
    bool valid = !overflow && !deserializeJson(json, line);
    line = ""; overflow = false;
    if (valid && json["reset"] == true) {
      preferences.clear();
      // Older firmware let the driver persist credentials; overwrite its flash copy too.
      esp_wifi_set_storage(WIFI_STORAGE_FLASH); WiFi.disconnect(true, true);
      Serial.println("Wi-Fi settings cleared. Restarting."); Serial.flush(); ESP.restart();
    }
    valid = valid && json["ssid"].is<const char*>() && json["password"].is<const char*>();
    const String ssid = json["ssid"] | "";
    const String password = json["password"] | "";
    if (!valid || ssid.length() < 1 || ssid.length() > 32 || password.length() > 63) {
      Serial.println("Invalid provisioning message."); continue;
    }
    StaticJsonDocument<384> settings;
    settings["ssid"] = ssid; settings["password"] = password;
    String encoded; serializeJson(settings, encoded);
    if (preferences.putString("config", encoded) != encoded.length()) {
      Serial.println("Could not save Wi-Fi settings. Retry provisioning."); continue;
    }
    Serial.println("Wi-Fi settings saved. Restarting."); Serial.flush(); ESP.restart();
  }
}
void setup() {
  Serial.begin(115200);
  stream_lock = xSemaphoreCreateMutex();
  if (!stream_lock) abort();
  uint8_t mac[6];
  if (esp_efuse_mac_get_default(mac) != ESP_OK) abort();
  char identity[19];
  snprintf(identity, sizeof(identity), "esp32-%02x%02x%02x%02x%02x%02x", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
  device_id = identity; hostname = String("iotensity-") + device_id.substring(6);
  Serial.println(device_id);
  for (uint8_t i = 0; i < 3; ++i) {
    ledcSetup(i, kPwmHz, 8); ledcAttachPin(kPins[i], i); ledcWrite(i, IOT_COMMON_ANODE ? 255 : 0);
  }
  if (xTaskCreate(output_task, "rgb-output", 4096, nullptr, 2, nullptr) != pdPASS) abort();
  const char* headers[] = {"Origin", "Content-Type"}; http.collectHeaders(headers, 2);
  http.on("/v1/info", HTTP_GET, status); http.on("/v1/status", HTTP_GET, status);
  http.addHandler(&control_handler);
  http.onNotFound([] { http.send(404, "application/json", "{}"); });
  if (!preferences.begin("wifi", false)) { Serial.println("Cannot open Wi-Fi settings storage."); abort(); }
  StaticJsonDocument<384> settings;
  const String stored = preferences.getString("config");
  if (stored.length() && !deserializeJson(settings, stored)) {
    configured_ssid = settings["ssid"] | "";
    configured_password = settings["password"] | "";
  }
  WiFi.persistent(false); // The `wifi` namespace is the only credential store.
  WiFi.mode(WIFI_STA); WiFi.setHostname(hostname.c_str()); WiFi.setAutoReconnect(true);
  WiFi.setSleep(false); // Powered prototype: avoid modem sleep latency, including discovery.
  if (configured_ssid.length()) WiFi.begin(configured_ssid.c_str(), configured_password.c_str());
  else Serial.println("Provision via serial JSON with ssid and password, then newline.");
}
void loop() {
  provision_serial();
  const bool wifi = WiFi.status() == WL_CONNECTED;
  static IPAddress advertised_ip;
  static uint32_t last_retry = 0;
  static uint32_t service_failed_at = 0;
  static bool service_failed = false;
  if (wifi && (!connected || WiFi.localIP() != advertised_ip) &&
      (!service_failed || uint32_t(millis() - service_failed_at) >= kServiceRetryMs)) {
    if (connected) { connected = false; http.stop(); MDNS.end(); Guard guard; stream.stop(); }
    advertised_ip = WiFi.localIP();
    http.begin();
    if (MDNS.begin(hostname.c_str())) {
      MDNS.addService("iotensity", "tcp", kHttpPort);
      MDNS.addServiceTxt("iotensity", "tcp", "id", device_id);
      MDNS.addServiceTxt("iotensity", "tcp", "model", "esp32-rgb");
      MDNS.addServiceTxt("iotensity", "tcp", "pv", "1");
      connected = true; service_failed = false;
      Serial.println("Local network services ready.");
    } else {
      http.stop(); MDNS.end(); service_failed = true; service_failed_at = millis();
      Serial.printf("mDNS start failed; retrying in %u ms.\n", unsigned(kServiceRetryMs));
    }
  }
  if (!wifi && connected) { connected = false; http.stop(); MDNS.end(); Guard guard; stream.stop(); }
  if (!wifi) service_failed = false; // A fresh Wi-Fi connection retries services immediately.
  if (!wifi && uint32_t(millis() - last_retry) >= 5000 && configured_ssid.length()) {
    last_retry = millis(); WiFi.reconnect();
  }
  if (connected) http.handleClient();
  delay(1);
}
