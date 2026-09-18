# paraco

Paraco is a local-first runtime for portable applications. This repository is
building its standalone foundation: Rust supervises Deno applications and serves
them through a local gateway, with a separate authenticated management dashboard.

## Try it

Prerequisites are a current [Rust toolchain](https://rustup.rs/) and
[Deno](https://docs.deno.com/runtime/) available on `PATH`.

```sh
cargo run -- run ./examples/hello
curl http://127.0.0.1:3000
```

The response is `Hello from Paraco`. Use `--port` to select another loopback
port, for example `cargo run -- run ./examples/hello --port 8787`. Press Ctrl+C
to stop the application; Paraco sends it a graceful termination signal, then
forces termination after five seconds if needed.

## Host multiple apps

```sh
cargo run -- serve --config ./examples/server.json
```

Open <http://127.0.0.1:3000/> for the dashboard. The example hosts:

- Hello and Fake AI at the distinct canonical `.localhost` URLs shown by the
  dashboard (the old `/apps/<name>/` links redirect only for GET/HEAD).

Use `--port 8787` for another loopback port. This runs in the foreground;
Ctrl+C stops all apps. Configuration paths resolve relative to the configuration
file, and the manifest name determines each app's URL. The dashboard refreshes
app statuses every five seconds. To start, stop, or restart apps in the browser,
open the separate `management dashboard` URL printed in the terminal. That page
refreshes status every two seconds; its private access link changes each server session.

See [local hosting](docs/local-hosting.md) for the configuration, app base-path
contract, and limits. [TODO.md](TODO.md) tracks completed work and the next
increment: background service operation.

## Manage running apps

On Unix, use another terminal to control apps hosted by `paraco serve`:

```sh
cargo run -- status
cargo run -- stop hello
cargo run -- start hello
cargo run -- restart hello
```

Commands print JSON status and acknowledge lifecycle requests immediately. Use
`status hello` to check completion; add the server's `--port` if it differs from
3000. CLI management uses a private local socket. Browser controls use the separate
authorized management dashboard; the gateway dashboard stays read-only.
See [local lifecycle](docs/local-lifecycle.md) for state, security, and recovery
semantics. Desired state is currently kept only for the running server session.
On Unix, app guardians clean up Deno after a runtime crash, and restarting `serve`
waits for cleanup before safely recovering its stale control socket.

## Persistent logs

Application output and lifecycle events are retained as agent-readable JSONL in
`~/.paraco/logs` by default (`%USERPROFILE%\.paraco\logs` on Windows). Override
with `--log-dir <path>` or `PARACO_LOG_DIR`. A rotating five-file store caps retained
log data at 10 MiB per directory, shared across apps and server sessions.

```sh
cargo run -- logs hello --tail 100
cargo run -- logs hello --port 3000 --tail 500
```

These commands work even after the server stops. Records identify the app,
launch, process, stream, and capture time. See [local logs](docs/local-logs.md)
for configuration, schema, pruning, dashboard viewing, and failure behavior.

## Application contract

An app directory contains `paraco.json` and its source. The currently supported
manifest schema is intentionally explicit:

```json
{
  "name": "hello",
  "entrypoint": "main.ts",
  "capabilities": []
}
```

`schemaVersion` may be omitted for the existing v1 manifest or set to `1`.
`name` is 1–63 lowercase ASCII letters, digits, or hyphens and starts with a
letter. `entrypoint` must be a relative file contained in the application
directory. `capabilities` is required and may be empty or contain `"ai"` for the
local fake AI capability. Unknown fields and unsupported capabilities fail
validation. The machine-readable draft schema is
[`docs/paraco-manifest.schema.json`](docs/paraco-manifest.schema.json), and the
matching handler types are in [`runtime/paraco.d.ts`](runtime/paraco.d.ts).

The entrypoint exports a default object whose `fetch` method receives web
standard `Request` and returns a `Response` (or a promise for one):

```ts
export default {
  fetch(_request: Request, _context: object): Response {
    return new Response("Hello from Paraco");
  },
};
```

The Deno adapter calls `Deno.serve` and binds only `127.0.0.1`. The Rust binary
embeds the adapter, so `paraco` does not rely on the shell working directory to
find it. It grants Deno read access to the app directory and network access only
to its selected loopback listener and, for AI-enabled apps, the capability port.
It clears inherited environment variables
apart from `PATH`, and supplies a temporary `DENO_DIR`; no ambient credential
environment variables are passed to apps. These permissions are only the narrow
ones needed for this example and are not a complete security sandbox for
untrusted code.

## Verify

```sh
cargo fmt --check
cargo test
node --test tests/management.test.cjs
cargo run -- run ./examples/hello
```

The dashboard JavaScript tests use Node.js (no npm dependencies); Node.js is not
needed to run Paraco.

See [the first-runtime brief](docs/first-runtime.md) for the acceptance criteria
and intentionally deferred work.

See [development and verification](docs/development.md) for pinned toolchains,
fresh-checkout checks, browser-smoke setup, and the distinction between configured
CI and observed native evidence.

See [installation and release planning](docs/installation-and-release.md) for
the future bundled runtime, CLI installers, and platform packaging plan.

## Current scope

The runtime supports foreground single-app and multi-app hosting, authenticated
browser lifecycle controls, Unix CLI lifecycle controls, and a local fake AI
capability, plus bounded persistent logs and CLI log queries. Authenticated dashboard logs and opt-in bounded automatic recovery are available.
Hosted apps use distinct canonical `.localhost` browser origins. They cannot use
shared-domain cookies; cross-origin browser requests are rejected, while the
separate management origin remains authenticated. Output-loss counters in the
management log view distinguish storage and console loss. Hosted admission is
bounded (50 apps, 64 gateway responses, 8 per app, and 4 log reads); these are
safety limits rather than CPU/RSS quotas.
Background service operation,
persistence, real AI providers, and AI HTTP compatibility/streaming
remain future work. Cloud integration and bundling/installers are deferred.

## Try the local AI capability

```sh
cargo run -- run ./examples/ai --ai-config ./examples/ai-config.json
curl http://127.0.0.1:3000
```

The response contains `Fake AI response` and the selected provider/model. This
example needs no account, provider secret, or external network request. The app
calls `context.ai.complete({ prompt: "Hello" })`; optional `provider` and `model`
fields select a route. Apps requesting `ai` receive the interface, but calls are
only permitted by grants in the explicitly selected host configuration. There
are no automatic grants. The configuration must live outside the app directory.
See [AI routing and transport](docs/ai-routing.md) for the contract and limits.
