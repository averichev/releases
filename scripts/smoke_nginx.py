#!/usr/bin/env python3
"""Verify the shipped nginx configuration against a disposable real Rust server."""
import http.cookiejar
import json
import os
from pathlib import Path
import re
import secrets
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, HTTPCookieProcessor, build_opener

ROOT = Path(__file__).resolve().parents[1]


def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/release/veruna-releases").resolve(strict=True)
    backend_port, nginx_port = port(), port()
    backend, base = f"http://127.0.0.1:{backend_port}", f"http://127.0.0.1:{nginx_port}"
    name = "veruna-nginx-smoke-" + secrets.token_hex(5)
    process = None
    started = False
    with tempfile.TemporaryDirectory(prefix=".veruna-nginx-", dir=ROOT) as directory:
        root = Path(directory)
        env = {k: v for k, v in os.environ.items() if k != "CONFIG" and not k.startswith(("VERUNA_", "SERVER__", "DATABASE__", "STORAGE__", "LOGGING__", "AUTH__", "DOWNLOAD__"))}
        env.update({"SERVER__BIND": f"0.0.0.0:{backend_port}", "AUTH__PUBLIC_ORIGIN": backend,
                    "DOWNLOAD__PUBLIC_BASE_URL": base, "DOWNLOAD__DELIVERY_MODE": "nginx",
                    "DATABASE__PATH": str(root / "db.sqlite"), "STORAGE__DIRECTORY": str(root / "objects"),
                    "STORAGE__ICONS_DIRECTORY": str(root / "icons")})
        password = secrets.token_urlsafe(24)
        boot = subprocess.run([str(binary), "admin", "bootstrap", "--stdin"], input=f"owner\n{password}\n", text=True, capture_output=True, cwd=root, env=env, timeout=30)
        assert boot.returncode == 0, boot.stderr
        browser = build_opener(HTTPCookieProcessor(http.cookiejar.CookieJar()))
        public = build_opener()
        csrf = None

        def request(url, method="GET", data=None, headers=None, authenticated=False):
            values = dict(headers or {})
            if data is not None:
                values.update({"Origin": backend, "X-Veruna-Request": "1"})
                if csrf:
                    values["X-CSRF-Token"] = csrf
                if not isinstance(data, bytes):
                    values["Content-Type"] = "application/json"
                    data = json.dumps(data).encode()
            try:
                response = (browser if authenticated else public).open(Request(url, data=data, method=method, headers=values), timeout=15)
            except HTTPError as error:
                response = error
            with response:
                return response.status, response.headers, response.read()

        def api(path, method="GET", data=None, status=200):
            actual, _, body = request(backend + "/api/v1" + path, method, data, authenticated=True)
            assert actual == status, (path, actual, body)
            return json.loads(body) if body else None

        try:
            with (root / "server.log").open("w") as log:
                process = subprocess.Popen([str(binary)], cwd=root, env=env, stdout=log, stderr=log)
            for _ in range(100):
                try:
                    if request(backend + "/api/v1/health")[0] == 200:
                        break
                except URLError:
                    time.sleep(0.1)
            else:
                raise AssertionError("Rust did not become ready")
            session = api("/auth/login", "POST", {"username": "owner", "password": password})
            csrf = session["csrf_token"]
            app = api("/admin/applications", "POST", {"name": "Smoke", "slug": "nginx-smoke", "visibility": "public"}, 201)
            app_path = "/admin/applications/" + app["id"]
            release = api(app_path + "/releases", "POST", {"version": "1.0.0", "release_notes": "Nginx smoke"}, 201)
            release_path = app_path + "/releases/" + release["id"]
            boundary = "VerunaBoundary" + secrets.token_hex(8)
            payload = b"0123456789abcdefghijklmnopqrstuvwxyz"
            multipart = (f'--{boundary}\r\nContent-Disposition: form-data; name="target"\r\n\r\nwindows-x86_64\r\n--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="setup.exe"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode() + payload + f'\r\n--{boundary}--\r\n'.encode())
            status, _, body = request(backend + "/api/v1" + release_path + "/artifacts", "POST", multipart, {"Content-Type": f"multipart/form-data; boundary={boundary}"}, True)
            assert status == 201, (status, body)
            artifact = json.loads(body)
            check = api(release_path + "/validate-publication", "POST", {})
            api(release_path + "/publish", "POST", {"expected_release_revision": check["release_revision"]})
            config = (ROOT / "deploy/nginx/veruna.conf").read_text()
            config = re.sub(r"listen 443 ssl;", "listen 8080;", config)
            config = re.sub(r"^\s*ssl_certificate(?:_key)? .*;$", "", config, flags=re.M)
            config = config.replace("/var/log/nginx/veruna-access.log", "/dev/stdout").replace("error_log /dev/null;", "error_log stderr notice;")
            config = config.replace("http://127.0.0.1:8080", f"http://host.docker.internal:{backend_port}")
            config = config.replace("/var/lib/veruna-releases/artifacts/", "/objects/")
            (root / "nginx.conf").write_text("events {}\nhttp {\n" + config + "\n}\n")
            subprocess.run(["docker", "run", "-d", "--name", name, "--add-host=host.docker.internal:host-gateway", "-p", f"127.0.0.1:{nginx_port}:8080", "-v", f"{root / 'nginx.conf'}:/etc/nginx/nginx.conf:ro", "-v", f"{root / 'objects'}:/objects:ro", "nginx:stable-alpine"], check=True, capture_output=True, timeout=30)
            started = True
            for _ in range(100):
                try:
                    if request(base + "/api/v1/health")[0] == 200:
                        break
                    time.sleep(0.1)
                except (URLError, ConnectionError):
                    time.sleep(0.1)
            else:
                raise AssertionError("Nginx did not become ready: " + subprocess.run(["docker", "logs", name], capture_output=True,text=True).stderr[-3000:])
            path = f"/nginx-smoke/releases/1.0.0/artifacts/{artifact['id']}/download"
            url = base + path
            status, headers, body = request(url)
            assert status == 200 and body == payload, (status, headers, body)
            assert "X-Accel-Redirect" not in headers and int(headers["Content-Length"]) == len(payload)
            assert "attachment" in headers["Content-Disposition"]
            etag = headers["ETag"]
            head_status, head_headers, head_body = request(url, "HEAD")
            assert head_status == 200 and head_body == b"" and head_headers["ETag"] == etag
            assert headers["Cache-Control"] == "private, no-cache, must-revalidate"
            assert request(url, headers={"Range": "bytes=0-3", "If-Range": headers["Last-Modified"]})[0] == 206
            for value, expected in [("bytes=0-3", payload[:4]), ("bytes=-4", payload[-4:]), ("bytes=35-", payload[-1:]), ("bytes=0-999", payload)]:
                status, h, body = request(url, headers={"Range": value, "If-Range": etag})
                assert status == 206 and body == expected and int(h["Content-Length"]) == len(expected), (status, h, body)
            assert request(url, headers={"Range": "bytes=999-"})[0] == 416
            assert request(url, headers={"Range": "bytes=0-2", "If-Range": '"old"'})[2] == payload
            assert request(url, headers={"Range": "bytes=0-2,4-5"})[0] == 200
            assert request(url, headers={"If-None-Match": etag})[0] == 304
            assert api(app_path)["download_count"] == 6
            assert api(release_path)["download_count"] == 6
            assert api(release_path + "/artifacts")[0]["download_count"] == 6
            assert request(base + "/_veruna_files/00000000000000000000000000000000.blob")[0] == 404
            assert request(base + "/_veruna_files/../db.sqlite")[0] == 404
            api(app_path, "PATCH", {"visibility": "private"})
            assert request(url, headers={"If-None-Match": etag})[0] == 404
            assert api(app_path)["download_count"] == 6
            print("Nginx smoke passed: real X-Accel file delivery, GET/HEAD, ETag/304, Range/206/416/If-Range, multiple ranges, internal location, visibility revocation, production config.")
        finally:
            if started:
                subprocess.run(["docker", "rm", "-f", name], capture_output=True, timeout=15)
            if process:
                process.terminate()
                process.wait(timeout=20)


if __name__ == "__main__":
    main()
