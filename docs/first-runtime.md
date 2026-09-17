# First runtime slice

This implementation establishes the first executable Paraco boundary:

```text
paraco run APP  ->  validate paraco.json  ->  Rust-supervised Deno  ->  localhost HTTP
```

It is complete when `paraco run ./examples/hello` serves `Hello from Paraco`.

The command validates `name`, `entrypoint`, and `capabilities` before launching
Deno. It rejects malformed manifests, unknown schema fields, entry points that
are missing or resolve outside the app directory, and unsupported capabilities.
The subsequent AI increment supports `ai`; see [AI routing](ai-routing.md).

The bundled Deno adapter imports the entry point, requires a default export with
`fetch(request, context)`, and reports ready only from Deno's listener callback
after the loopback port is bound. Rust forwards child output, reports missing
Deno, startup failures, occupied ports, and unexpected child exits. Ctrl+C sends
SIGTERM on Unix. The Deno adapter responds by stopping its listener; Rust waits
up to five seconds, then kills and reaps a child that has not exited. Other
platforms use immediate child termination; graceful signal handling has only
been implemented and tested on Unix.

Acceptance checks:

- `cargo fmt --check`, `cargo test`, and `cargo build` pass.
- The example can be requested over `127.0.0.1` when Deno is installed.
- Invalid manifests fail before Deno launches.
- An app that cannot import or bind does not report ready and returns an error.
- Ctrl+C reaps the app process and releases its port.

## Automated verification

On Unix, run `cargo test` with Deno available on `PATH`. The integration suite
launches the actual CLI and Deno adapter using temporary app directories and
loopback ports. Missing Deno fails the suite explicitly; it does not silently
skip runtime coverage. `cargo test --bin paraco manifest::tests` runs only the manifest unit
tests and does not require Deno or network access.

The suite covers the repository hello example, stdout/stderr forwarding, invalid
manifests, missing Deno, invalid exports/imports, occupied ports, request failures
followed by successful requests, unexpected child exit, Ctrl+C cleanup, startup
timeout, and the five-second forced-shutdown fallback. Cleanup checks verify
that the Deno PID no longer exists and the app port can be bound again. Tests use
bounded waits and isolated process groups for cleanup even on assertion failure.
The timeout cases make the suite take approximately ten seconds.

These integration tests currently target Unix only because they verify Unix
signals and process cleanup. Windows runtime verification remains future work.
Run them in an environment that permits loopback listeners and subprocess signals.

Subsequent increments add [fake AI access](ai-routing.md) and
[foreground multi-app hosting with a dashboard](local-hosting.md). Deferred work
includes containers, real AI credentials/providers, persistent services,
background jobs, non-loopback networking, cloud worker adapters, databases,
installation, and lifecycle controls.
