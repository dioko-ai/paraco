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

`npm run check` is the authoritative local and CI verification entry point. It
fails before testing if Deno 2.2.5, Node 22, npm dependencies, or Playwright's
Chromium setup are unavailable; `cargo test` launches real Deno applications
and therefore cannot be treated as a skipped success when Deno is absent.
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

## M0/M1 evidence matrix

| Gate | Current status | Command or artifact |
| --- | --- | --- |
| Linux static Rust checks | observed pass | `cargo fmt --check`, `cargo check`, and `cargo test --test server --no-run` on Linux x86_64/Rust 1.96.1. |
| Complete pinned Linux check | pending | `npm run check`; this machine deliberately fails preflight because Deno 2.2.5/Chromium are absent and Node is 22.22.3. |
| Browser origin and management tests | configured-only | CI artifact `verification-<OS>-<arch>` from `node tests/browser-smoke.cjs`; a retained passing native artifact is required. |
| macOS guardian/crash recovery | pending | Native macOS 13 `npm run check` artifact; Linux or emulation does not substitute. |
| Output-loss and admission paths | implemented / compile-checked | Run `cargo test` under the pinned Deno toolchain to observe subprocess paths. |
| Workload measurements | pending | Run the 1/10/50 probe in `scripts/probe-hosted-workload.cjs` and retain its JSON; do not invent budgets. |
| Other browsers/architectures | pending | Record native command, OS/browser identity, and retained result. |
