# First runtime slice

This implementation establishes the first executable Paraco boundary:

```text
paraco run APP  ->  validate paraco.json  ->  Rust-supervised Deno  ->  localhost HTTP
```

It is complete when `paraco run ./examples/hello` serves `Hello from Paraco`.

The command validates `name`, `entrypoint`, and `capabilities` before launching
Deno. It rejects malformed manifests, unknown schema fields, entry points that
are missing or resolve outside the app directory, and every requested capability
because no capabilities are implemented yet.

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

Deferred work includes containers, AI access and credential proxying, persistent
services, background jobs, non-loopback networking, cloud worker adapters,
databases, installation, and management interfaces.
