#!/usr/bin/env python3
"""Native systemd terminal-exit, update, backup/restore, and cleanup evidence.

Uses a unique temporary unit and private fixtures; does not install paraco.service,
change lingering, reboot the machine, or retain authenticated entry URLs.
"""
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request

BINARY = str(Path(os.environ.get("PARACO_BIN", "target/debug/paraco")).resolve())


def run(*args, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True, timeout=30)


def wait(check):
    deadline = time.monotonic() + 40
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError, subprocess.CalledProcessError):
            pass
        time.sleep(.1)
    raise RuntimeError("native durable verification timed out")


with tempfile.TemporaryDirectory(prefix="pc-durable-", dir="/tmp") as directory:
    root = Path(directory)
    state = root / "state with spaces"
    state.mkdir(mode=0o700)
    config = root / "server.json"
    for name in ("drafts", "stopped"):
        app = root / name
        app.mkdir()
        (app / "paraco.json").write_text(json.dumps({
            "name": name, "entrypoint": "main.ts", "capabilities": ["storage"]}))
        (app / "main.ts").write_text('''export default {async fetch(r,c) {
          if (r.method === 'PUT') {
            await c.data.set('draft', {source:'return 8675309'});
            await c.config.set('timezone', 'America/Denver');
          }
          return Response.json({draft:await c.data.get('draft'),
            timezone:await c.config.get('timezone'), revision:1});
        }};''')
    config.write_text(json.dumps({"apps": [{"path": str(root / name)}
                                          for name in ("drafts", "stopped")]}))
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    unit = f"dev.paraco.durable.{os.getpid()}.service"
    definition = root / unit
    rendered = run(BINARY, "service", "render", "--platform", "linux", "--entry",
                   BINARY, "--config", str(config), "--state", str(state)).stdout
    definition.write_text(rendered.replace(" serve --config", f" serve --port {port} --config"))

    def manager(*args):
        return run("systemctl", "--user", *args)

    def cli(*args):
        return run(BINARY, *args, "--port", str(port)).stdout

    def statuses():
        return {item["name"]: item for item in json.loads(cli("status"))}

    def request(name="drafts", method="GET"):
        req = urllib.request.Request(f"http://127.0.0.1:{port}/", method=method,
                                     headers={"Host": f"{name}.localhost:{port}"})
        with urllib.request.urlopen(req, timeout=3) as response:
            return json.load(response)

    def start():
        manager("start", unit)
        wait(lambda: statuses()["drafts"]["state"] == "running")

    def stop_and_check():
        group = manager("show", unit, "--property=ControlGroup", "--value").stdout.strip()
        processes = Path("/sys/fs/cgroup") / group.lstrip("/") / "cgroup.procs"
        pids = processes.read_text().split() if processes.exists() else []
        manager("stop", unit)
        wait(lambda: all(not Path(f"/proc/{pid}").exists() for pid in pids))
        with socket.socket() as sock:
            assert sock.connect_ex(("127.0.0.1", port)) != 0, "listener survived stop"
        assert not processes.exists() or not processes.read_text().strip(), "service cgroup not empty"
        return len(pids)

    def restored(expected_ids):
        current = statuses()
        assert {name: item["deployment_id"] for name, item in current.items()} == expected_ids
        assert current["stopped"]["desired"] == current["stopped"]["state"] == "stopped"
        value = request()
        assert value["draft"] == {"source": "return 8675309"}
        assert value["timezone"] == "America/Denver"
        return value

    try:
        manager("link", str(definition))
        # script allocates a real pseudo-terminal; the launching session exits
        # before we ask the independently supervised runtime to handle requests.
        run("script", "-q", "-e", "-c", shlex.join([
            "systemctl", "--user", "start", unit]), "/dev/null")
        wait(lambda: all(item["state"] == "running" for item in statuses().values()))
        ids = {name: item["deployment_id"] for name, item in statuses().items()}
        request(method="PUT")
        assert request("stopped")["draft"] is None
        cli("stop", "stopped")
        wait(lambda: statuses()["stopped"]["state"] == "stopped")
        entry = cli("open", "--print")
        reaped = stop_and_check()
        database = next(state.glob("state/*/state.sqlite3"))
        backup = root / "backup.sqlite3"
        run(BINARY, "backup", "--state", str(database.parent), "--output", str(backup))
        assert backup.stat().st_mode & 0o777 == 0o600
        digest = hashlib.sha256(backup.read_bytes()).hexdigest()

        # Explicit downtime, same canonical config/source, new source revision.
        source = root / "drafts/main.ts"
        source.write_text(source.read_text().replace("revision:1", "revision:2"))
        start()
        assert restored(ids)["revision"] == 2
        assert cli("open", "--print") != entry
        reaped += stop_and_check()

        # Keep the original directory intact and restore into a fresh private one.
        original = database.parent.with_name(database.parent.name + "-original")
        database.parent.rename(original)
        database.parent.mkdir(mode=0o700)
        shutil.copy2(backup, database)
        start()
        assert restored(ids)["revision"] == 2
        reaped += stop_and_check()
        assert hashlib.sha256(backup.read_bytes()).hexdigest() == digest

        invalid = root / "invalid-legacy"
        invalid.mkdir(mode=0o700)
        (invalid / "state.json").write_text('{"format":2,"deployments":INVALID}')
        failed = run(BINARY, "backup", "--state", str(invalid), "--output",
                     str(root / "invalid-export"), check=False)
        assert failed.returncode != 0 and "invalid legacy state" in failed.stderr
        with sqlite3.connect(invalid / "state.sqlite3") as db:
            assert db.execute("PRAGMA user_version").fetchone()[0] == 0
            assert db.execute("SELECT count(*) FROM sqlite_master WHERE type='table'").fetchone()[0] == 0
        future = root / "future"
        future.mkdir(mode=0o700)
        shutil.copy2(backup, future / "state.sqlite3")
        with sqlite3.connect(future / "state.sqlite3") as db:
            db.execute("PRAGMA user_version=999")
        before = (future / "state.sqlite3").read_bytes()
        failed = run(BINARY, "backup", "--state", str(future), "--output",
                     str(root / "future-export"), check=False)
        assert failed.returncode != 0 and "newer" in failed.stderr
        assert (future / "state.sqlite3").read_bytes() == before
        print(json.dumps({"unit": unit, "deployment_ids": ids,
                          "terminal_exit": "service continued after launching PTY exited",
                          "source_update": "revision 2, same IDs/data/stopped intent",
                          "backup_restore": "fresh private state directory; IDs/data/intent preserved",
                          "backup_sha256": digest, "invalid_legacy": "refused without schema",
                          "future_schema": "refused without changing copy",
                          "shutdown": f"{reaped} cgroup processes reaped across 3 stops; listener closed",
                          "machine_reboot": "not performed"}))
    finally:
        manager("disable", "--now", unit)
        manager("daemon-reload")
