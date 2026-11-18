#!/usr/bin/env python3
"""Check an actual Linux binary behind loopback nginx/TLS, without configured URLs."""
import json
from pathlib import Path
import secrets
import ssl
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]


def command(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs).stdout.strip()


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/x86_64-unknown-linux-musl/release/veruna-releases").resolve(strict=True)
    name = "veruna-origin-smoke-" + secrets.token_hex(5)
    started = False
    # The certificate is disposable; the test checks proxy TLS handling rather
    # than certificate issuance. Requests still use a real TLS connection.
    context = ssl._create_unverified_context()
    with tempfile.TemporaryDirectory(prefix="veruna-origin-") as directory:
        root = Path(directory)
        root.chmod(0o755)
        command("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                "-subj", "/CN=releases.example.test", "-keyout", str(root / "key.pem"),
                "-out", str(root / "cert.pem"))
        (root / "key.pem").chmod(0o644)
        (root / "nginx.conf").write_text('''pid /tmp/nginx.pid;
error_log stderr warn;
events {}
http {
    access_log off;
    client_body_temp_path /tmp/client;
    proxy_temp_path /tmp/proxy;
    fastcgi_temp_path /tmp/fastcgi;
    uwsgi_temp_path /tmp/uwsgi;
    scgi_temp_path /tmp/scgi;
    server {
        listen 8443 ssl;
        server_name releases.example.test another.example.test;
        ssl_certificate /smoke/cert.pem;
        ssl_certificate_key /smoke/key.pem;
        location / {
            proxy_pass http://127.0.0.1:3001;
            proxy_set_header Host $http_host;
            proxy_set_header X-Forwarded-Proto $scheme;
            proxy_set_header Forwarded "";
        }
    }
}
''')
        try:
            command("docker", "run", "--rm", "--detach", "--name", name,
                    "--platform", "linux/amd64", "--user", "65532:65532", "--workdir", "/tmp",
                    "--publish", "127.0.0.1::8443",
                    "--mount", f"type=bind,source={binary},target=/veruna-releases,readonly",
                    "--mount", f"type=bind,source={root},target=/smoke,readonly",
                    "--env", "SERVER__BIND=127.0.0.1:3001",
                    "--env", "SERVER__AUTO_PORT=false", "--env", "SERVER__OPEN_BROWSER=false",
                    "nginx:stable-alpine", "sh", "-c",
                    "/veruna-releases >/tmp/server.log 2>&1 & exec nginx -c /smoke/nginx.conf -g 'daemon off;'")
            started = True
            port = command("docker", "port", name, "8443/tcp").rsplit(":", 1)[1]
            base = f"https://127.0.0.1:{port}"

            def request(host, path, payload=None, cookie=None, csrf=None, origin=None, expected=200):
                headers = {"Host": host, "X-Forwarded-Proto": "http", "X-Forwarded-Host": "attacker.example"}
                if cookie:
                    headers["Cookie"] = cookie
                data = None
                if payload is not None:
                    headers.update({"Origin": origin or f"https://{host}", "X-Veruna-Request": "1", "Content-Type": "application/json"})
                    if csrf:
                        headers["X-CSRF-Token"] = csrf
                    data = json.dumps(payload).encode()
                try:
                    response = urlopen(Request(base + path, data=data, headers=headers), context=context, timeout=5)
                except HTTPError as error:
                    response = error
                with response:
                    body = response.read()
                    assert response.status == expected, (path, response.status, body)
                    return (json.loads(body) if "application/json" in response.headers.get("Content-Type", "") else body), response.headers

            for _ in range(100):
                try:
                    request("releases.example.test", "/api/v1/health")
                    break
                except (URLError, AssertionError):
                    time.sleep(.1)
            else:
                raise AssertionError("Linux server/nginx did not become ready")
            password = secrets.token_urlsafe(24)
            command("docker", "exec", "-i", name, "/veruna-releases", "admin", "bootstrap", "--stdin",
                    input=f"owner\n{password}\n")
            for index, host in enumerate(["releases.example.test", "another.example.test"]):
                session, headers = request(host, "/api/v1/auth/login", {"username": "owner", "password": password})
                full_cookie = headers["Set-Cookie"]
                assert full_cookie.startswith("__Host-veruna_session=") and "; Secure" in full_cookie
                assert "Domain=" not in full_cookie
                cookie, csrf = full_cookie.split(";", 1)[0], session["csrf_token"]
                html, headers = request(host, "/admin/applications/example/integrations/tauri")
                assert b"<html" in html and "text/html" in headers["Content-Type"]
                invitation = {"username": f"invited-{index}", "role": "operator"}
                request(host, "/api/v1/admin/users/invitations", invitation, cookie, csrf,
                        origin="https://attacker.example", expected=403)
                token, _ = request(host, "/api/v1/admin/users/invitations", invitation, cookie, csrf, expected=201)
                assert token["activation_url"].startswith(f"https://{host}/admin/activate#token=")
                app, _ = request(host, "/api/v1/admin/applications", {"name": "Smoke", "slug": f"smoke-{index}"}, cookie, csrf, expected=201)
                settings, _ = request(host, f'/api/v1/admin/applications/{app["id"]}/integrations/tauri', cookie=cookie)
                assert settings["public_base_url"] == f"https://{host}"
                assert settings["endpoint"].startswith(f"https://{host}/api/v1/updates/tauri/")
            print("Origin smoke passed: Linux x86_64, non-root, port 3001, loopback nginx/TLS, two automatic domains, secure cookies, CSRF, invitations, Tauri URLs, nested documents; no configured public URLs.")
        except Exception:
            if started:
                for diagnostic in [("exec", name, "cat", "/tmp/server.log"), ("logs", name)]:
                    output = subprocess.run(("docker", *diagnostic), text=True, capture_output=True)
                    print(output.stdout + output.stderr, file=sys.stderr)
            raise
        finally:
            if started:
                subprocess.run(("docker", "rm", "--force", name), check=False, capture_output=True)


if __name__ == "__main__":
    main()
