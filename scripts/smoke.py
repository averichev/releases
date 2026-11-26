#!/usr/bin/env python3
"""Check the standalone binary and real cookie/API flow using only the stdlib."""
import base64
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import struct
import sqlite3
import subprocess
import sys
import tempfile
import time
import zlib
from urllib.error import HTTPError
from urllib.request import Request, HTTPCookieProcessor, build_opener


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/veruna-releases").resolve(strict=True)
    migration_count = len(list((Path(__file__).resolve().parent.parent / "migrations").glob("[0-9]*.sql")))
    with tempfile.TemporaryDirectory(prefix="veruna-smoke-") as directory:
        root = Path(directory)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        environment = {key: value for key, value in os.environ.items() if key != "CONFIG" and not key.startswith(("VERUNA_", "SERVER__", "DATABASE__", "STORAGE__", "LOGGING__", "AUTH__", "DOWNLOAD__", "SELF_UPDATE__"))}
        environment.update({
            "SERVER__BIND": f"127.0.0.1:{port}",
            "SERVER__OPEN_BROWSER": "false",
            "SERVER__AUTO_PORT": "false",
            "DATABASE__PATH": str(root / "state/db/releases.sqlite"),
            "STORAGE__DIRECTORY": str(root / "storage/artifacts"),
            "STORAGE__ICONS_DIRECTORY": str(root / "storage/icons"),
        })
        if os.name == "posix":
            environment["VERUNA_SMOKE_UNRELATED"] = os.fsdecode(b"\xff")
        password = secrets.token_urlsafe(24)
        bootstrap = subprocess.run([str(binary), "admin", "bootstrap", "--stdin"],
                                   input=f"owner\n{password}\n", text=True, capture_output=True,
                                   cwd=root, env=environment, timeout=30)
        assert bootstrap.returncode == 0, bootstrap.stderr
        assert password not in bootstrap.stdout + bootstrap.stderr
        repeat = subprocess.run([str(binary), "admin", "bootstrap"], stdin=subprocess.DEVNULL,
                                text=True, capture_output=True, cwd=root, env=environment, timeout=30)
        assert repeat.returncode == 0 and "уже существует" in repeat.stdout
        log_path = root / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([str(binary)], cwd=root, env=environment, stdout=log, stderr=log)
        secret_values = [password, "not-for-logs", "path-secret-not-for-logs"]
        try:
            for _ in range(100):
                if "Veruna Releases started" in log_path.read_text():
                    break
                if process.poll() is not None:
                    raise AssertionError(log_path.read_text())
                time.sleep(0.1)
            else:
                raise AssertionError("Server did not become ready")
            jar = http.cookiejar.CookieJar()
            browser = build_opener(HTTPCookieProcessor(jar))

            def call(path, method="GET", payload=None, csrf=None, expected=200, opener=browser, content_type="application/json"):
                headers = {"Accept": "application/json"}
                data = None
                if method != "GET":
                    headers.update({"Origin": base, "X-Veruna-Request": "1", "Content-Type": content_type})
                    data = payload if isinstance(payload, bytes) else json.dumps(payload if payload is not None else {}).encode()
                    if csrf:
                        headers["X-CSRF-Token"] = csrf
                try:
                    response = opener.open(Request(base + path, data=data, headers=headers, method=method), timeout=10)
                except HTTPError as error:
                    response = error
                with response:
                    assert response.code == expected, (path, response.code, expected)
                    body = response.read()
                    return (json.loads(body) if body and "application/json" in response.headers.get("Content-Type", "") else body), response.headers

            health, _ = call("/api/v1/health?token=not-for-logs")
            assert health["status"] == "ok" and health["service"] == "veruna-releases"
            if len(sys.argv) > 2:
                assert health["version"] == sys.argv[2], (health["version"], sys.argv[2])
            html, headers = call("/admin/users")
            assert headers["Content-Type"].startswith("text/html")
            assets = re.findall(r'(?:src|href)="(/[^\"]+)"', html.decode())
            assert any(path.endswith(".js") for path in assets)
            assert any(path.endswith(".css") for path in assets)
            for path in assets:
                body, _ = call(path)
                assert body, path
            for path, status in [("/api/v1/missing", 404), ("/assets/missing.js", 404),
                                 ("/api/v1/admin/users", 401), ("/path-secret-not-for-logs", 404)]:
                body, _ = call(path, expected=status)
                assert ("error" in body) if isinstance(body, dict) else b"<html" in body
            session, headers = call("/api/v1/auth/login", "POST", {"username": "owner", "password": password})
            assert "HttpOnly" in headers["Set-Cookie"] and "SameSite=Lax" in headers["Set-Cookie"]
            csrf = session["csrf_token"]
            secret_values.extend([cookie.value for cookie in jar])
            secret_values.append(csrf)
            me, _ = call("/api/v1/auth/me")
            assert me["user"]["username"] == "owner" and "password_hash" not in me["user"]
            call("/api/v1/auth/logout", "POST", expected=403)  # no CSRF
            users, _ = call("/api/v1/admin/users")
            assert len(users) == 1
            catalog, _ = call("/api/v1/admin/applications")
            assert catalog["items"] == [] and catalog["total"] == 0
            application, _ = call("/api/v1/admin/applications", "POST", {"name": "Проверка каталога", "slug": "catalog-smoke"}, csrf, 201)
            application_path = f'/api/v1/admin/applications/{application["id"]}'
            assert application["visibility"] == "private" and application["status"] == "active"
            call(application_path, "PATCH", {"slug": "changed"}, csrf, 400)
            application, _ = call(application_path, "PATCH", {"description": "Описание", "visibility": "public"}, csrf)

            def png_chunk(kind, data):
                return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

            png = (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 6, 0, 0, 0))
                   + png_chunk(b"IDAT", zlib.compress((b"\x00" + b"\x23\x6b\x54\xff" * 2) * 2)) + png_chunk(b"IEND", b""))
            application, _ = call(application_path + "/icon", "POST", png, csrf)
            icon, icon_headers = call(application["icon_url"])
            assert icon.startswith(b"\x89PNG") and icon_headers["Content-Type"] == "image/png"
            assert icon_headers["Cache-Control"] == "no-store"
            svg = b'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" onload="alert(1)"><path d="M0 0H24V24Z"/><script>alert(1)</script></svg>'
            application, _ = call(application_path + "/icon", "POST", svg, csrf)
            icon, icon_headers = call(application["icon_url"])
            assert icon_headers["Content-Type"] == "image/svg+xml"
            assert b"<path" in icon and b"onload" not in icon and b"script" not in icon
            assert "sandbox; default-src 'none'" in icon_headers["Content-Security-Policy"]
            public_icon, public_headers = call("/api/v1/public/applications/catalog-smoke/icon", opener=build_opener())
            assert public_icon == icon and public_headers["Content-Type"] == "image/svg+xml"
            call(application_path + "/icon", "POST", b"<svg/>", csrf, 400)
            call(application_path + "/archive", "POST", csrf=csrf)
            active, _ = call("/api/v1/admin/applications")
            assert active["total"] == 0
            archived, _ = call("/api/v1/admin/applications?status=archived&visibility=public&search=catalog")
            assert archived["items"][0]["id"] == application["id"]
            call(application_path + "/restore", "POST", csrf=csrf)
            nested, nested_headers = call(f'/admin/applications/{application["id"]}/settings')
            assert nested_headers["Content-Type"].startswith("text/html") and nested == html
            integration_path = application_path + "/integrations/tauri"
            nested, nested_headers = call(f'/admin/applications/{application["id"]}/integrations/tauri?step=3')
            assert nested == html and nested_headers["Content-Type"].startswith("text/html")
            integration, _ = call(integration_path)
            assert integration["public_key"] is None and integration["configuration"] is None and not integration["ready"]
            # Syntactic public-key fixture only; this smoke does not claim a client installation.
            public_key = base64.b64encode(("untrusted comment: smoke public key\n" + base64.b64encode(b"Ed" + bytes(40)).decode() + "\n").encode()).decode()
            call(integration_path, "PUT", {"public_key": public_key}, expected=403)
            call(integration_path, "PUT", {"public_key": public_key}, csrf, 204)
            call(integration_path, "PUT", {"private_key": "never accepted"}, csrf, 400)
            integration, _ = call(integration_path)
            assert integration["public_key"] == public_key
            assert integration["configuration"]["plugins"]["updater"]["pubkey"] == public_key
            assert integration["endpoint"] == base + "/api/v1/updates/tauri/catalog-smoke/{{target}}/{{arch}}/{{current_version}}"
            release, _ = call(application_path + "/releases", "POST", {"version": " v1.2.0 ", "release_notes": "Smoke changes"}, csrf, 201)
            assert release["version"] == "1.2.0" and release["status"] == "draft" and release["published_at"] is None
            release_path = application_path + "/releases/" + release["id"]
            call(application_path + "/releases", "POST", {"version": "1.2.0+build.2"}, csrf, 409)
            uploaded = []
            for target, filename, contents in [("windows-x86_64", "installer.exe", b"windows installer"), ("linux-x86_64", "installer.tar.gz", b"linux installer")]:
                multipart = (f'--SMOKE\r\nContent-Disposition: form-data; name="target"\r\n\r\n{target}\r\n'
                             f'--SMOKE\r\nContent-Disposition: form-data; name="file"; filename="{filename}"\r\n\r\n').encode() + contents + b"\r\n--SMOKE--\r\n"
                artifact, _ = call(release_path + "/artifacts", "POST", multipart, csrf, 201, content_type="multipart/form-data; boundary=SMOKE")
                assert artifact["size_bytes"] == len(contents) and artifact["sha256"] == hashlib.sha256(contents).hexdigest()
                assert "storage_key" not in artifact
                download = release_path + "/artifacts/" + artifact["id"] + "/download"
                data, headers = call(download)
                assert data == contents and headers["Content-Type"] == "application/octet-stream"
                assert headers["Content-Disposition"].startswith("attachment;")
                call(download, opener=build_opener(), expected=401)
                verified, _ = call(release_path + "/artifacts/" + artifact["id"] + "/verify", "POST", csrf=csrf)
                assert verified["valid"] is True
                uploaded.append(artifact)
            release, _ = call(release_path)
            assert release["artifact_count"] == 2
            nested, nested_headers = call(f'/admin/applications/{application["id"]}/releases/{release["id"]}')
            assert nested == html and nested_headers["Content-Type"].startswith("text/html")
            invitation, _ = call("/api/v1/admin/users/invitations", "POST", {"username": "operator", "role": "operator"}, csrf, 201)
            token = invitation["activation_url"].split("#token=")[1]
            secret_values.append(token)
            call("/api/v1/auth/activate", "POST", {"token": token, "password": password}, expected=204)
            call("/api/v1/auth/activate", "POST", {"token": token, "password": password}, expected=400)
            call("/api/v1/auth/logout", "POST", csrf=csrf, expected=204)
            call("/api/v1/auth/me", expected=401)
            operator, _ = call("/api/v1/auth/login", "POST", {"username": "operator", "password": password})
            call("/api/v1/admin/users", expected=403)
            catalog, _ = call("/api/v1/admin/applications")
            assert catalog["total"] == 1
            call(application["icon_url"])
            call(release_path)
            call(release_path + "/artifacts/" + uploaded[0]["id"] + "/download")
            call(release_path, "DELETE", csrf=operator["csrf_token"], expected=403)
            call(application_path, "PATCH", {"name": "Forbidden"}, operator["csrf_token"], 403)
            call(application_path + "/archive", "POST", csrf=operator["csrf_token"], expected=403)
            call(integration_path)
            call(integration_path, "PUT", {"public_key": public_key}, operator["csrf_token"], 403)
            call(integration_path + "/readiness", "POST", csrf=operator["csrf_token"])
            call(application_path + "/updater/tauri/preview", "POST", {"target": "windows-x86_64", "current_version": "1.0.0"}, operator["csrf_token"])
            call("/api/v1/auth/logout", "POST", csrf=operator["csrf_token"], expected=204)
            recovery = subprocess.run([str(binary), "admin", "reset-password", "--stdin"],
                                      input="owner\n", text=True, capture_output=True,
                                      cwd=root, env=environment, timeout=30)
            assert recovery.returncode == 0, recovery.stderr
            recovery_url = next(line for line in recovery.stdout.splitlines() if line.startswith("/admin/activate#token="))
            recovery_token = recovery_url.split("#token=")[1]
            secret_values.append(recovery_token)
            new_password = secrets.token_urlsafe(24)
            secret_values.append(new_password)
            call("/api/v1/auth/activate", "POST", {"token": recovery_token, "password": new_password}, expected=204)
            recovered, _ = call("/api/v1/auth/login", "POST", {"username": "owner", "password": new_password})
            call(release_path + "/artifacts/" + uploaded[0]["id"], "DELETE", csrf=recovered["csrf_token"], expected=204)
            call(release_path, "DELETE", csrf=recovered["csrf_token"], expected=204)
            call(release_path, expected=404)
            # Stage 5 uses the standalone binary too: real publication, manifests,
            # signed updater diagnostics and immutable files.
            def uploaded_release(version):
                row, _ = call(application_path + "/releases", "POST", {"version": version}, recovered["csrf_token"], 201)
                path = application_path + "/releases/" + row["id"]
                multipart = (b'--SMOKE\r\nContent-Disposition: form-data; name="target"\r\n\r\nwindows-x86_64\r\n'
                             b'--SMOKE\r\nContent-Disposition: form-data; name="file"; filename="setup.exe"\r\n\r\npackage\r\n--SMOKE--\r\n')
                artifact, _ = call(path + "/artifacts", "POST", multipart, recovered["csrf_token"], 201, content_type="multipart/form-data; boundary=SMOKE")
                return row, path, artifact

            published, published_path, artifact = uploaded_release("2.0.0")
            artifact_path = published_path + "/artifacts/" + artifact["id"]
            call(artifact_path + "/updater", "PUT", {"distribution_role": "updater"}, recovered["csrf_token"])
            packet = base64.b64encode(b"ED" + bytes(72)).decode()
            global_signature = base64.b64encode(bytes(64)).decode()
            signature = base64.b64encode(f"untrusted comment: smoke signature\n{packet}\ntrusted comment: smoke\n{global_signature}\n".encode())
            call(artifact_path + "/signature", "PUT", signature, recovered["csrf_token"], content_type="application/octet-stream")
            ready, _ = call(published_path + "/validate-publication", "POST", {}, recovered["csrf_token"])
            assert ready["validation"]["state"] == "ready"
            call(published_path + "/publish", "POST", {"expected_release_revision": ready["release_revision"]}, recovered["csrf_token"])
            current_release, _ = call(published_path)
            assert current_release["status"] == "published"
            catalog, catalog_headers = call("/api/v1/public/applications", opener=build_opener())
            assert catalog["total"] == 1 and catalog["items"][0]["slug"] == "catalog-smoke"
            assert catalog_headers["Cache-Control"] == "no-store"
            manifest, manifest_headers = call("/catalog-smoke/manifest.json", opener=build_opener())
            assert manifest["schema_version"] == 2 and manifest["releases"][0]["version"] == "2.0.0" and "ETag" in manifest_headers
            download_url = manifest["releases"][0]["artifacts"][0]["download_url"]
            assert download_url.startswith(base + "/catalog-smoke/releases/2.0.0/artifacts/")
            public_path = download_url.removeprefix(base)
            download, metadata = call(public_path, opener=build_opener())
            assert download and metadata["Accept-Ranges"] == "bytes"
            with build_opener().open(Request(download_url, method="HEAD")) as head:
                assert int(head.headers["Content-Length"]) == len(download) and not head.read()
            with build_opener().open(Request(download_url, headers={"Range": "bytes=1-3"})) as partial:
                assert partial.status == 206 and partial.read() == download[1:4]
            counted_app, _ = call(application_path)
            counted_release, _ = call(published_path)
            counted_files, _ = call(published_path + "/artifacts")
            assert counted_app["download_count"] == counted_release["download_count"] == 1
            assert sum(artifact["download_count"] for artifact in counted_files) == 1
            update, _ = call("/api/v1/updates/tauri/catalog-smoke/windows/x86_64/1.0.0", opener=build_opener())
            assert update["url"] == download_url and update["signature"] == signature.decode()
            no_update, _ = call("/api/v1/updates/tauri/catalog-smoke/windows/x86_64/2.0.0", opener=build_opener(), expected=204)
            assert not no_update
            page, metadata = call("/catalog-smoke", opener=build_opener())
            assert page and metadata["Content-Type"].startswith("text/html")
            preview, _ = call(application_path + "/updater/tauri/preview", "POST", {"target": "windows-x86_64", "current_version": "1.0.0"}, recovered["csrf_token"])
            assert preview["reason"] == "update_available" and preview["response"]["url"] == download_url
            call(artifact_path, "DELETE", csrf=recovered["csrf_token"], expected=409)
            call(published_path, "PATCH", {"release_notes": "forbidden"}, recovered["csrf_token"], 409)
            newer, newer_path, _ = uploaded_release("3.0.0")
            ready, _ = call(newer_path + "/validate-publication", "POST", {}, recovered["csrf_token"])
            call(newer_path + "/publish", "POST", {"expected_release_revision": ready["release_revision"]}, recovered["csrf_token"])
            manifest, _ = call("/catalog-smoke/manifest.json?limit=1", opener=build_opener())
            assert manifest["latest_by_target"]["windows-x86_64"]["version"] == "3.0.0" and manifest["releases"][0]["version"] == "3.0.0"
            history, _ = call(application_path + "/publication-history")
            assert history[0]["version"] == "3.0.0" and len(history) == 2
            latest_bytes, _ = call("/catalog-smoke/latest/windows-x86_64", opener=build_opener())
            assert latest_bytes == b"package"
            choice, choice_headers = call("/catalog-smoke/latest", opener=build_opener())
            assert choice == page and choice_headers["Content-Type"].startswith("text/html")
            # Universal distribution: one release, explicit types, no extra manifests or signatures.
            universal, _ = call(application_path + "/releases", "POST", {"version": "0.5.0"}, recovered["csrf_token"], 201)
            universal_path = application_path + "/releases/" + universal["id"]
            variants = [
                ("windows-x86_64", "mockport.exe", "executable", b"MZ standalone executable"),
                ("linux-x86_64", "mockport", "executable", b"\x7fELF standalone executable"),
                ("linux-aarch64", "mockport-aarch64", "executable", b"\x7fELF arm executable"),
                ("windows-x86_64", "MockPort-Setup.exe", "installer", b"MZ installer"),
                ("linux-x86_64", "mockport.tar.gz", "archive", b"archive contents"),
            ]
            for target, filename, kind, contents in variants:
                multipart = (f'--SMOKE\r\nContent-Disposition: form-data; name="target"\r\n\r\n{target}\r\n'
                             f'--SMOKE\r\nContent-Disposition: form-data; name="distribution_role"\r\n\r\n{kind}\r\n'
                             f'--SMOKE\r\nContent-Disposition: form-data; name="file"; filename="{filename}"\r\n\r\n').encode() + contents + b"\r\n--SMOKE--\r\n"
                artifact, _ = call(universal_path + "/artifacts", "POST", multipart, recovered["csrf_token"], 201, content_type="multipart/form-data; boundary=SMOKE")
                assert artifact["filename"] == filename and artifact["distribution_role"] == kind
                assert artifact["sha256"] == hashlib.sha256(contents).hexdigest()
                assert not artifact["signature_present"] and not artifact["standalone_signature_present"]
                if filename == "mockport":
                    assert artifact["file_type"] == "bin"
                data, _ = call(universal_path + "/artifacts/" + artifact["id"] + "/download")
                assert data == contents
            call(universal_path + "/artifacts", "POST", multipart, recovered["csrf_token"], 409, content_type="multipart/form-data; boundary=SMOKE")
            ready, _ = call(universal_path + "/validate-publication", "POST", {}, recovered["csrf_token"])
            assert ready["validation"]["state"] == "ready"
            call(universal_path + "/publish", "POST", {"expected_release_revision": ready["release_revision"]}, recovered["csrf_token"])
            for target, filename, kind, contents in variants:
                data, _ = call(f"/catalog-smoke/0.5.0/{target}?type={kind}", opener=build_opener())
                assert data == contents
                if kind != "installer":
                    data, _ = call(f"/catalog-smoke/latest?target={target}&type={kind}", opener=build_opener())
                    assert data == contents
            ambiguous, _ = call("/catalog-smoke/latest?type=executable", opener=build_opener(), expected=409)
            assert ambiguous["error"]["code"] == "ambiguous_artifact"
            # Adding generic files cannot replace the existing signed Tauri update.
            update, _ = call("/api/v1/updates/tauri/catalog-smoke/windows/x86_64/1.0.0", opener=build_opener())
            assert update["version"] == "2.0.0" and update["signature"] == signature.decode()

            # Real curl publication, including a retry with independently generated
            # multipart boundaries. Credentials enter curl only through stdin.
            publication_token, token_headers = call(application_path + "/publication-tokens", "POST",
                                                   {"name": "CI smoke"}, recovered["csrf_token"], 201)
            assert token_headers["Cache-Control"] == "no-store"
            secret_values.append(publication_token["token"])
            cli_directory = root / "releases/4.0.0"
            cli_directory.mkdir(parents=True)
            cli_files = [("linux", "product-linux-x86_64", "linux-x86_64", "bin", b"cli linux executable"),
                         ("windows", "product.exe", "windows-x86_64", "exe", b"cli windows executable")]
            cli_manifest = {"version": "4.0.0", "notes": "## CLI release\n- Atomic publication.", "artifacts": []}
            command = ["curl", "--silent", "--show-error", "--fail-with-body", "--header", "@-",
                       "--write-out", "\n%{http_code}", "-F", "release=@publish.json;type=application/json"]
            for name, filename, target, file_type, contents in cli_files:
                (cli_directory / filename).write_bytes(contents)
                cli_manifest["artifacts"].append({"file": name, "target": target, "distribution_role": "executable",
                                                  "file_type": file_type, "sha256": hashlib.sha256(contents).hexdigest(),
                                                  "size_bytes": len(contents)})
                command.extend(["-F", f"{name}=@{filename}"])
            (cli_directory / "publish.json").write_text(json.dumps(cli_manifest), encoding="utf-8")
            command.append(base + "/api/v1/publish/catalog-smoke")
            cli_results = []
            for status in [201, 200]:
                result = subprocess.run(command, input=f'Authorization: Bearer {publication_token["token"]}\n',
                                        text=True, capture_output=True, cwd=cli_directory, timeout=30)
                assert result.returncode == 0, (result.stderr, result.stdout)
                body, actual_status = result.stdout.rsplit("\n", 1)
                assert int(actual_status) == status, (actual_status, body)
                cli_results.append(json.loads(body))
            assert cli_results[0] == cli_results[1]
            cli_history, _ = call(application_path + "/publication-history")
            event = [row for row in cli_history if row["release_id"] == cli_results[0]["release_id"]]
            assert len(event) == 1 and event[0]["author"] == "Токен CI smoke" and event[0]["user_id"] is None
            for _, _, target, _, contents in cli_files:
                downloaded, _ = call(f"/catalog-smoke/latest?target={target}&type=executable", opener=build_opener())
                assert hashlib.sha256(downloaded).hexdigest() == hashlib.sha256(contents).hexdigest()
            call(application_path + f'/publication-tokens/{publication_token["id"]}/revoke', "POST",
                 csrf=recovered["csrf_token"], expected=204)
            after_revoke, _ = call(application_path + "/publication-history")
            assert after_revoke == cli_history
            print("curl first (201): " + json.dumps(cli_results[0], ensure_ascii=False))
            print("curl retry (200): " + json.dumps(cli_results[1], ensure_ascii=False))
            call(application_path, "PATCH", {"visibility": "private"}, recovered["csrf_token"])
            call("/catalog-smoke/manifest.json", opener=build_opener(), expected=404)
            call(application_path + "/manifests/preview")
            call("/api/v1/auth/logout", "POST", csrf=recovered["csrf_token"], expected=204)
            assert (root / "storage/artifacts").is_dir()
            with sqlite3.connect(root / "state/db/releases.sqlite") as database:
                assert database.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
                assert database.execute("SELECT count(*) FROM _sqlx_migrations WHERE success = 1").fetchone()[0] == migration_count
                assert database.execute("SELECT count(*) FROM user_tokens").fetchone()[0] == 0
                assert database.execute("SELECT slug FROM applications").fetchone()[0] == "catalog-smoke"
                assert database.execute("SELECT tauri_public_key FROM applications").fetchone()[0] == public_key
                icon_key = database.execute("SELECT icon_key FROM applications").fetchone()[0]
                assert icon_key.endswith(".svg") and (root / "storage/icons" / icon_key).read_bytes() == icon
            assert len(list((root / "storage/icons").glob("*.png"))) == 1
            process.send_signal(signal.SIGTERM)
            assert process.wait(timeout=20) == 0, log_path.read_text()
            logs = log_path.read_text()
            assert "Veruna Releases stopped" in logs
            assert all(value not in logs for value in secret_values), "Secret leaked into logs"
            print("Smoke passed: standalone assets, CLI recovery, sessions/CSRF, catalog, sanitized SVG icons, uploads/SHA-256, universal artifact types/extensionless binaries, publication/history, curl batch publication/retry/token revocation, filtered/ambiguous latest, manifest v2, downloads/HEAD/Range, Tauri, immutability, nested routes, safe logs, WAL, migrations, SIGTERM.")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


if __name__ == "__main__":
    main()
