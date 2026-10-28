# Development and verification

## Toolchains

The host is Rust 1.96.1 (`rust-toolchain.toml`). Development dependencies are
pinned in `package-lock.json`: Node.js 22.14.0 in CI, workerd 1.20261026.1
(reporting `2026-10-26`), esbuild 0.28.2, and Playwright 1.64.1. From the
repository root:

```sh
npm ci
npx playwright install chromium
PATH="$PWD/node_modules/.bin:$PATH" cargo build --locked
npm run check
```

`npm run check:rust` runs formatting, strict Clippy for every target, and Rust
unit tests. Node-based core checks require `npm ci`; browser checks also require
Playwright Chromium. The local HTTPS provider fixture uses Python 3 and OpenSSL
to serve a temporary trusted test certificate; it contacts no external provider.
On macOS, use `TMPDIR=/private/tmp npm run test:browser` if the system temporary
directory exceeds the Unix control-socket path limit.

`npm run check` runs core checks, integration tests, and browser smoke tests.
The template test runs with Node's type stripping. Linux app execution requires
`/usr/bin/bwrap` with unprivileged user and network namespaces. macOS currently
fails closed for normal app execution because secure containment is not
implemented. `PARACO_ALLOW_UNSANDBOXED=1` is for trusted development code only.
Linux cgroup memory quotas and macOS sandbox verification remain release gates.

## CI evidence

`.github/workflows/ci.yml` configures checks on Ubuntu 22.04 and macOS 14.
Configuration is not observed evidence. Successful native runs must be retained
for each supported OS; macOS sandbox verification is still pending.

## Current evidence

See [current status](status.md) for observed checks and remaining platform
validation. Historical audits and milestone plans retain their original context;
they are not current verification records.

## Expanded foundation verification

Run `scripts/verify-foundation.py` with the npm-installed workerd and esbuild
binaries on PATH after building. It waits for apps at 1/10/50 scale, measures
host and process-tree RSS, exercises noisy output, checks management latency,
and verifies scoped storage, private browser entry, and desired-state
restoration after SIGKILL. Set `PARACO_BIN` for a non-default build location.
Use `scripts/verify-prepared.py` for relocated prepared-app launches and
failed-preparation preservation.

Tests and browser scripts use isolated data directories, separate from their log
directories, to avoid touching the developer’s state. The dashboard management mock tests use fixtures; browser smoke tests also
exercise a running host.

The complete checks use `RUST_TEST_THREADS=1` in CI to avoid ephemeral-port reuse
between unrelated integration fixtures. Application concurrency is exercised by
the multi-app suites and workload probe. `scripts/verify-service.py` operates a
uniquely named temporary native service and requires a real systemd user session
or macOS GUI login session.

On Linux, `PARACO_BIN=/absolute/bundle/bin/paraco python3 scripts/verify-linux-durable.py`
checks service survival after a launching pseudo-terminal exits, source updates,
backup restoration into fresh private state, invalid legacy/future-schema refusal,
and cgroup/listener cleanup. It uses a unique temporary systemd unit and requires
`script` from util-linux. It does not exercise machine reboot or the public
`paraco.service` installation workflow. See the
[native evidence](verification-linux-native-2026-09-21.md) for exact observations.

## Cron and release checks

The complete check also runs the cron public-API integration and real Chromium
workflow. Run `npm run test:cron-runtime`, `npm run test:cron-ui`, and
`npm run test:cron-browser` independently during development. Cron unit tests
accept deterministic clock inputs to test DST, downtime, archival, and crash state.

`npm run test:bundle-contract` verifies archive composition with stub workerd and
esbuild executables; it does not claim native compiled-runtime validation.
`npm run test:installer` exercises malformed archives, pointer ownership,
upgrade, rollback, and removal. The manual release-validation workflow runs
native archive checks across its configured matrix and retains evidence. Fixture
notices make those test archives non-distributable; successful CI is not signing
or publication.

Dependency verification uses `npm audit --audit-level=low` and
`cargo audit --deny warnings` (cargo-audit 0.22.2). CI checks committed lockfiles
against current advisories. An audit is time-specific evidence, not a guarantee
of absence of vulnerabilities.

Runner labels in CI use the [GitHub hosted-runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
Configured architecture coverage is only a claim about intended verification;
successful native evidence must be retained separately.
