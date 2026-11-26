#!/usr/bin/env python3
"""Exercise a signed, self-hosted update with two real Linux builds in an isolated directory."""
import argparse
import base64
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import uuid
from urllib.error import HTTPError, URLError
from urllib.request import HTTPCookieProcessor, Request, build_opener


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, **kwargs).stdout


def encoded(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def wait_for(probe, predicate, timeout=90):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            last = probe()
            if predicate(last):
                return last
        except HTTPError as error:
            if error.code != 503:
                raise
        except (URLError, ConnectionError):
            pass  # The listener goes away during a switch.
        time.sleep(0.15)
    raise AssertionError(f"Timed out waiting for server, last state: {last}")


def test_identity(directory):
    directory.mkdir(mode=0o700)
    key = directory / "test.pem"
    run("openssl", "genpkey", "-algorithm", "Ed25519", "-out", key)
    public = encoded(run("openssl", "pkey", "-in", key, "-pubout", "-outform", "DER")[-32:])
    identity = {"product_id": str(uuid.uuid4()), "public_key": {"kty": "OKP", "crv": "Ed25519", "x": public}}
    (directory / "identity.json").write_text(json.dumps(identity) + "\n")


def scenario(root, old, new, mode, identity_dir):
    initial, candidate = [json.loads(run(path, "--standalone-updater-build-info", cwd=root))
                          for path in (old, new)]
    for field in ("rust_target", "product_id", "data_compatibility"):
        assert initial[field] == candidate[field], field
    target = "linux-" + initial["rust_target"].split("-")[0]
    version = candidate["version"]
    product = candidate["product_id"]
    identity = json.loads((identity_dir / "identity.json").read_text())
    if product != identity["product_id"]:
        raise ValueError("Build both test binaries with RELEASE_UPDATE_IDENTITY pointing to this fixture")
    key = identity_dir / "test.pem"
    public = identity["public_key"]["x"]
    executable = root / "veruna-releases"
    shutil.copy2(old, executable)
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        address = f"127.0.0.1:{listener.getsockname()[1]}"
    base = "http://" + address
    env = {"PATH": os.environ["PATH"], "SERVER__BIND": address, "SERVER__OPEN_BROWSER": "false",
           "SELF_UPDATE__MANIFEST_URL": base + "/releases/manifest.json"}
    if mode is not None:
        env.update({"SELF_UPDATE__MODE": mode, "SELF_UPDATE__DIRECTORY": str(root / "updates")})
    password = secrets.token_urlsafe(24)
    run(executable, "admin", "bootstrap", "--stdin", cwd=root, env=env, input=f"owner\n{password}\n".encode())
    browser = build_opener(HTTPCookieProcessor(http.cookiejar.CookieJar()))
    csrf = ""

    def api(path, method="GET", data=None):
        headers = {"Origin": base, "X-Veruna-Request": "1", "X-CSRF-Token": csrf, "Content-Type": "application/json"}
        req = Request(base + "/api/v1" + path, method=method, headers=headers,
                      data=None if data is None else json.dumps(data).encode())
        with browser.open(req, timeout=10) as response:
            payload = response.read()
            return json.loads(payload) if payload else None

    update = "/admin/self-update"

    def stage(name, operation):
        def reached(summary):
            status = summary["status"]
            assert status["operation_id"] == operation, status
            if status["state"] == "failed" and name != "failed":
                raise AssertionError(status)
            return status["state"] == name
        return wait_for(lambda: api(update), reached)

    # Exercise the real producer and client against the same live SQLite catalog.
    def publish(binary, release_version):
        metadata = {"schema_version": 1, "application_id": product, "version": release_version,
                    "os": "linux", "arch": target.split("-")[1], "size_bytes": binary.stat().st_size,
                    "sha256": digest(binary)}
        header = {"alg": "EdDSA", "typ": "releases-standalone+jws"}
        signing_input = ".".join(encoded(json.dumps(obj).encode()) for obj in (header, metadata))
        (root / "message").write_text(signing_input)
        signature = run("openssl", "pkeyutl", "-sign", "-rawin", "-inkey", key, "-in", root / "message")
        (root / "file.jws").write_text(signing_input + "." + encoded(signature))
        manifest = {"version": release_version, "notes": "Smoke", "artifacts": [
            {"target": target, "distribution_role": "executable", "file_type": "bin",
             "file": "binary", "standalone_signature": "signature"}]}
        (root / "release.json").write_text(json.dumps(manifest))
        run("curl", "--fail", "--silent", "--show-error", "-H", "@-",
            "-F", "release=@release.json;type=application/json",
            "-F", f"binary=@{binary};filename=veruna-releases", "-F", "signature=@file.jws",
            base + "/api/v1/publish/releases", cwd=root, input=("Authorization: Bearer " + token + "\n").encode())

    def prepare(release_version):
        checked = api(update + "/check", "POST", {})
        operation = checked["operation_id"]
        assert stage("available", operation)["status"]["available_version"] == release_version
        api(update + "/prepare", "POST", {"check_id": operation, "version": release_version})
        return operation

    def publish_broken(release_version, fingerprint):
        # A signed native fixture answers the real preflight, then fails to start.
        info = dict(candidate, version=release_version, data_compatibility=fingerprint)
        program = ('#include <stdio.h>\n#include <string.h>\nint main(int argc, char **argv) {\n'
                   '  if (argc == 2 && strcmp(argv[1], "--standalone-updater-build-info") == 0) {\n'
                   f'    puts({json.dumps(json.dumps(info))}); return 0;\n'
                   '  }\n  return 1;\n}\n')
        fixture = root / "broken-release"
        run("cc", "-x", "c", "-", "-o", fixture, input=program.encode())
        publish(fixture, release_version)

    server = subprocess.Popen([executable], cwd=root, env=env, start_new_session=True, stdout=subprocess.DEVNULL)
    try:
        wait_for(lambda: api("/health"), lambda health: health["version"] == initial["version"])
        assert api("/health/self-update")["version"] == initial["version"]
        try:
            browser.open(Request(base + "/api/v1/health/self-update", headers={"X-Forwarded-Proto": "https"}))
        except HTTPError as error:
            assert error.code == 404
        else:
            raise AssertionError("Readiness nonce exposed by a proxied request")
        csrf = api("/auth/login", "POST", {"username": "owner", "password": password})["csrf_token"]
        summary = api(update)
        assert summary["mode"] == (mode or "supervisor") and summary["installation_supported"], summary
        if mode is None:
            assert (root / "data/updates").stat().st_mode & 0o777 == 0o700
        app = api("/admin/applications", "POST", {"slug": "releases", "name": "Releases"})
        app_path = "/admin/applications/" + app["id"]
        api(app_path, "PATCH", {"visibility": "public"})
        api(app_path + "/integrations/standalone", "PUT", {"product_id": product, "public_key": public})
        token = api(app_path + "/publication-tokens", "POST", {"name": "Smoke"})["token"]
        publish(new, version)
        with browser.open(base + "/releases/manifest.json") as response:
            published = json.load(response)["releases"][0]["artifacts"][0]
        assert published["purpose"] == "executable" and published["signed_metadata"] == (root / "file.jws").read_text()
        operation = prepare(version)
        stage("prepared", operation)
        api(update + "/cancel", "POST", {"operation_id": operation})
        assert api("/health")["version"] == initial["version"]
        operation = prepare(version)
        stage("prepared", operation)
        api(update + "/install", "POST", {"operation_id": operation})
        assert stage("completed", operation)["current_version"] == version
        assert api(app_path)["slug"] == "releases" and api("/auth/me")["user"]["username"] == "owner"
        assert digest(executable) == digest(new if mode == "standalone" else old)
        if mode != "standalone":
            assert server.poll() is None
            # Admin commands still reach SQLite when the artifact directory is unavailable.
            run(executable, "admin", "bootstrap", cwd=root, env=dict(env, STORAGE__DIRECTORY=str(executable)),
                stdin=subprocess.DEVNULL)
        checked = api(update + "/check", "POST", {})
        assert stage("idle", checked["operation_id"])["status"]["check_outcome"] == "no_update"
        major = int(version.split(".")[0])
        incompatible = f"{major + 1}.0.0"
        publish_broken(incompatible, "different-format")
        assert not stage("failed", prepare(incompatible))["status"]["recovery_required"]
        assert api("/health")["version"] == version
        broken = f"{major + 2}.0.0"
        publish_broken(broken, candidate["data_compatibility"])
        operation = prepare(broken)
        stage("prepared", operation)
        api(update + "/install", "POST", {"operation_id": operation})
        assert stage("rolled_back", operation)["current_version"] == version
        assert api(app_path)["id"] == app["id"]
        print(f"{mode or 'default'}: {initial['version']} -> {version}, incompatible update refused, failed startup rolled back; catalog and session preserved", flush=True)
    finally:
        try:
            os.killpg(server.pid, signal.SIGTERM)
            server.wait(timeout=20)
        finally:
            try:
                os.killpg(server.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("init").add_argument("identity_dir", type=Path)
    test = commands.add_parser("run")
    test.add_argument("identity_dir", type=Path)
    test.add_argument("current", type=Path)
    test.add_argument("candidate", type=Path)
    test.add_argument("--mode", choices=("supervisor", "standalone"))
    args = parser.parse_args()
    if args.command == "init":
        test_identity(args.identity_dir)
    else:
        with tempfile.TemporaryDirectory(prefix="veruna-update-") as directory:
            scenario(Path(directory), args.current.resolve(), args.candidate.resolve(), args.mode, args.identity_dir.resolve())
