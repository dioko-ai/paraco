# Local app lifecycle

Start the foreground server in one terminal:

```sh
cargo run -- serve --config ./examples/server.json
```

Manage its configured apps from another terminal:

```sh
cargo run -- status
cargo run -- stop hello
cargo run -- status hello
cargo run -- start hello
cargo run -- restart hello
```

Use `--port 8787` on both `serve` and management commands when selecting another
server. Management commands apply to `serve`, not standalone `run` processes.
They currently require Unix; verification is on Linux. Foreground hosting remains
available on other platforms.

## Command behavior

Every successful command prints a JSON array of app statuses. `status` lists all
apps, or just the named app. Each record contains `name`, `desired`, `state`, and
an `error` when failed. Unknown app names and transport errors produce a nonzero
exit code and an error on stderr.

- `start` requests a running app. Starting an already running or starting app is
  a no-op. Starting a failed app retries it.
- `stop` requests a stopped app. Repeating it is a no-op. It also cancels startup.
- `restart` requests replacement of the current process, or starts a stopped or
  failed app. The old process and its capability resources are cleaned up before
  the replacement is launched.

Lifecycle commands acknowledge the request immediately; success means the
request was accepted, not that the app has finished starting or stopping. Use
`status <name>` to observe completion or failure. The management dashboard displays desired and observed state every two seconds.
The read-only gateway dashboard continues refreshing every five seconds.

Desired state is `running` or `stopped`. Observed state is `starting`, `running`,
`stopping`, `stopped`, or `failed`. A crashed app has desired state `running` and
observed state `failed`. There are no automatic retries yet. Apps deliberately
stopped stay stopped for this server session.

Commands for an app are serialized. A newer request supersedes a pending request;
several quick restarts may coalesce. There is never more than one supervised
process for that app. Other apps and management remain responsive during startup
and the bounded shutdown wait. Non-running app routes return 503, and the public
URL remains the same across restarts.

Each launch revalidates the manifest and reloads source and AI configuration, so a
failed app can be fixed and started without restarting the server. The configured
name must remain the same; changing app names or the server's app list requires a
server restart. Capability grants still come only from the host configuration.

## Local management boundary

The CLI connects to a Unix socket, separate from the HTTP gateway. The socket
lives at `<temporary-directory>/paraco-control-<uid>/<port>.sock`. The parent must
be a real directory owned by the current user with mode 0700; the socket has mode
0600. The server and CLI must use the same user and temporary directory (including
any `TMPDIR` override). Management commands do not use browser cookies, HTTP
routes, or application capability tokens.

This provides a local user boundary; it does not isolate other programs running
as that user. Apps are still trusted local code under the existing documented
Deno permission limits. The gateway dashboard remains read-only. Browser management uses a separate
loopback origin with a per-server credential, as described below.

Control messages have size limits and timeouts. Normal server shutdown removes
its socket. An existing socket is never automatically replaced. If a server is
forcibly killed and leaves a stale socket, first confirm it is no longer running,
then remove the exact socket path reported by the startup error and retry. The
private parent directory is retained for subsequent server sessions.

## Browser management

`serve` prints a separate link such as
`http://127.0.0.1:<management-port>/#<access-token>`. Open the exact terminal link
in your browser. The management port is allocated by the OS; `--port` continues
to identify the app gateway and CLI endpoint. The original gateway dashboard
remains available at `/` as a read-only overview.

The management page provides Start, Stop, and Restart for each app, displays
desired and observed state plus failure details, and refreshes every two seconds.
Actions use the same supervisor operations as the CLI. An acknowledgment means
the operation was requested; watch status for completion. App links open the
app gateway in a separate tab without an opener or referrer.

The URL fragment holds a random 256-bit bearer token. The page immediately removes
it from the address bar and keeps it in tab-scoped session storage, so refreshes
remain authorized. If browser storage is disabled, the current page still works,
but a refresh requires reopening the original link. Closing the tab normally
ends that tab's access; browser session restoration may retain it. Restarting
Paraco rotates the credential. Keep the printed link private: anyone with the
link and local network access can manage this server. It appears in terminal
output and any file to which that output is redirected.

The server never embeds the token in a page, app response, or cookie. Status and
commands require an authorization header. Commands additionally require an exact
management Origin and JSON content type. Host validation, Fetch Metadata checks,
no CORS permissions, a restrictive content security policy, and no-store responses
protect the browser boundary. Requests have a 4 KiB body limit and a two-second
body-read deadline. App text is rendered as text, not HTML. This does not protect
against another program running as the same OS user, browser extensions with
access, or compromise of the browser or host.

Both loopback listeners bind before apps launch and close when the server stops.
Browser controls do not require the Unix CLI socket, so their transport also
works on platforms without Unix sockets; only Linux is verified so far.

## Scope and verification

Desired state is in memory. Restarting the server starts all configured apps;
persisting stopped state across server restarts is future work. Retained logs, automatic crash recovery, background services, cloud integration,
and bundling/installers are outside this increment.

Integration tests cover independent app control, repeated requests, changed
process identities, process reaping, failure repair, startup cancellation, rapid
command changes, slow shutdown, management access restrictions, malformed and
stalled clients, occupied sockets, and AI across restarts. On Linux, the AI test
also inspects host-owned listeners and verifies their release after stopping.

```sh
cargo fmt --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
node --test tests/management.test.cjs
```

Tests require Deno on `PATH`, local sockets, and subprocess signals.

Dashboard integration tests exercise authorization, cross-origin rejection, invalid
commands, process replacement, independent apps, and listener cleanup. JavaScript
tests use Node.js and a simulated DOM to check token handling, button requests,
polling, keyboard focus, and safe text rendering. A graphical browser was not
available for visual verification in the development environment.
