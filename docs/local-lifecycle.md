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
an `error` when failed or waiting to retry. Unknown app names and transport errors produce a nonzero
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
`stopping`, `stopped`, `backoff`, or `failed`. Apps deliberately stopped stay
stopped for this server session. Automatic retries are opt-in per app.

Commands for an app are serialized. A newer request supersedes a pending request;
several quick restarts may coalesce. There is never more than one supervised
process for that app. Other apps and management remain responsive during startup
and the bounded shutdown wait. Non-running app routes return 503, and the public
URL remains the same across restarts.

Each launch revalidates the manifest and reloads source and AI configuration, so a
failed app can be fixed and started without restarting the server. The configured
name must remain the same; changing app names or the server's app list requires a
server restart. Capability grants still come only from the host configuration.

## Automatic recovery

Configure recovery in each `serve` app entry:

```json
{"apps":[{"path":"./hello","restart":{"onFailure":true,"maxRetries":3,"backoffMs":1000,"maxBackoffMs":30000}}]}
```

Omitting `restart` disables automatic recovery. The other fields default to the
values above. Startup failures and unexpected process exits (including exit code
zero) count as failures; individual HTTP errors do not. Each retry revalidates
and reloads the app after cleaning up its old process and capability resources.

The delay doubles after each failure up to `maxBackoffMs`. `maxRetries` counts
additional launches after the initial attempt, and is limited to 0–100. Delays
must satisfy `1 <= backoffMs <= maxBackoffMs <= 300000` milliseconds. The retry
budget lasts until a manual restart, a start of a failed/stopped app, or server
restart; a successful launch does not reset it. This prevents repeatedly crashing
apps from retrying forever.

Both dashboards and CLI status show `backoff` with the last failure, retry number,
and scheduled delay. Exhaustion shows `failed` with the failure and retry limit.
Stop cancels a pending delay or launch. Restart resets the budget and supersedes
pending retries. Start during backoff is a no-op. Other apps and management remain
responsive while an app waits. Each launch has its own ID in persistent logs.

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
its socket. A private `<port>.lock` file establishes exclusive endpoint ownership;
the file stays in place between sessions. Startup waits up to eight seconds for
ownership, then checks any existing socket. Only a socket owned by this user with
mode 0600 whose connection is refused can be removed. Live listeners, symlinks,
regular files, and ambiguous connection failures are preserved and reported.
Do not delete ownership lock files: their stable inode coordinates all owners.

## Runtime crash recovery

Each app runs beneath a small Rust guardian process. The runtime keeps a private
pipe open to that guardian; closing the pipe or killing the runtime closes the
ownership channel. The guardian sends Deno SIGTERM, allows five seconds for
graceful exit, then kills and reaps it if necessary. This also works during app
import and when JavaScript's event loop is blocked. The guardian owns the temporary
adapter/cache directory and removes it after Deno exits. Persistent logs remain.

For `serve`, guardians retain the endpoint ownership lock until cleanup completes.
An immediate restart waits for them before recovering the stale socket and
launching apps. A second runtime cannot replace a live runtime's endpoint. No
stored PID is used as evidence that a process belongs to Paraco. The same
parent-death cleanup applies to standalone `run`.

This adds one guardian process per running app. It covers termination of the
main runtime while its guardians remain operational; killing a guardian itself
or failure of the OS is outside this mechanism. Apps remain trusted local code,
and arbitrary app-created subprocess trees are not supported. Durable desired
state and opt-in owned user-service definitions are available, but retained
native service-manager/reboot evidence remains pending.

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
persisting stopped state across server restarts is future work. Bounded persistent
logs are available through the [log CLI and dashboard](local-logs.md).
## User-service definitions

`paraco service render --platform linux|macos --entry /absolute/paraco --config /absolute/config.json --state /absolute/state` renders a private user-service definition using absolute paths and a ten-second graceful-stop bound. `paraco service setup|remove|status --platform linux|macos --config /absolute/config.json` explicitly operates an owned user registration; setup atomically installs the stable `~/.paraco/service/paraco` launcher pointing to the current immutable executable and uses `~/.paraco/state`, so future service launches do not depend on PATH, terminal CWD, or a release-specific definition. Replacing that launcher does not affect a process already running its immutable executable. It never invokes `sudo`, refuses foreign definitions or launcher files, and removal preserves configuration and state. Native-manager invocations are bounded to ten seconds and report an actionable error when unavailable. There is no `paraco open` command: the current private management entry is intentionally printed only by a running server, rather than recovering or logging its rotating bearer token.

A Linux systemd user unit normally starts when the user session is available; unattended boot requires user-controlled `loginctl enable-linger` and is not enabled by Paraco. A macOS LaunchAgent runs at user login, not before login. Native manager, reboot, terminal, and shutdown behavior has not been verified on either platform.

Cloud integration, scheduling, notifications, health monitoring, and published
bundling/installers remain future work. The local service and installer tooling
is unpublished and does not establish native release support.

Integration tests cover independent app control, repeated requests, changed
process identities, process reaping, failure repair, startup cancellation, rapid
command changes, slow shutdown, management access restrictions, malformed and
stalled clients, occupied sockets, and AI policy/restart paths. The Rust/unit
suite does not itself establish Deno-process integration evidence.
Crash tests kill the runtime during startup and with a blocked app event loop,
restart immediately with multiple apps, verify Deno reaping and temporary-directory
cleanup, and preserve live or unexpected control endpoints. These tests run in the
Unix suite for Linux and macOS when its Deno prerequisite is available. The
current pinned target is Deno 2.2.5; this workspace has no Deno, so no local
Deno-process verification is claimed. Native macOS evidence remains pending.

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
