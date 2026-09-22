# M0–M3 verification: 2026-09-21

This records observed checks for the working-tree implementation, not a release
or blanket completion of every milestone. Evidence is retained under
[`verification-artifacts`](../verification-artifacts/). Source/tool fingerprints
and command statuses accompany the logs. The owner requested a
[Linux-agent handoff](linux-agent-handoff.md) for the remaining native Linux work.

## Environments and complete checks

- macOS ARM64: Darwin 25.3.0, actual Mac host.
- Linux ARM64: native ARM64 Linux kernel in Docker Desktop's VM, Debian container.
  This is not x86 emulation, but does not establish a native desktop/user-service
  session or machine-reboot behavior.
- Both use Rust 1.96.1, Deno 2.2.5, Node 22.14.0, and Playwright 1.52.0 Chromium.
- Complete `CI=1 RUST_TEST_THREADS=1 npm run check` passes: formatting, strict
  Clippy, 119 distinct Rust tests (some repeated by the core/full scripts), schema,
  declaration checks, management DOM tests, and real Chromium checks.
- Actual launch/request/shutdown, crash cleanup, blocked app/event-loop cleanup,
  stale endpoint protection, AI allow/deny/authentication, browser isolation, and
  authenticated management are exercised by those suites.

See `macos-check.log` and the complete-check section of `linux-final.log`.
The earlier `linux-check.log` is also retained, but predates the final cache fix.
No other OS/CPU/browser combination is established by these runs.

## Workloads

Each fixture waits for all apps, performs 64 dashboard requests and eight app
requests producing approximately 16 MiB of output, then queries management.
All 72 requests return 200 and all apps remain running. Memory is observed RSS,
not a quota or minimum-system requirement. Process-tree totals sum RSS and can
double-count shared pages. Measurements use debug host builds.

| Platform | Apps | Startup ms | Management ms | Host RSS after load KiB | Tree RSS after load KiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| macOS ARM64 | 1 | 307 | 15 | 20,848 | 86,176 |
| macOS ARM64 | 10 | 208 | 17 | 24,768 | 621,280 |
| macOS ARM64 | 50 | 829 | 14 | 26,384 | 2,647,760 |
| Linux ARM64 container | 1 | 175 | 13 | 17,632 | 95,700 |
| Linux ARM64 container | 10 | 333 | 10 | 20,828 | 692,376 |
| Linux ARM64 container | 50 | 1,344 | 10 | 25,664 | 3,356,124 |

See `macos-foundation.log` and `linux-final.log`. These are individual observations,
not statistically established latency budgets. Fifty Deno applications consume
substantial aggregate memory even though the Rust host stays small.

## Prepared artifacts and bundles

The HTTPS dependency example is prepared, its source is removed, and its artifact
is moved. It serves `dependency-ok` twice from `/` with PATH set to `/nonexistent`.
Invalid new preparation and attempts to overwrite the installed artifact fail
without changing the installed metadata. Both launches shut down cleanly.

This uncovered two defects now fixed: an absolute preparation-machine Deno path,
and Deno's execution-time cache writes invalidating an immutable artifact. Format 2
contains a digest-verified private Deno and uses disposable per-launch cache copies
with cached-only execution and frozen locks.

Linux clean-container evidence uses Debian trixie-slim with `--network none` and
no Rust, cargo, Node, or system Deno. The dependency response and clean exit were
observed. The final installed archive passes two prepared launches and one ordinary bundled
hello launch through a symlink, with empty PATH, cwd `/`, and read-only artifact
mounts. Results are in `linux-clean-final.log`; the initial proof is retained
separately as `linux-clean-offline.log`. macOS has relocated/empty-PATH proof and an actual
private-Deno archive/service run, but is not a clean-machine or OS-isolated-network
verification. Archive notices used for these tests are explicitly non-distributable
fixtures; no signing, publication, or authenticity claim is made.

## Durable state and service operation

SQLite regression tests pass for format-2 JSON migration, stable identities and
stopped intent, transactional constraint-failure rollback, future-schema refusal,
failed writes, scoped storage, backup/export and restore. State directories and
backup files are private. `examples/cron-drafts` provides the concrete source/
timezone storage use case; no scheduling or publication API is implied.

Both platforms demonstrate cross-deployment storage isolation and retained draft,
timezone, identity, and stopped intent after SIGKILL/restart. The private `open`
entry URL changes on restart. Credential-bearing URLs are not retained in evidence.

Native launchd passes using a private-Deno bundle and a temporary unique service:
startup, forced-death restart, stopped-state/identity restoration, entry rotation,
HTTP, and graceful removal. See `macos-service.log`. Unit tests cover rollback of
launcher/definition files after activation failure. Generated systemd definitions
are not yet an observed native systemd pass.

## Failures found and remaining gates

Initial runs exposed macOS accepted-socket nonblocking behavior and errors when
changing receive timeouts after a peer closed with a large reply still buffered.
Control I/O now uses absolute-deadline polling; a multi-chunk reply regression
passes. Parallel integration fixtures also collided on ephemeral ports; complete
checks run serially, while workload tests explicitly exercise concurrent apps.

Verification fixtures were corrected to wait for all apps, use a real app Host
header for noisy requests, and allow the runtime's documented cleanup deadline.
A failed workload attempt is retained in `macos-workload-attempt.log`; earlier
failed/recovery logs are historical diagnostics, not final acceptance results.

Remaining acceptance work:

- Native Linux systemd user-session operation, terminal independence, and reboot
  restoration: assigned to the Linux agent via the handoff.
- Actual machine reboot/restoration on macOS and a clean macOS dependency/bundle
  test without developer tooling. The active Mac was not rebooted.
- Additional CPU architectures, minimum OS/libc targets, and other browsers.
- Release signing, complete third-party notices, provenance, and publication.

M0–M3 must not be marked universally complete until the relevant native,
clean-machine, and reboot gates have retained evidence. Scheduling remains deferred.
