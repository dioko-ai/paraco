# Paraco development TODO

Work in small increments and check items off only after implementation and
verification. Cloud work remains deferred.

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

## M1 crash recovery — implemented, native macOS verification pending

- [x] Add per-app Rust guardians that terminate and reap Deno after runtime death,
      including blocked JavaScript and incomplete startup.
- [x] Retain endpoint ownership until guardians finish cleanup, then safely
      recover verified stale sockets without replacing live endpoints.
- [x] Verify immediate multi-app restart, temporary-directory cleanup, and
      protection of unexpected endpoint files on Linux.
- [ ] Retain successful native macOS crash-recovery test results.
- [x] Implement browser-origin isolation, output-loss reporting, and bounded
      workload admission; retain environment-specific measurement evidence before
      claiming the workload gate observed.

## Foundations

- [x] Validate manifests and run one TypeScript HTTP app with `paraco run`.
- [x] Verify startup errors, request failures, logs, and bounded process cleanup.
- [x] Implement offline AI routing and grant enforcement with a fake provider.
- [x] Connect `context.ai.complete` through authenticated local transport.
- [x] Unit-test AI policy, authentication framing, and cleanup behavior.
- [ ] Retain pinned-Deno end-to-end evidence for AI calls, denied grants,
      authentication, and cleanup (blocked locally: Deno is unavailable).

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
- [ ] Add app storage only with a concrete app-facing use case; unused storage
      scaffolding was removed during the 2026-09-21 audit repair.
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

## M0/M1 verification status (2026-09)

| Gate | Status | Evidence / required follow-up |
| --- | --- | --- |
| Rust format, check, server-test compilation | observed pass | Linux x86_64, Rust 1.96.1; run `cargo fmt --check && cargo check && cargo test --test server --no-run`. |
| Pinned complete check | pending | `npm run check` correctly fails locally before suites: Deno 2.2.5 and Playwright Chromium are absent; local Node is 22.22.3, not 22.14.0. |
| Browser isolation and management | configured-only | CI installs Chromium and runs `node tests/browser-smoke.cjs`; retain a successful native artifact before claiming observed browser support. |
| Guardian/crash recovery | pending native macOS | Run the Rust suite on macOS 13 with Deno 2.2.5; CI configuration is not evidence. |
| Output-loss and admission regressions | implemented / compile-checked | Included in Rust tests; execute `cargo test` with pinned Deno to observe launch and blocked-output paths. |
| 1/10/50 workload budget | pending observation | Run `PARACO_BIN=... node scripts/probe-hosted-workload.cjs one.json ten.json fifty.json` on the target host; retain its JSON, including optional noisy endpoint results. |
| Other architectures and browsers | pending | No emulation or YAML configuration closes native architecture/browser gates. |
