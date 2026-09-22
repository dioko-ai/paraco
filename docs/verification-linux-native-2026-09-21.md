# Native Linux verification — 2026-09-21

Native x86-64 checks, systemd supervision, terminal independence, durable-state
restoration, and clean offline bundles pass. Machine reboot and the public
service setup/remove workflow in a dedicated test account remain unverified.
This is the execution record for [the handoff](linux-agent-handoff.md), not a
claim that every M0–M3 gate or release requirement is complete.

## Source and environment

Base commit: `c039a810584851ee314e3ffd12339d96663ae46d` (initially clean).
The tested implementation includes the `src/service.rs` fix described below.
Evidence is under [`verification-artifacts/linux-native`](../verification-artifacts/linux-native/).
`source-diff.patch`, `source-diff.sha256`, and `source-sha256.txt` bind the final
implementation and verification scripts, including the new durable-state probe.
`environment.txt` captures the initial run; `environment-final.txt` and
`pinned-tools.txt` supersede its early missing-Deno observation.

- Physical/native x86-64 Linux host (`systemd-detect-virt`: `none`), Ubuntu
  24.04.4 LTS, kernel 6.8.0-138-generic, glibc 2.39. This does not establish the
  proposed Ubuntu 22.04/minimum-libc baseline.
- systemd 255, active user session, session type `tty`, `Linger=no` before and
  after testing. No unattended/pre-login startup configuration was changed.
- Rust/cargo 1.96.1 with rustfmt/Clippy, Deno 2.2.5, Node 22.14.0, Playwright
  1.52.0. Python 3 and OpenSSL support the local HTTPS fixture.
- Node and Deno were downloaded from official versioned upstream URLs into
  `/tmp/paraco-native-tools`; system runtimes were not replaced. Node's archive
  was checked against upstream SHASUMS256; downloaded archive/executable hashes
  and the existing Rust binary hashes are retained. Deno's recorded SHA-256 is
  an observed download checksum, not an independently authenticated signature.
- Existing Playwright Chromium dependencies worked; `npm ci` and
  `npx playwright install chromium` passed. No system package changes were needed.

## Checks and failures repaired

`CI=1 RUST_TEST_THREADS=1 npm run check` passed both before and after the renderer
fix. Final `check-fixed.log` records formatting, strict all-target Clippy, 119
unique Rust tests (core tests repeat in the full suite), schema/type checks,
management DOM checks, pin enforcement, and real Chromium isolation. There were
no skipped Rust tests. Named tests include forced parent death during import and
blocked event loop, immediate restart/stale endpoint ownership, blocked output,
AI grants/authentication, browser storage/cookie/origin boundaries, management
access, and IPv4/IPv6 listener behavior.

Native systemd initially rejected `WorkingDirectory="/tmp/.../state"` because
that directive consumes a path, not a shell-style argument. The renderer now
validates the path without adding quotes; `ExecStart` arguments remain quoted.
The unit regression and actual native runs both use a state directory with spaces.
`systemd-initial.log` and `systemd-initial-journal.log` retain the failure.

The initial prepared probe served HTTP before the host had consumed Deno's
readiness message, then interrupted startup and incorrectly expected exit 0.
Both prepared/offline probes now wait for host readiness before normal shutdown.
`prepared-initial.log` retains the failure; `prepared.log` records two successful
relocated launches. The native service probe now also cleans up after partial
setup. The exact stale symlink from the initial failed unit was removed.

`exit-statuses.txt` retains initial failures and successful follow-ups rather
than overwriting them. The first tool bootstrap lacked `unzip`; Python's standard
`zipfile` module extracted the downloaded Deno archive successfully instead.

## Workload and crash restoration

`foundation.log` records all apps running before measurement, 64 dashboard
requests plus eight noisy-app requests per workload, all 72 returning HTTP 200.
These are individual debug-build observations, not performance guarantees.
Process-tree RSS sums can double-count shared pages.

| Apps | Startup ms | Management ms | Host RSS after load KiB | Tree RSS after load KiB |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 174 | 10 | 20,248 | 104,584 |
| 10 | 295 | 11 | 21,492 | 745,016 |
| 50 | 1,166 | 8 | 27,936 | 3,634,676 |

The foundation probe also verifies app-to-app storage isolation, draft source and
UTC timezone retention after SIGKILL, stable deployment ID, stopped intent, and
rotation of the private browser entry URL. Bearer URLs are not retained.

## Bundles and clean offline execution

Installer tests and the original and corrected bundle smoke tests pass.
`bundle-fixed.json` records the final release binary/private-Deno digests and
source revision; the accompanying source patch identifies the uncommitted fix.
The unsigned fixture archive SHA-256 is
`181e91dd7b68f5348bf8594bc7c071a63d3489dca5b5f6f4e2a4a9f433a214d5`.
Notices are explicitly non-distributable verification fixtures, not complete
third-party notices. No archive was published.

The clean test used amd64 `ubuntu:24.04` at digest
`sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3`,
glibc 2.39, with Docker `--network none`. Only `lo` existed. No developer home or
cache was mounted, and Rust/cargo, Node, and system Deno were absent. Inputs were
only the archive/checksum, read-only relocated prepared artifact, hello source,
and verification script. The archive was checksum-verified and extracted under
`/opt/relocated` inside the container.

Two prepared launches returned `dependency-ok`; one ordinary bundled launch
returned `Hello from Paraco`. All returned HTTP 200 and exited cleanly from cwd
`/`, through a symlinked launcher, with `PATH=/missing`. This separately proves
artifact-contained Deno and bundle-private Deno resolution. Invalid preparation
into a new revision failed without publishing it. Every installed artifact file
hash matched before the failure, afterward, and after both offline launches.
See `clean-offline.log`, `clean-image.txt`, and `prepared-*.sha256`.

## Native manager, update, backup, and cleanup

`systemd.log` records the private-Deno bundle starting under a real systemd user
manager, restart after forced main-process death, preserved stopped intent and
identity, rotated authenticated entry, HTTP success, and removal.

`systemd-durable.log` records a second unique temporary unit:

- It remained operational after the launching pseudo-terminal/session exited.
- A distinctive draft (`return 8675309`) and `America/Denver` timezone were stored;
  a second app could not read the draft and was deliberately stopped.
- After stopping the service, `paraco backup` exported a private mode-0600 SQLite
  snapshot. Updating source at the same canonical paths retained IDs/data/intent.
- Restoring that snapshot into a fresh mode-0700 state directory retained the
  same IDs, draft, timezone, and stopped intent. The original database directory
  remained intact during the restore test. Fixture IDs and backup hash are logged.
- Invalid legacy JSON failed without creating a schema; a future-schema copy
  was refused without changing its database bytes. Unit tests additionally cover
  legacy constraint rollback and failed service-activation file rollback.
- Nine observed cgroup processes were reaped across three stops, cgroups emptied,
  and the gateway listener closed. All test units and the offline container were
  removed; `cleanup.log` records no matching units and unchanged `Linger=no`.

These tests use temporary unique units, not the public `paraco.service`
registration. Downtime source update/restore evidence does not establish the full
public remove/setup installation workflow.

## Remaining gates

- No disposable VM or VM manager was found. The user had no known VM to supply.
  No machine was rebooted. Login-after-reboot restoration remains unobserved.
- A dedicated test account was unavailable and `sudo -n true` required a password.
  Public `service setup/status/open/remove`, registration upgrade/downtime, and
  data preservation in that account remain open. Existing user registrations
  were not replaced. Activation rollback is unit-tested, not a native failure
  injection into that public workflow.
- Clean macOS and macOS machine reboot remain open, as do unobserved CPU/OS/libc
  and browser combinations. This run adds native Linux x86-64, not universal support.
- Release signing, authentic provenance, complete notices, and publication remain
  separate. Scheduling and the cron product were not implemented.

Commands used for bootstrap, checks, bundle build, and clean container execution
are retained in the evidence `commands/` directory. The reusable service and
backup probes live in `scripts/verify-service.py` and
`scripts/verify-linux-durable.py`.
