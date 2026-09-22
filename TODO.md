# Paraco development TODO

Work in small increments and check items off only after implementation and
verification. Cloud work remains deferred.

## M0–M3 follow-up

See [current verification evidence](docs/verification-2026-09-21.md) and the
[Linux-agent handoff](docs/linux-agent-handoff.md). SQLite migration, scoped
cron-draft storage, backup export, authenticated `open`, and relocatable private
Deno artifacts are implemented with regression checks. Historical pending labels
below are superseded only where that evidence explicitly records a pass. Native
Linux systemd, clean macOS, and machine reboot gates remain separate acceptance
work; scheduling is not implemented.

## Proposed foundation pivot

See [the foundation and cron plan](docs/foundation-and-cron-plan.md) for the
proposed next milestone order, confirmed product decisions, and acceptance gates.
It replaces the previous proposal to defer all bundling/platform verification
until after capabilities. The unchecked sections below remain a backlog, not an
execution order; completed work remains recorded as historical milestones.

1. Establish Linux/macOS CI, pinned tools, and versioned contracts (M0).
2. Harden lifecycle recovery and isolate application browser origins (M1).
3. Prepare reproducible apps and test minimal private-Deno bundles (M2).
4. Add durable identity/state and native service operation (M3).
5. Add bounded tasks, scheduling, history, and notifications (M4–M5).
6. Deliver the manual cron app, then AI drafting with explicit publication (M6–M7).
7. Finish installation and release verification (M8).

The detailed plan defines the acceptance gates; partial implementation progress
is recorded below without declaring an entire milestone complete.

## M1 crash recovery — Linux/macOS ARM64 evidence retained

- [x] Add per-app Rust guardians that terminate and reap Deno after runtime death,
      including blocked JavaScript and incomplete startup.
- [x] Retain endpoint ownership until guardians finish cleanup, then safely
      recover verified stale sockets without replacing live endpoints.
- [x] Verify immediate multi-app restart, temporary-directory cleanup, and
      protection of unexpected endpoint files on Linux.
- [x] Retain successful native macOS ARM64 crash-recovery test results.
- [x] Implement browser-origin isolation, output-loss reporting, and bounded
      workload admission; retain environment-specific measurement evidence before
      claiming the workload gate observed.

## Foundations

- [x] Validate manifests and run one TypeScript HTTP app with `paraco run`.
- [x] Verify startup errors, request failures, logs, and bounded process cleanup.
- [x] Implement offline AI routing and grant enforcement with a fake provider.
- [x] Connect `context.ai.complete` through authenticated local transport.
- [x] Unit-test AI policy, authentication framing, and cleanup behavior.
- [x] Retain pinned-Deno end-to-end evidence for AI calls, denied grants,
      authentication, and cleanup on Linux/macOS ARM64.

## Foreground multi-app hosting — completed

- [x] Add `paraco serve` with explicit local configuration listing app directories.
- [x] Supervise multiple Deno apps independently behind one loopback gateway.
- [x] Route `/apps/<name>/` and define app request paths and `context.basePath`.
- [x] Verify relative assets, queries, request bodies, and redirects under app paths.
- [x] Serve a read-only dashboard at `/` with names, links, and app status.
- [x] Keep healthy apps and the dashboard available when another app fails.
- [x] Shut down and reap all app processes and release listeners on exit.
- [x] Add a runnable multi-app example and document the configuration and limits.
- [x] Pass existing checks and new multi-app integration tests.

## Local lifecycle management — completed

- [x] Add start, stop, and restart operations through a private Unix control socket.
- [x] Expose status and lifecycle controls through the CLI.
- [x] Expose lifecycle controls through a separately protected dashboard management origin.
- [x] Persist bounded JSONL logs with configurable location, rotation, and CLI app filters.
- [x] Add log viewing to the authenticated dashboard.
- [x] Track in-memory desired state separately from observed process state.
- [x] Add bounded restart attempts and clear failure reporting.
- [x] Verify manual failure recovery, startup cancellation, process and capability
      cleanup, and that deliberately stopped apps stay stopped during the session.
- [x] Verify automatic recovery, retry exhaustion, stop cancellation, and app isolation.

## Background service operation

- [x] Generate explicit, user-scoped systemd/launchd definitions and document
      opt-in setup/removal/status behavior.
- [ ] Retain observed native manager, terminal-independence, reboot, and graceful
      shutdown evidence (generated definitions are not native verification).
- [ ] Retain restart/machine-restart restoration evidence on supported targets.

## Further local capabilities

- [x] Implement the documented bounded loopback OpenAI-compatible HTTP subset.
- [x] Implement the bounded provider/HTTP SSE streaming foundation.
- [x] Add a real-provider adapter and host-owned credential-file handling.
- [x] Persist host deployment identity and desired state.
- [x] Add deployment-scoped SQLite configuration/data storage for cron drafts,
      with backup/export and migration/restore checks.
- [x] Define deferred bounded scheduling, notifications, and health increments.
- [ ] Verify Deno SDK streaming, provider saturation/cancellation, and native
      end-to-end HTTP behavior with deterministic fixtures.

## Bundling and installation

- [x] Define local unsigned private-Deno bundle, archive installer, and generated
      unpublished Homebrew/Debian/macOS-package recipe tooling.
- [x] Test hostile archive-path rejection and local installer pointer/data safety.
- [ ] Verify actual archives on clean machines, offline dependencies, HTTP, and
      shutdown across supported native targets.
- [ ] Execute authenticity, dependency review, signing/notarization, publication,
      native package, service, upgrade/rollback, and health-recovery gates.

See `docs/installation-and-release.md` for the detailed release plan.

## Cloud — deferred

- [ ] Revisit public control protocols, cloud connectivity, and remote deployment
      only after the standalone local foundation is useful and verified.

## M0/M1 verification status (2026-09 follow-up)

See [retained evidence](docs/verification-2026-09-21.md) for commands, logs,
source fingerprints, failures repaired, and platform limitations.

| Gate | Status | Evidence / required follow-up |
| --- | --- | --- |
| Pinned complete checks | observed pass | Rust 1.96.1, Deno 2.2.5, Node 22.14.0, Playwright 1.52.0 on macOS ARM64 and Linux ARM64 container. |
| Browser isolation, management, AI authorization | observed pass | Complete Rust/Chromium suites; see retained platform logs. |
| Guardian/crash recovery | observed pass on tested targets | Includes blocked import/event loop and immediate restart. |
| 1/10/50 workload | observed pass | Readiness, noisy output, 72 successful requests per workload, management latency, host/tree RSS recorded. |
| Native service | launchd observed; systemd pending | Linux agent handoff covers actual systemd session, terminal independence, and reboot. |
| Clean offline bundle | Linux observed; clean macOS pending | Linux network-disabled container, no SDK tools, relocated artifact, symlinked private-Deno launcher. |
| Machine restart / other targets | pending | No active Mac reboot or unobserved architecture/browser support claim. |
