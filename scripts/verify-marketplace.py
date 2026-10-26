#!/usr/bin/env python3
"""Exercise the source-package -> approved install -> live runtime boundary."""
import json
import hashlib
import re
import os
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target/debug/paraco"


def run(*args, ok=True):
    result = subprocess.run([str(a) for a in args], cwd=ROOT, capture_output=True, text=True)
    if (result.returncode == 0) != ok:
        raise AssertionError(f"unexpected result for {args}: {result.stdout}\n{result.stderr}")
    return result


run("cargo", "build", "--locked")
with tempfile.TemporaryDirectory(prefix="paraco-marketplace-") as temporary:
    root = Path(temporary)
    package = root / "hello-world.paraco.json"
    run("python3", ROOT / "templates/hello-world/scripts/package.py", "--version", "0.1.0", "--output", package)
    # A changed template is a new candidate, not the bytes already published at v0.1.0.
    # Use a local explicit approval fixture; never rewrite the shipped release digest.
    catalog = root / "catalog.yaml"
    catalog.write_text(re.sub(r"sha256: [0-9a-f]{64}", "sha256: " + hashlib.sha256(package.read_bytes()).hexdigest(), (ROOT / "marketplace/catalog.yaml").read_text(), count=1))
    installed = root / "installed"
    command = [BINARY, "marketplace", "install", "hello-world", "--catalog", catalog, "--package", package, "--output", installed]
    run(*command)
    assert (installed / "paraco-install.json").is_file()
    run(*command, ok=False)
    bad = root / "corrupted.json"
    bad.write_bytes(package.read_bytes() + b" ")
    run(BINARY, "marketplace", "install", "hello-world", "--catalog", catalog, "--package", bad, "--output", root / "rejected", ok=False)
    assert not (root / "rejected").exists()
    import socket
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    with (root / "runtime.log").open("w+") as log:
        process = subprocess.Popen([str(BINARY), "--log-dir", str(root / "logs"), "run", str(installed), "--port", str(port)], cwd=root, stdout=log, stderr=log)
        try:
            for _ in range(100):
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as response:
                        assert json.load(response) == {"status": "ok", "name": "hello-world"}
                    break
                except OSError:
                    if process.poll() is not None:
                        log.seek(0)
                        raise AssertionError(log.read())
                    time.sleep(0.1)
            else:
                raise AssertionError("installed app did not become healthy")
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=2) as response:
                assert b"Your first Paraco app" in response.read()
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
print("Marketplace: approved install, receipt, reinstall refusal, checksum rejection and live app passed")
