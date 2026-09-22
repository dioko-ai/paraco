# Linux agent handoff: M0–M3 native acceptance

## Objective and authorization

Finish the Linux verification requested by the owner: complete pinned checks,
crash cleanup, browser isolation, AI authorization, 1/10/50-app workloads,
offline prepared/private-Deno bundles, SQLite migration/backup/storage, and
native systemd operation with restart restoration. Fix failures and retain
actual results. Do not mark a gate complete from code, CI configuration, or an
emulated target alone. Scheduling and the cron product remain out of scope.

The owner requested this handoff for an agent running on Linux. The current
macOS agent could run Linux ARM64 inside Docker, but had no native systemd user
session or disposable machine-reboot environment. Do not reboot an active user
machine without explicit authorization for that machine. Use a disposable VM
for reboot testing. Do not publish archives or claim signing/release readiness.

## Starting context

Read these files before working:

- `docs/verification-2026-09-21.md` and `verification-artifacts/` for observed
  results, failed attempts, and platform limitations.
- `docs/durable-state.md` for SQLite, storage, backup, removal, and service contracts.
- `docs/foundation-and-cron-plan.md`, M0–M3 exit criteria.
- `docs/development.md`, `docs/prepared-artifacts.md`, and
  `docs/portable-bundles.md` for existing contracts and limitations.

The working tree may contain uncommitted implementation changes from the macOS
agent. Preserve them; do not reset the tree. Record `git status --short`, the
base commit, and a hash of `git diff --binary` with the evidence so results are
bound to the exact tested source. Check for applicable `AGENTS.md` instructions.

Implemented changes include:

- Transactional SQLite schema version 1 with format-2 JSON migration, stable
  deployment IDs/desired state, future-schema rejection, and backup export.
- `storage` capability exposing deployment-scoped config/data for cron drafts.
- `paraco open` via the private same-user control socket, with `--print` for
  headless use. Never place the returned bearer URL in retained logs.
- Prepared artifact format 2 with a private, digest-verified Deno executable,
  relocation, frozen locks, and disposable per-launch dependency caches.
- Service-definition fixes, failed-activation file rollback, and portable
  deadline-based control I/O. Native macOS probing uncovered accepted-socket
  nonblocking behavior and large-reply/peer-close problems; regression tests exist.

## 1. Establish a native Linux baseline

Use a real Linux kernel on the target CPU architecture, preferably a disposable
Ubuntu 22.04 VM for the documented baseline. Docker-on-ARM evidence already
exists; it does not establish a native desktop/login/service/reboot gate.
Record distro, kernel, CPU architecture, libc, session type, and systemd version.
Do not silently substitute x86 emulation for x86 native support.

Required pins:

| Tool | Version |
| --- | --- |
| Rust | 1.96.1, rustfmt and clippy |
| Deno | 2.2.5 |
| Node | 22.14.0 |
| Playwright | 1.52.0, from package-lock.json |

Install the exact tools using trusted upstream distributions; retain their
checksums/provenance. Do not replace the system's other runtimes globally.
The HTTPS provider fixture also requires Python 3 and OpenSSL. Use a test user
with a working systemd user session:

```sh
systemctl --version
systemctl --user show-environment
loginctl show-user "$(id -un)" -p Linger -p State
```

If no user bus exists, establish a proper login/session or report the gate
blocked. Do not fake manager success with a shell supervisor. Do not enable
lingering or unattended operation without the owner's explicit choice.

## 2. Run all checks and retain exit statuses

From the repository root, with the pinned tool directories on PATH:

```bash
set -o pipefail
mkdir -p verification-artifacts/linux-native
{
  date -u
  uname -a
  cat /etc/os-release
  getconf GNU_LIBC_VERSION
  rustc --version
  cargo --version
  deno --version
  node --version
  git rev-parse HEAD
  git status --short
  git diff --binary | sha256sum
} > verification-artifacts/linux-native/environment.txt
npm ci
npx playwright install --with-deps chromium
CI=1 RUST_TEST_THREADS=1 npm run check 2>&1 |
  tee verification-artifacts/linux-native/check.log
printf 'complete check exit: %s\n' "${PIPESTATUS[0]}"
```

Save each status to a result file as well. `npm run check` includes formatting,
strict Clippy, Rust unit/integration checks, schema/types/management tests, tool
pin enforcement, and Chromium isolation. Serial Rust test execution prevents
unrelated ephemeral-port races; actual app concurrency remains covered.

Retain named test results for forced parent death (including blocked import and
blocked event loop), stale control ownership, blocked output, AI allow/deny and
authentication, origin/cookie/storage isolation, management authorization, and
IPv4/IPv6 listeners. A skipped test or missing browser is not a pass.

## 3. Workload and durable-state evidence

```bash
PARACO_BIN="$PWD/target/debug/paraco" python3 scripts/verify-foundation.py 2>&1 |
  tee verification-artifacts/linux-native/foundation.log
PARACO_BIN="$PWD/target/debug/paraco" python3 scripts/verify-prepared.py 2>&1 |
  tee verification-artifacts/linux-native/prepared.log
```

The foundation script creates isolated fixtures and checks:

- 1, 10, and 50 apps actually reach running state before measurements.
- Dashboard load, eight noisy-app requests, host/process-tree RSS, and management
  latency. Confirm noisy responses are 200, not an accidental 404 path.
- App A's cron draft is unavailable to app B.
- Stopped intent, deployment identity, draft source, and timezone survive SIGKILL
  and restart; the authenticated entry URL rotates.

The prepared script uses `examples/offline-dependency` (pinned HTTPS import),
fails both replacement and invalid new preparation, removes the source, relocates
the artifact, clears executable PATH, changes cwd, and launches the same artifact
twice. Both launches must serve `dependency-ok` and exit cleanly. This is not by
itself OS-level network isolation or a clean-machine proof.

Unit checks cover legacy import, constraint-failure rollback without partial
schema, newer-schema rejection, failed durable writes, private snapshots, restore,
and failed service-activation rollback. Add targeted tests for any new fixes.

## 4. Build and prove a clean offline bundle

```bash
npm run test:installer
native_target=$(rustc -vV | sed -n 's/^host: //p')
scripts/build-bundle.sh \
  --deno "$(command -v deno)" \
  --deno-notice /absolute/path/to/verified-deno-notice \
  --third-party-notice /absolute/path/to/verified-rust-notices \
  --target "$native_target" --output /absolute/new/bundle-output
```

For verification only, clearly marked fixture notices are acceptable; such an
archive must never be represented as distributable. Preserve archive checksum,
source revision/diff hash, tool versions, and target. The builder respects
`CARGO_TARGET_DIR`; use a new output directory.

Use `tests/bundle-smoke.sh` with the same trusted-input flags to exercise archive
installation, launcher/private-Deno resolution, HTTP, and shutdown. Retain its
output. Then perform the stronger clean-machine gate:

1. Prepare the HTTPS dependency app with the matching private Deno. Transfer only
   the archive and prepared artifact to a fresh compatible Linux VM/container.
2. Verify Rust/cargo, Node, and system Deno are absent. Do not mount the developer's
   home or caches. Record the clean image identity and libc.
3. Disable external networking at the VM/container level while retaining loopback.
   For a container, use `--network none`; do not merely say the app made no requests.
4. `tests/clean-offline.sh` can run inside the isolated machine with arguments
   for the installed launcher, prepared artifact, and hello-app paths. It checks
   two prepared launches and one ordinary bundled launch.
   Extract/install at a different path. Launch from `/` through a symlinked bundle
   launcher with an empty/unrelated PATH. Run `run-prepared` with the relocated
   artifact, then repeat. Check the expected response from inside the isolated
   machine and graceful shutdown. Also test ordinary bundled `run` to prove
   private bundle Deno resolution independently of artifact-contained Deno.
5. Attempt a failed preparation into a new revision and verify the previous
   installed revision remains usable. Preserve checksums before/after.
6. Record exit statuses, response, absence of tools, disabled-network setup, and
   cleanup. An unsigned archive is not a signed release.

## 5. Native systemd operation

Use an actual private-Deno bundle as `PARACO_BIN`, not a development binary that
finds Deno on your interactive PATH:

```bash
PARACO_BIN=/absolute/bundle/bin/paraco python3 scripts/verify-service.py 2>&1 |
  tee verification-artifacts/linux-native/systemd.log
```

This creates a unique temporary user unit and removes it afterward. It checks
startup, forced main-process death/restart, stopped intent and stable identity,
private browser-entry rotation, HTTP, and removal. Inspect the unit and journal
without retaining bearer tokens. Confirm all app/guardian processes and listeners
are gone after removal.

Also verify the public opt-in workflow in a dedicated test user's home:

```sh
/absolute/bundle/bin/paraco service setup --platform linux --config /absolute/server.json
/absolute/bundle/bin/paraco service status --platform linux --config /absolute/server.json
/absolute/bundle/bin/paraco open --print
/absolute/bundle/bin/paraco service remove --platform linux --config /absolute/server.json
```

Do not log the `open --print` output. Verify operation after the launching terminal
exits. Removal must preserve data. Update at the same canonical config/app paths
must preserve identity, data, and desired state. Use the documented explicit
remove/setup downtime workflow; test failed activation and restored service files.
Do not overwrite a real user's existing `paraco.service` registration.

## 6. Machine restart and backup restore (disposable VM)

This gate is not covered by `verify-service.py`, which cleans up before exit.
Create a persistent fixture outside `/tmp` and install the owned user service.
Write a distinctive cron draft/timezone to one storage-enabled app and stop a
second app. Record deployment IDs and expected values, not live bearer URLs.

Before reboot, export the stopped runtime state using:

```sh
paraco backup --state /absolute/runtime-state-directory --output /safe/new-backup.sqlite3
```

Start the service again, then reboot the explicitly authorized disposable VM.
After logging back in as the same user, verify the running/stopped intent, IDs,
draft/configuration, fresh authenticated browser entry, no duplicate/orphaned
children, and service-manager status. Document login-scoped behavior; do not claim
pre-login/unattended startup unless it was separately configured and tested.

Stop the service, restore the snapshot as `state.sqlite3` into a fresh private
state directory, using the same canonical config/source paths, and verify again.
Test invalid legacy migration and future-schema refusal on copies. Do not corrupt
or downgrade the only copy of real data. Retain configuration/artifacts and any
external grants separately because SQLite export does not include them.

## Deliverables and completion criteria

Update `docs/verification-2026-09-21.md` or add a dated follow-up with:

- Exact platform/architecture and source identification.
- Commands, logs, exit statuses, measured workload numbers, and fixture IDs.
- Native systemd, terminal-independence, shutdown, restart, and reboot observations.
- Clean offline dependency/archive proof and failed-update preservation.
- Backup/restore and migration-failure outcomes.
- Bugs fixed and their regression checks.
- Explicit remaining gates (especially any untested CPU architecture or macOS
  clean-machine/reboot requirement).

Clean up only your test units, containers, processes, and temporary data. Preserve
reviewable evidence and implementation changes. Do not declare all of M0–M3
complete while a native/reboot/clean-machine gate remains unobserved.
