# Verification evidence — 2026-10-25

The final Linux x86-64 working tree passed the complete `npm run check` command.
This is uncommitted source, not a published release. Retained native reports record
source hashes and the dirty source revision so the evidence is attributable.

## Observed passes

- Formatting, strict Clippy, schema and TypeScript checks.
- 146 Rust tests: 10 library, 69 binary, 14 AI, 1 logs, 18 runtime and 34 server.
- Four management JavaScript tests, management Chromium smoke, and all six Cron
  UI/adapter/browser tests, including the real browser publication lifecycle.
- Real task execution: publication conflicts, permission boundaries,
  success/error/timeout/cancellation, overlap prevention, archive/restart,
  notifications, one-off deduplication, database-only backup restoration, and
  durable live-log prefixes after SIGKILL.
- Installer, bundle-contract and synthetic signed-release verification tests.
  These use fixtures and ephemeral signing keys, not real publisher credentials.
- Dependency review: npm reported zero vulnerabilities; cargo-audit 0.22.2
  reported zero vulnerabilities and warnings after dependency updates.
- Deno 2.9.8 and Playwright 1.64.1 were used. The browser check initially found
  a Deno syntax-error reporting regression and a live-log test race; both were
  corrected before the final full command passed.
- Foundation workloads with 1, 10 and 50 apps returned 200 for every request,
  including noisy-output pressure. Storage isolation, SIGKILL/restart recovery,
  stopped intent, stable identity and private management URL rotation passed.
- Prepared-artifact relocation ran twice with `PATH=/nonexistent` and CWD `/`;
  failed preparation preserved the installed revision and shutdown passed.
- The final native bundle served HTTP and ran embedded Cron outside the checkout
  with a deliberately broken system Deno. Draft/publish/run/logging, useful syntax
  errors and shutdown passed using the bundled runtime.
- Temporary Linux systemd units passed startup, unexpected-death restart,
  terminal-session exit, source update, stable identity/data/config/stopped intent,
  backup restoration and invalid legacy/future-schema refusal. Nine cgroup
  processes were reaped across three stops; listeners and test units were removed.

## Retained evidence

See [the full check log](../verification-artifacts/2026-10-25/check.log),
[workload and recovery measurements](../verification-artifacts/2026-10-25/foundation.log),
[prepared-artifact results](../verification-artifacts/2026-10-25/prepared.log),
[dependency audit summary](../verification-artifacts/2026-10-25/dependency-audit.json),
and [native bundle/service evidence](../verification-artifacts/2026-10-25/native-release.json).
The native record includes source, executable and archive hashes plus the exact
private runtime version. Its archive uses incomplete fixture notices and must
not be distributed.

## Remaining release gates and product limits

- Native macOS and ARM64 validation, minimum OS/libc baselines, machine reboot,
  start-at-login and public service setup/removal in a dedicated test account.
- Real redistribution notices, production signing keys, authenticated release
  provenance, applicable notarization, publication and clean-machine installation.
- Tasks currently accept self-contained TypeScript modules. Task imports,
  task-level AI/configuration/data APIs and native desktop notification delivery
  are outside this first adapter; notifications persist in the local inbox.

The September records remain historical evidence for their exact targets. This
record does not extend those observations to the current release or claim
network-isolated clean-machine testing.
