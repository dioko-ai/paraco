# Foreground local hosting

Run several app directories and local dashboards in one foreground server:

```sh
cargo run -- serve --config ./examples/server.json
```

Open `http://127.0.0.1:3000/`. Use `--port 8787` to choose another port. Deno must
be on `PATH`. The terminal also prints a private management dashboard link on a
separate loopback port, with start, stop, and restart controls. See
[local lifecycle](local-lifecycle.md#browser-management) for authorization.
There is no runtime installation or service registration in this
increment. Ctrl+C or SIGTERM stops the server and all its apps on Unix.

## Configuration

```json
{
  "apps": [
    { "path": "hello" },
    { "path": "ai", "aiConfig": "ai-config.json" }
  ]
}
```

Paths are relative to the configuration file, independent of the shell's current
working directory. Absolute paths also work. The app's manifest `name` determines
its route; names must be unique within the server. Unknown configuration fields,
invalid manifests, and duplicate names fail before launching any app. An empty
`apps` list serves an empty dashboard. Changes take effect after restarting the
server; hot reload and persistent registration are not implemented.

`aiConfig` is optional and has the same semantics as `run --ai-config`. It must
be outside that app's directory and is only valid for an app requesting `ai`.
A configuration or startup error specific to that capability marks the app
failed without stopping the other apps. No grants are inferred from listing an
app in the server configuration.

## Routing and app URLs

| Public request | App receives | `context.basePath` |
| --- | --- | --- |
| `/apps/hello/` | `/` | `/apps/hello/` |
| `/apps/hello/style.css` | `/style.css` | `/apps/hello/` |
| `/apps/hello/items?q=one` | `/items?q=one` | `/apps/hello/` |
| Single-app `paraco run` request `/items` | `/items` | `/` |

The app sees the gateway's canonical loopback origin in `request.url`, with the
mount prefix removed from the path. The base path always begins and ends with
`/`. Query strings, methods, request bodies, and end-to-end headers are forwarded.
Client-supplied forwarding metadata is removed. Hop-by-hop headers are removed
in both directions. Local upstream requests never use inherited proxy settings.

`/apps/hello` redirects with HTTP 308 to `/apps/hello/`, preserving the query.
Unknown routes return 404; starting, stopping, stopped, or failed apps return 503. Proxy connection
failures return 502, and an upstream timeout before headers returns 504.

Use relative assets such as `style.css` on an app's root page, or construct
public links using `context.basePath`. For redirects, for example:

```ts
return new Response(null, {
  status: 302,
  headers: { location: `${context.basePath}settings` },
});
```

Relative redirect locations also work. The gateway returns redirects to the
browser without following them. It does not rewrite HTML, JavaScript, `Location`,
or cookie paths. An absolute path such as `/settings` refers to the gateway root;
it will not automatically become `/apps/hello/settings`. Set cookie `Path` to the
app base path when appropriate. Relative URLs on nested pages follow normal
browser URL resolution rules.

## Dashboard and supervision

The gateway root dashboard shows configured app names, routes, and starting/running/failed
states, plus stopping and stopped states after lifecycle commands. Running apps
have an Open app link. Failed apps show a supervisor error;
other apps continue serving. The page refreshes every five seconds and does not
cache its status. It is read-only and requires no external assets or services.
Running means the adapter bound its listener and the process has not exited;
it is not an application health probe.

Each app runs in its own supervised Deno process, using the same owner and cleanup
implementation as `paraco run`. Deno chooses an ephemeral loopback port and reports
that port after binding. This avoids allocating a free port and hoping it remains
free before startup. The public gateway binds before any app is launched.

Startup happens independently: a slow import does not prevent the dashboard or
another app from serving. Startup times out after ten seconds. A failed or crashed
app retries when its restart policy is enabled; otherwise it remains failed until started or restarted through the
[local lifecycle CLI](local-lifecycle.md); automatic retries are future work. App output goes
to the foreground terminal and the bounded persistent [JSONL log store](local-logs.md).
Use `paraco logs <app>` even after the server stops.

On shutdown, supervisors stop apps concurrently, send SIGTERM on Unix, and force
termination after five seconds if needed. Every Deno child is reaped and its
capability listener and temporary files are cleaned up. Apps still starting are
also cleaned up. The gateway stops accepting connections; in-flight requests may
be interrupted. Background operation and OS service integration come later.

## Current limits

- This is loopback-only hosting for trusted local apps. Hosted app paths share one browser
  origin; path routing does not isolate cookies, browser storage, or scripts across
  apps. Separate processes do not provide a complete untrusted-code sandbox.
- HTTP/1 request/response proxying is supported. Protocol upgrades such as
  WebSockets are explicitly unsupported.
- Request bodies are buffered up to 1 MiB with a ten-second body-read deadline.
  Upstream responses stream with a thirty-second request deadline. A timeout after
  response headers interrupts the body; it cannot replace the already-sent status.
- Only `127.0.0.1:<port>` and `localhost:<port>` Host headers are accepted. The
  gateway always supplies the canonical `127.0.0.1:<port>` host to apps.
- Unix CLI lifecycle commands use a private local socket. Browser controls use a
  separate authenticated loopback origin. Service installation and automatic
  restart recovery remain future work.
  Cloud and release packaging remain deferred.

## Verification

With Deno on `PATH`, run:

```sh
cargo fmt --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
```

The Unix multi-app integration suite verifies two live apps, distinct processes,
path stripping, base paths, queries, POST bodies, assets, redirects, HEAD requests,
dashboard status, AI through the gateway, configuration resolution from another
working directory, duplicate-name rejection, occupied ports, failure isolation,
hanging requests, and cleanup both while running and during startup. The existing
single-app and AI suites continue to run against the shared supervisor.
