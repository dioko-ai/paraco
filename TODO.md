# Paraco development TODO

Work in small increments and check items off only after implementation and
verification. Cloud work is deferred; bundling and installation come last.

## Foundations

- [x] Validate manifests and run one TypeScript HTTP app with `paraco run`.
- [x] Verify startup errors, request failures, logs, and bounded process cleanup.
- [x] Implement offline AI routing and grant enforcement with a fake provider.
- [x] Connect `context.ai.complete` through authenticated local transport.
- [x] Verify AI calls, denied grants, authentication, and cleanup end to end.

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

## Local lifecycle management — next

- [ ] Add start, stop, and restart operations through a local control interface.
- [ ] Expose lifecycle controls through the CLI and dashboard.
- [ ] Add bounded per-app log access.
- [ ] Track desired state separately from observed process state.
- [ ] Add bounded restart attempts and clear failure reporting.
- [ ] Verify failure recovery and that deliberately stopped apps stay stopped.

## Background service operation

- [ ] Run the foreground server under the Linux service manager.
- [ ] Verify terminal independence and graceful service shutdown.
- [ ] Restore configured apps after service restart and machine restart.
- [ ] Document development service setup; integrate it with installers later.
- [ ] Add and verify service support for other target platforms later.

## Further local capabilities

- [ ] Add the documented OpenAI-compatible HTTP subset.
- [ ] Add and verify streaming separately.
- [ ] Add a real AI provider and host-owned secret storage.
- [ ] Add local application configuration and persistence.
- [ ] Define subsequent scheduling, notifications, and health increments.

## Bundling and installation — last

- [ ] Define and build the pinned private Deno release bundle.
- [ ] Verify clean-machine execution and supported platform targets.
- [ ] Add portable archives, then the shell installer and Homebrew channel.
- [ ] Verify upgrade, rollback, removal, and preservation of application data.
- [ ] Add signing, native packages, and optional service installation.

See `docs/installation-and-release.md` for the detailed release plan.

## Cloud — deferred

- [ ] Revisit public control protocols, cloud connectivity, and remote deployment
      only after the standalone local foundation is useful and verified.

## Latest verification

Foreground hosting: 41 tests passed on Linux with Deno 2.9.7, including six
multi-app integration tests. Rust formatting, Clippy with warnings denied, and
Deno type checks passed. The integration tests require local networking and
subprocess signals. This environment uses `/tmp/paraco-deno/bin` on `PATH`.
