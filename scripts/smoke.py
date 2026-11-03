#!/usr/bin/env python3
"""Check a standalone binary from an empty directory, using only the stdlib."""
import json
import os
from pathlib import Path
import re
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import urlopen


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/veruna-releases").resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="veruna-smoke-") as directory:
        root = Path(directory)
        environment = {key: value for key, value in os.environ.items() if not key.startswith("VERUNA_")}
        environment.update({
            "VERUNA__SERVER__BIND": "127.0.0.1:0",
            "VERUNA__DATABASE__PATH": str(root / "state/db/releases.sqlite"),
            "VERUNA__STORAGE__DIRECTORY": str(root / "storage/artifacts"),
        })
        # Unrelated Unix environment bytes must not crash the config loader.
        if os.name == "posix":
            environment["VERUNA_SMOKE_UNRELATED"] = os.fsdecode(b"\xff")
        log_path = root / "server.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([str(binary)], cwd=root, env=environment, stdout=log, stderr=log)
        try:
            address = None
            for _ in range(100):
                for line in log_path.read_text().splitlines():
                    record = json.loads(line)
                    if record.get("fields", {}).get("message") == "Veruna Releases started":
                        address = record["fields"]["address"]
                if address:
                    break
                if process.poll() is not None:
                    raise AssertionError(log_path.read_text())
                time.sleep(0.1)
            assert address, "Server did not become ready"
            base = f"http://{address}"

            def get(path):
                return urlopen(base + path, timeout=5)

            with get("/api/v1/health?token=not-for-logs") as response:
                health = json.load(response)
                assert health["status"] == "ok" and health["service"] == "veruna-releases"
            with get("/admin/releases") as response:
                assert response.headers["Content-Type"].startswith("text/html")
                html = response.read().decode()
            assets = re.findall(r'(?:src|href)="(/[^\"]+)"', html)
            assert any(path.endswith(".js") for path in assets)
            assert any(path.endswith(".css") for path in assets)
            for path in assets:
                with get(path) as response:
                    assert response.status == 200 and response.read(), path
            for path, status in [("/api/v1/missing", 404), ("/assets/missing.js", 404), ("/api/v1/admin/applications", 503)]:
                try:
                    get(path)
                    raise AssertionError(f"Unexpected success: {path}")
                except HTTPError as error:
                    assert error.code == status
                    assert "error" in json.load(error)
            assert (root / "storage/artifacts").is_dir()
            with sqlite3.connect(root / "state/db/releases.sqlite") as database:
                assert database.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
                assert database.execute("SELECT count(*) FROM _sqlx_migrations WHERE success = 1").fetchone()[0] == 1
            process.send_signal(signal.SIGTERM)
            assert process.wait(timeout=20) == 0, log_path.read_text()
            logs = log_path.read_text()
            assert "Veruna Releases stopped" in logs
            assert "not-for-logs" not in logs
            print("Smoke passed: standalone UI, JS/CSS, health, API boundaries, configured paths, WAL, migrations, SIGTERM.")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


if __name__ == "__main__":
    main()
