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
working directory. Absolute paths also work. The app's manifest `name` labels the
dashboard and must be unique within the server. The host allocates the browser
origin and AI-grant identity from the app directory's canonical source, not that
display name. Replacing a configured path allocates a new identity; removing a
path retires it, so grants do not transfer when it is added again. Unknown configuration fields,
invalid manifests, and duplicate names fail before launching any app. An empty
`apps` list serves an empty dashboard. Changes take effect after restarting the
server; hot reload and persistent registration are not implemented.

`aiConfig` is optional and has the same semantics as `run --ai-config`. It must
be outside that app's directory and is only valid for an app requesting `ai`.
A configuration or startup error specific to that capability marks the app
failed without stopping the other apps. No grants are inferred from listing an
app in the server configuration. To configure a hosted grant, start the server,
read the deployment ID from its management API (`GET /api/apps`) or the
`app-<deployment-id>.localhost` link, put that ID under `apps` in the AI config,
then restart the server. This is a host-managed discovery flow; app manifests
cannot select an identity or grant themselves credentials.

## Routing and app URLs

| Public request | App receives | `context.basePath` |
| --- | --- | --- |
| `http://app-<stable-id>.localhost:<port>/` | `/` | `/` |
| `http://app-<stable-id>.localhost:<port>/style.css` | `/style.css` | `/` |
| `http://app-<stable-id>.localhost:<port>/items?q=one` | `/items?q=one` | `/` |
| Single-app `paraco run` request `/items` | `/items` | `/` |

Each hosted app has a distinct deterministic `.localhost` hostname derived from
its host-owned durable deployment ID. The app receives that canonical URL
and Host; this separates browser origins even though all traffic reaches the same
loopback listener. Query strings, methods, request bodies, and end-to-end headers
are forwarded. Client-supplied forwarding metadata and hop-by-hop headers are
removed. Local upstream requests never use inherited proxy settings.

The legacy shared `/apps/<name>/...` URLs are migration redirects only: GET and
HEAD receive a 308 to the app's canonical origin, while other methods receive
405. The shared gateway never proxies app-controlled content. Unknown routes
return 404; starting, stopping, stopped, or failed apps return 503. Proxy
connection failures return 502, and an upstream timeout before headers returns 504.

## Hosted-workload limits

Hosted configuration accepts at most 50 applications (`max_apps`, default 50).
The gateway admits up to 64 concurrent proxy responses overall and 8 per app;
it does not queue excess work, returning 503 so callers can retry. Permits stay
held until a streamed response completes or is cancelled. The management log
endpoint likewise admits four blocking reads and returns 503 rather than
creating an unbounded blocking-pool queue. These are M1 safety limits, not
throughput guarantees or CPU/RSS quotas. Shutdown allows at most two seconds
for outstanding runtime work after listeners close. Measure 1, 10, and 50-app
workloads on the target host with `scripts/probe-hosted-workload.cjs`.

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
or cookie paths. An absolute path such as `/settings` stays on the app's own
canonical origin. Set cookie `Path` to the app base path when appropriate. Relative URLs on nested pages follow normal
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
[local lifecycle CLI](local-lifecycle.md); automatic retries are opt-in. App output goes
to the foreground terminal and the bounded persistent [JSONL log store](local-logs.md).

### Output and log delivery

Child stdout and stderr are always drained independently of terminal output and
persistent logging. Each destination has a finite record queue (1024 records);
Child-to-persistence queues hold 1024 records and terminal queues hold 256; when
a destination is blocked, later records for that destination are discarded
instead of delaying readiness, lifecycle actions, or child reaping. Accepted
JSONL records retain FIFO ordering at the persistence worker and the existing
rotation limits, but a blocked filesystem may leave a gap or lose a final
lifecycle record. Stopping first terminates and reaps the child; it never waits
for a terminal or log-writer flush. Queue-drop, truncation, write-failure, and
shutdown-discard counters are kept independently so a blocked sink cannot hide
loss in another sink. This is the M1 blocked-output safety slice, not completion
of the remaining M1 cron/scheduler gates.
Use `paraco logs <app>` even after the server stops.

On shutdown, supervisors stop apps concurrently, send SIGTERM on Unix, and force
termination after five seconds if needed. Every Deno child is reaped and its
capability listener and temporary files are cleaned up. Apps still starting are
also cleaned up. The gateway stops accepting connections; in-flight requests may
be interrupted. Background operation and OS service integration come later.

## Current limits

- This is loopback-only hosting for trusted local apps. Each hosted app uses a
  distinct `.localhost` browser origin, and cross-origin `Origin` requests are
  rejected without CORS headers. The gateway drops response cookies that request
  a `Domain` attribute and gives every app launch a private loopback authorization
  header which the adapter verifies and removes before calling app code. This is
  a browser boundary, not authentication against a hostile local process or a
  complete untrusted-code sandbox.
- HTTP/1 request/response proxying is supported. Protocol upgrades such as
  WebSockets are explicitly unsupported.
- Request bodies are buffered up to 1 MiB with a ten-second body-read deadline.
  Upstream responses stream with a thirty-second request deadline. A timeout after
  response headers interrupts the body; it cannot replace the already-sent status.
- The shared dashboard accepts only `127.0.0.1:<port>` and `localhost:<port>`;
  app traffic accepts only its generated canonical hostname. Forged forwarding
  metadata and backend authorization headers are overwritten by the gateway.
- Unix CLI lifecycle commands use a private local socket. Browser controls use a
  separate authenticated loopback origin. Explicit user-service installation and
  opt-in automatic restart recovery are implemented; native service-manager
  verification and release publication remain pending. Cloud work is deferred.

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
