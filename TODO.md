# Paraco TODO

Keep new follow-up work here. Check an item only when its validation evidence is
recorded. Runtime implementation details and observed checks are in
[workerd migration](docs/workerd-migration.md).

## Runtime migration release gates

- [ ] Implement and verify macOS containment. Native untrusted execution currently
  fails closed; `PARACO_ALLOW_UNSANDBOXED=1` is only for trusted development.
  An opt-in native Seatbelt prototype and executable escape probes are now
  available; real macOS validation remains required. See
  [native containment](docs/native-containment.md).
- [ ] Enforce and test hard per-app memory/CPU/process limits using delegated
  cgroups on Linux and an equivalent macOS/VM policy. Task wall deadlines,
  concurrency and descriptor limits do not replace memory containment.
- [ ] Audit the namespace, compiler and capability boundaries against hostile
  workloads, including resource exhaustion and runtime escape scenarios.
- [ ] Run native macOS and supported Linux architecture installation/recovery
  checks. Record exact minimum OS/libc versions and redistribution notices.
- [ ] Verify reproducible native release builds, signing and provenance before
  publishing the first installer; no public release has been published.

## Deferred Cloudflare verification

- [ ] Choose a Cloudflare account and test Worker, then deploy the representative
  `examples/cloudflare` application and verify its response using the same core
  logic as the local app. **Deferred at the user's request.** The local Workers
  compatibility test is available as `npm run test:cloudflare-contract`.
- [ ] Implement deployment-owned Cloudflare capability services (including D1)
  behind the `PARACO` service binding and test permission failures and storage
  semantics. The checked-in adapter defines a transport, not a complete backend.
- [ ] Define cloud task duration/background-work limits and deployment checks;
  do not promise local one-hour execution semantics on cloud targets.

## Platform follow-ups

- [ ] Publish a new workerd-compatible Hello World app release and review its
  catalog version, commit and digest. Keep the existing published approval intact.
- [ ] Add deployment-target/capability compatibility metadata as cloud backends
  become available; fail clearly for unsupported capabilities.
- [ ] Add the shared storage/artifact registry through the scoped capability
  boundary, with documented concurrency and installation identities.
- [ ] Add Kubernetes packaging; evaluate Lambda separately without changing the
  portable application handler.

## Completed foundation

- [x] Replace app/task execution with workerd and preserve Rust lifecycle/state.
- [x] Bundle the native compiler and runtime; remove Deno as a tool dependency.
- [x] Add Linux namespace containment and fail closed on unsupported hosts.
- [x] Test scoped capabilities, task networking/redirect denial, persistence,
  task recovery and offline prepared packages.
- [x] Add a portable Cloudflare entry point and a local compatibility check.

Previous milestone history is retained in
[the pre-workerd checklist](docs/todo-before-workerd.md).
