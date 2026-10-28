# Runtime and contributor guide

Paraco is a local-first runtime for portable applications. This repository is
a Rust host that runs apps on workerd and serves them through a local gateway,
with controls in the main dashboard. Bundles include workerd
`2026-10-26` and native esbuild `0.28.2`.

## Run the cron app

```sh
cargo run -- cron
```

An installed bundle runs the same app with `paraco cron`, without a checkout.
Open the local dashboard and select **cron**. Create and review TypeScript jobs,
publish one-off or recurring schedules, inspect run history and notifications,
and archive or restore jobs. AI drafting is optional and requires an explicit
host grant; proposals never publish automatically. See [tasks and cron](tasks-and-cron.md)
for the task contract, authorization, limits and current dependency restrictions.

## Try it

For source development, install Rust, Node.js, and the pinned npm dependencies:
`npm ci`, then put `node_modules/.bin` on `PATH`. End users of a Paraco bundle
need no Node.js or Deno.

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
  dashboard (the old `/apps/<name>/` links redirect only for GET/HEAD). AI calls
  are denied until you configure the hosted deployment grant described below.

Use `--port 8787` for another loopback port. Hosting reserves both IPv4 and IPv6
loopback addresses; if either is occupied, choose another port. This runs in the foreground;
Ctrl+C stops all apps. Configuration paths resolve relative to the configuration
file, and each app's validated manifest name determines its URL (for example,
`http://hello.localhost:3000`). Private deployment IDs remain separate from URLs. The dashboard
at `http://localhost:3000/` includes app controls, logs, and plugin downloads.
It refreshes status every two seconds and requires no separate management link.

See [local hosting](local-hosting.md) for the configuration, app base-path
contract, and limits. [Current status](status.md) records implementation
and verification evidence.

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
3000. CLI management uses a private local socket. Browser controls use the main dashboard on the same gateway port.
See [local lifecycle](local-lifecycle.md) for state, security, and recovery
semantics. Deployment identity and desired state persist across server sessions.
App guardians clean up workerd after a runtime crash, and restarting `serve`
waits for cleanup before safely recovering its stale control socket.

## Persistent logs

Application output and lifecycle events are retained as agent-readable JSONL in
`~/.paraco/logs` by default (`%USERPROFILE%\.paraco\logs` on Windows). Override
with `--log-dir <path>` or `PARACO_LOG_DIR`. Data is independent: select its
base with `--data-dir` or `PARACO_DATA_DIR` (default `~/.paraco`). A rotating
five-file store caps retained
log data at 10 MiB per directory, shared across apps and server sessions.

```sh
cargo run -- logs hello --tail 100
cargo run -- logs hello --port 3000 --tail 500
```

These commands work even after the server stops. Records identify the app,
launch, process, stream, and capture time. See [local logs](local-logs.md)
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
`name` is 1–63 lowercase ASCII letters, digits, or hyphens, starts with a
letter, and ends with a letter or digit. `entrypoint` must be a relative file contained in the application
directory. `capabilities` is required and may be empty or contain `"ai"` for AI routing, `"storage"` for scoped configuration/data, and `"tasks"` for explicitly host-granted task authoring. Unknown fields and unsupported capabilities fail
validation. The machine-readable draft schema is
[`docs/paraco-manifest.schema.json`](paraco-manifest.schema.json), and the
matching handler types are in [`runtime/paraco.d.ts`](../runtime/paraco.d.ts).

The entrypoint exports a default object whose `fetch` method receives web
standard `Request` and returns a `Response` (or a promise for one). esbuild
bundles package dependencies during compilation or preparation; apps do not
download modules at launch. Static text imports use the JavaScript import attribute syntax, for
example `import page from "./page.html" with { type: "text" };`.

```ts
export default {
  fetch(_request: Request, _context: object): Response {
    return new Response("Hello from Paraco");
  },
};
```

Rust compiles TypeScript and local app imports to browser ES modules with the
bundled esbuild compiler. Apps run on the pinned workerd runtime and do not get
Node or Deno globals, runtime dependency downloads, or ambient filesystem access.
On Linux, execution uses bubblewrap with unprivileged user
and network namespaces; macOS fails closed until secure containment is
implemented. `PARACO_ALLOW_UNSANDBOXED=1` is an explicit trusted-development
override. See [workerd migration limits](workerd-migration.md).

## Verify

```sh
cargo fmt --check
cargo test
node --test tests/management.test.cjs
cargo run -- run ./examples/hello
```

The dashboard tests use Node.js; it is a development dependency and is not
needed to run a packaged Paraco host.

See [the first-runtime brief](first-runtime.md) for the acceptance criteria
and intentionally deferred work.

See [development and verification](development.md) for pinned toolchains,
fresh-checkout checks, browser-smoke setup, and the distinction between configured
CI and observed native evidence.

See [installation and release planning](installation-and-release.md) for
the archive bundle workflow, CLI installer, and platform packaging plan.

## Current scope

The runtime supports foreground single-app and multi-app hosting, browser lifecycle controls, Unix CLI lifecycle controls, and a local fake AI
capability, plus bounded persistent logs and CLI log queries. Dashboard logs and opt-in bounded automatic recovery are available.
Hosted apps use distinct canonical `.localhost` browser origins. They cannot use
shared-domain cookies; cross-origin subresource requests are rejected, while the
dashboard remains on its own origin, separate from hosted apps. Output-loss counters in the
management log view distinguish storage and console loss. Hosted admission is
bounded (50 apps, 64 gateway responses, 8 per app, and 4 log reads); these are
safety limits rather than CPU/RSS quotas.
Opt-in user-service definitions, deployment-scoped durable state, and a bounded
host-owned OpenAI-compatible provider are implemented. The retained verification record distinguishes observed Linux/macOS integration
and launchd checks from remaining native Linux service, clean macOS, machine
reboot, signing, publication, and native package gates. Bounded tasks, durable scheduling, a local notification inbox and the cron reference app
are implemented; see their documented limits. Health monitoring and cloud integration remain deferred. Local operation needs neither a
cloud account nor paid AI; the fake provider remains available.

## Try the local AI capability

First allocate the app's stable host identity:

```sh
cargo run -- identity ./examples/ai
```

Copy `examples/ai-config.json` to a host-owned location outside the app directory
and replace `REPLACE_WITH_PARACO_IDENTITY_OUTPUT` with that output. Then:

```sh
cargo run -- run ./examples/ai --ai-config /path/to/ai-config.json
curl http://127.0.0.1:3000
```

Use the same `--log-dir` for identity discovery and launch if you override it.
Hosted deployments have separate IDs; discover them with `paraco status` after
starting the server, configure their grants, and restart the affected app.

The response contains `Fake AI response` and the selected provider/model. This
example needs no account, provider secret, or external network request. The app
calls `context.ai.complete({ prompt: "Hello" })`; optional `provider` and `model`
fields select a route. Apps requesting `ai` receive the interface, but calls are
only permitted by grants in the explicitly selected host configuration. There
are no automatic grants. The configuration must live outside the app directory.
See [AI routing and transport](ai-routing.md) for the contract and limits.

SQLite migration, scoped cron-draft storage, backup/export, and authenticated
`paraco open` are described in [durable state and services](durable-state.md).
See [retained verification evidence](verification-2026-09-21.md) for the
observed platform checks and gates that still require native machines.

## Approved app marketplace

Browse the curated catalog and install an exact approved package:

```sh
cargo run -- marketplace list
cargo run -- marketplace install hello-world --output ./installed/hello-world
cargo run -- run ./installed/hello-world
```

See [marketplace design and packaging](marketplace.md) for approval, local
package testing, repository templates and the release workflow. The
[Hello World v0.1.0 release](https://github.com/dioko-ai/paraco-hello-world/releases/tag/v0.1.0)
is published and its catalog download has been verified end to end.

## System monitoring

Local Linux applications may request the read-only `system` capability for aggregate metrics and socket discovery. See [system monitoring](system-monitoring.md) for the API, installation permissions and platform limits.
