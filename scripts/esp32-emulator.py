#!/usr/bin/env python3
"""Local ESP32 emulator: actual mDNS + HTTP + UDP, using the firmware C++ receiver.

No credentials, app config, or captured images are needed. The optional test API
can reboot the emulated receiver or temporarily simulate an unavailable device.
"""
import argparse
import ctypes as ct
import json
import os
from pathlib import Path
import re
import secrets
import socket
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from zeroconf import ServiceInfo, Zeroconf

ROOT = Path(__file__).resolve().parents[1]
U8P = ct.POINTER(ct.c_uint8)


def receiver():
    output = ROOT / ".tooling" / "esp32-receiver.so"
    output.parent.mkdir(exist_ok=True)
    subprocess.run([os.environ.get("CXX", "c++"), "-std=c++17", "-shared", "-fPIC",
                    "-Ifirmware/esp32/include", "firmware/esp32/test/emulator_bridge.cpp",
                    "-o", str(output)], cwd=ROOT, check=True)
    lib = ct.CDLL(str(output))
    lib.rgb_create.restype = ct.c_void_p
    for name in ["rgb_destroy", "rgb_stop", "rgb_active", "rgb_accepted"]:
        getattr(lib, name).argtypes = [ct.c_void_p]
    lib.rgb_accepted.restype = ct.c_uint32
    lib.rgb_expire.argtypes = [ct.c_void_p, ct.c_uint32]
    lib.rgb_session.argtypes = [ct.c_void_p, U8P]
    lib.rgb_color.argtypes = [ct.c_void_p, U8P]
    lib.rgb_start.argtypes = [ct.c_void_p, ct.c_uint32, U8P, U8P, U8P, ct.c_uint32]
    lib.rgb_receive.argtypes = [ct.c_void_p, U8P, ct.c_size_t, ct.c_uint32, ct.c_uint32]
    return lib


def array(data):
    return (ct.c_uint8 * len(data))(*data)


def ip_number(ip):
    return int.from_bytes(socket.inet_aton(ip), "little")


def token(text):
    # Firmware parse_token(): exactly 32 lowercase hex characters.
    return bytes.fromhex(text) if isinstance(text, str) and re.fullmatch(r"[0-9a-f]{32}", text) else None


class Emulator:
    def __init__(self, args):
        self.args = args
        self.lib = receiver()
        self.stream = self.lib.rgb_create()
        self.lock = threading.RLock()
        self.start_time = time.monotonic()
        self.offline_until = 0
        self.identifies = 0
        self.owner = None
        self.arrivals = []
        self.udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.udp.bind((args.bind, args.udp_port))
        self.udp.settimeout(0.05)
        self.udp_port = self.udp.getsockname()[1]

    def now(self):
        return int((time.monotonic() - self.start_time) * 1000) & 0xffffffff

    def status(self):
        with self.lock:
            self.lib.rgb_expire(self.stream, self.now())
            token, rgb = array(bytes(16)), array(bytes(3))
            self.lib.rgb_session(self.stream, token)
            self.lib.rgb_color(self.stream, rgb)
            return {"deviceId": self.args.id, "shortId": "IOT-" + self.args.id[-6:].upper(),
                    "model": "esp32-rgb", "protocol": 1, "udpPort": self.udp_port, "maxFps": 30,
                    "sessionId": bytes(token).hex() if self.lib.rgb_active(self.stream) else None,
                    "rgb": list(rgb), "acceptedFrames": self.lib.rgb_accepted(self.stream),
                    "identifyCount": self.identifies}

    def receive(self):
        while True:
            try:
                data, peer = self.udp.recvfrom(2048)
                with self.lock:
                    if time.monotonic() < self.offline_until:
                        continue
                    if self.lib.rgb_receive(self.stream, array(data), len(data), ip_number(peer[0]), self.now()):
                        self.arrivals.append(time.monotonic())
                        self.arrivals = self.arrivals[-120:]
            except socket.timeout:
                with self.lock:
                    self.lib.rgb_expire(self.stream, self.now())

    def handler(self):
        emulator = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def respond(self, status=200, value=None):
                body = json.dumps(emulator.status() if value is None else value).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                if value is None:
                    self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def header(self, name):
                # Arduino WebServer keeps the last trimmed value of a collected header.
                return (self.headers.get_all(name) or [""])[-1].strip()

            def body(self, raw):
                # Mirrors firmware body(): every POST route rejects with the same 400.
                try:
                    value = json.loads(raw.decode()) if len(raw) <= 512 else None
                except ValueError:
                    value = None
                if (self.header("Origin") or self.header("Content-Type") != "application/json"
                        or not isinstance(value, dict) or value.get("deviceId") != emulator.args.id):
                    self.respond(400, {"error": "invalid request"}); return None
                return value

            def do_GET(self):
                if time.monotonic() < emulator.offline_until:
                    self.respond(503, {}); return
                if self.command != "GET" or self.path.split("?", 1)[0] not in ["/v1/info", "/v1/status"]:
                    self.respond(404, {}); return
                self.respond()

            do_HEAD = do_OPTIONS = do_GET  # Firmware routes only GET/POST; others are 404.

            def do_POST(self):
                path = self.path.split("?", 1)[0]
                try:
                    size = int(self.headers.get("Content-Length", "0"))
                except ValueError:
                    size = 0  # String::toInt() reads an invalid length as an empty body.
                if size < 0:
                    self.close_connection = True; return
                if size > 65536:  # Emulator bound; the firmware reads any body its heap allows.
                    self.respond(400, {"error": "invalid request"}); return
                try:
                    self.connection.settimeout(5)  # WebServer HTTP_MAX_POST_WAIT
                    raw = self.rfile.read(size)
                except OSError:
                    raw = b""
                if len(raw) < size:
                    # WebServer drops an incomplete body without responding.
                    self.close_connection = True; return
                if emulator.args.test_api and self.command == "POST" and path == "/_test/reboot":
                    body = self.body(raw)
                    if body is None:
                        return
                    try:
                        offline = min(float(body.get("offlineSeconds", 0)), 5)
                    except (TypeError, ValueError):
                        self.respond(400, {"error": "invalid request"}); return
                    with emulator.lock:
                        emulator.lib.rgb_stop(emulator.stream)
                        emulator.offline_until = time.monotonic() + offline
                    self.respond(); return
                if time.monotonic() < emulator.offline_until:
                    self.respond(503, {}); return
                if self.command != "POST" or path not in ["/v1/identify", "/v1/stream/start", "/v1/stream/stop"]:
                    self.respond(404, {}); return
                body = self.body(raw)
                if body is None:
                    return
                if path == "/v1/identify":
                    with emulator.lock:
                        emulator.identifies += 1
                    print("Identify: LED would blink for 900 ms", flush=True)
                elif path == "/v1/stream/start":
                    client, request = token(body.get("clientId")), token(body.get("requestId"))
                    # ArduinoJson, like Python, compares true and 1.0 equal to 1.
                    if body.get("protocol") != 1 or client is None or request is None:
                        self.respond(400, {"error": "invalid stream request"}); return
                    with emulator.lock:
                        ok = emulator.lib.rgb_start(emulator.stream, ip_number(self.client_address[0]), array(client), array(request), array(secrets.token_bytes(16)), emulator.now())
                        if ok:
                            emulator.owner = self.client_address[0]
                    if not ok:
                        self.respond(409, {"error": "busy"}); return
                else:
                    session = token(body.get("sessionId"))
                    if session is None:
                        self.respond(400, {}); return
                    with emulator.lock:
                        status = emulator.status()
                        allowed = not status["sessionId"] or (status["sessionId"] == session.hex() and emulator.owner == self.client_address[0])
                        if allowed:
                            emulator.lib.rgb_stop(emulator.stream)
                    if not allowed:
                        self.respond(409, {"error": "busy"}); return
                self.respond()

            do_PUT = do_PATCH = do_DELETE = do_POST  # Firmware reads their bodies, then 404.

        return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bind", required=True, help="This computer's private LAN IPv4 address")
    parser.add_argument("--port", type=int, default=8099)
    parser.add_argument("--udp-port", type=int, default=49600)
    parser.add_argument("--id", default="esp32-020000a1b2c3")
    parser.add_argument("--test-api", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch(r"esp32-[0-9a-f]{12}", args.id):
        parser.error("Expected full esp32-<12 lowercase hex> ID")
    emulator = Emulator(args)
    http = ThreadingHTTPServer((args.bind, args.port), emulator.handler())
    service_type = "_iotensity._tcp.local."
    info = ServiceInfo(service_type, "iotensity-" + args.id[6:] + "." + service_type,
                       addresses=[socket.inet_aton(args.bind)], port=http.server_port,
                       properties={"id": args.id, "model": "esp32-rgb", "pv": "1"},
                       server="iotensity-" + args.id[6:] + ".local.")
    mdns = Zeroconf(interfaces=[args.bind])
    mdns.register_service(info)
    threading.Thread(target=emulator.receive, daemon=True).start()
    print(json.dumps({"deviceId": args.id, "http": f"http://{args.bind}:{http.server_port}", "udpPort": emulator.udp_port}), flush=True)
    try:
        http.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        mdns.unregister_service(info)
        mdns.close()
        http.server_close()


if __name__ == "__main__":
    main()
