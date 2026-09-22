# Development and verification

## Pinned toolchains

The repository uses Rust 1.96.1 (`rust-toolchain.toml`), Deno 2.2.5, Node.js
22.14.0, and Playwright 1.52.0. The latter three pins are enforced by the native
CI workflow. Install Rust with rustup and Deno from its official installer, then:

```sh
npm ci
npx playwright install chromium
npm run check
```

`npm run check:rust` runs formatting, strict Clippy for every target, and Rust
unit tests without requiring Deno, Node pins, or Chromium. The local HTTPS
provider fixture uses Python 3 and OpenSSL to serve a temporary trusted test
certificate; it contacts no external provider. `npm run check:core`
also runs schema, TypeScript declarations, and management DOM checks. The
Node-based core checks require `npm ci`. On macOS, use
`TMPDIR=/private/tmp npm run test:browser` if the system temporary directory
exceeds the Unix control-socket path limit. Linux archive checks run separately
with `npm run test:installer` and `tests/bundle-smoke.sh` (see its required
trusted-input arguments); CI runs these on Linux.

`npm run check` runs those independent checks first, then verifies prerequisites
and runs every integration test and the browser smoke test. Full verification
requires Deno 2.2.5 and Playwright Chromium. Local Node versions may be 22 or
newer; CI enforces exactly 22.14.0. `cargo test` launches real Deno applications,
so absent Deno is a failure, not a skipped success.
`browser-smoke.cjs` builds on the management DOM test with a real headless
Chromium session: it opens the private management URL, observes the app, requests
a restart, and opens retained logs. It uses a temporary app, log directory, and
control directory, and its process cleanup is time-bounded.
Rust integration tests also isolate the Unix control-socket root through
`TMPDIR`, so parallel suites do not share an ambient endpoint namespace.

## CI evidence

`.github/workflows/ci.yml` configures the above checks on Ubuntu 22.04 and
macOS 13. Configuration is not observed evidence. Until successful native runs
are retained for both OS families, the M0 native-support acceptance gate remains
pending.

## Current evidence

See [current status](status.md) for observed checks and remaining platform
validation. Historical audits and milestone plans retain their original context;
they are not current verification records.

## Expanded foundation verification

Run `scripts/verify-foundation.py` with pinned Node/Deno on PATH after building.
It waits for every app at 1/10/50 scale, measures host and process-tree RSS,
exercises noisy output, checks management latency, and verifies scoped storage,
private browser entry, and desired-state restoration after SIGKILL. Set
`PARACO_BIN` for a non-default build location. Use `scripts/verify-prepared.py`
for two relocated dependency-bearing launches and failed-preparation preservation.

The complete checks use `RUST_TEST_THREADS=1` in CI to avoid ephemeral-port reuse
between unrelated integration fixtures. Application concurrency is exercised by
the multi-app suites and workload probe. `scripts/verify-service.py` operates a
uniquely named temporary native service; use a private-Deno bundle as `PARACO_BIN`.
It requires a real systemd user session or macOS GUI login session.
