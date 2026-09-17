# paraco

Paraco is a local-first runtime for portable applications. This repository is
building its standalone foundation: Rust supervises Deno applications and serves
them through a local gateway with a read-only dashboard.

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

- Hello at <http://127.0.0.1:3000/apps/hello/>
- Fake AI at <http://127.0.0.1:3000/apps/ai-example/>

Use `--port 8787` for another loopback port. This runs in the foreground;
Ctrl+C stops all apps. Configuration paths resolve relative to the configuration
file, and the manifest name determines each app's URL. The dashboard refreshes
its starting/running/failed statuses every five seconds.

See [local hosting](docs/local-hosting.md) for the configuration, app base-path
contract, and limits. [TODO.md](TODO.md) tracks completed work and the next
increments: lifecycle controls, then background service operation.

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

`name` is 1–63 lowercase ASCII letters, digits, or hyphens and starts with a
letter. `entrypoint` must be a relative file contained in the application
directory. `capabilities` is required and may be empty or contain `"ai"` for the
local fake AI capability. Unknown fields and unsupported capabilities fail
validation.

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
cargo run -- run ./examples/hello
```

See [the first-runtime brief](docs/first-runtime.md) for the acceptance criteria
and intentionally deferred work.

See [installation and release planning](docs/installation-and-release.md) for
the future bundled runtime, CLI installers, and platform packaging plan.

## Current scope

The runtime supports foreground single-app and multi-app hosting, a read-only
dashboard, and a local fake AI capability. Lifecycle controls, background service
operation, persistence, real AI providers, and AI HTTP compatibility/streaming
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
