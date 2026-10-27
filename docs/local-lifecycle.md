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
They currently require Unix; Linux/macOS ARM64 checks are retained. Foreground hosting remains
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
The main dashboard refreshes app status every two seconds.

Desired state is `running` or `stopped`. Observed state is `starting`, `running`,
`stopping`, `stopped`, `backoff`, or `failed`. Apps deliberately stopped stay
stopped across server restarts. Desired state is stored durably for the
canonical configured app source; automatic retries are opt-in per app.

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
as that user. Apps are not isolated from the same OS user. Linux app execution uses bubblewrap
with unprivileged user and network namespaces; macOS currently fails closed for
normal execution. Trusted development can explicitly set
`PARACO_ALLOW_UNSANDBOXED=1`. See [workerd migration limits](workerd-migration.md). The main dashboard includes browser controls on the gateway port, as described below.

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
ownership channel. The guardian sends workerd SIGTERM, allows five seconds for
graceful exit, then kills and reaps it if necessary. This also works during app
startup and when JavaScript's event loop is blocked. The guardian owns the
temporary host/runtime files and removes them after workerd exits. Persistent logs remain.

For `serve`, guardians retain the endpoint ownership lock until cleanup completes.
An immediate restart waits for them before recovering the stale socket and
launching apps. A second runtime cannot replace a live runtime's endpoint. No
stored PID is used as evidence that a process belongs to Paraco. The same
parent-death cleanup applies to standalone `run`.

This adds one guardian process per running app. It covers termination of the
main runtime while its guardians remain operational; killing a guardian itself
or failure of the OS is outside this mechanism. Apps remain trusted local code,
and arbitrary app-created subprocess trees are not supported. Durable desired
state and opt-in owned user-service definitions are available. Linux systemd
restart and cleanup evidence is retained in the [current verification record](verification-2026-10-25.md); machine reboot and native macOS checks remain open.

## Browser management

Open `http://localhost:3000/` (or the port selected with `--port`). This main
dashboard provides Start, Stop, Restart, logs, and plugin download and uninstall
controls. `127.0.0.1` and `[::1]` also work. `paraco open [--port N]` opens the same
page; no separate management listener or access token is required.

Status refreshes every two seconds. Actions use the same supervisor operations as
the CLI; an acknowledgment means the operation was requested. App links open their
isolated `.localhost` origins in another tab. Downloaded plugins can be stopped
and uninstalled from the dashboard; manually configured apps remain managed by
the server configuration.

Browser commands require an Origin matching the dashboard address and JSON
content type. Host validation, Fetch Metadata checks, no CORS permissions, and a
restrictive content security policy keep hosted app pages and other websites
from issuing dashboard commands. Requests have a 4 KiB body limit and a two-second
body-read deadline. These controls are available to local programs; there is no
browser login or bearer credential.

## Scope and verification

Desired state is committed before it is published to lifecycle controls. Restarting the server
reconciles the current configuration: configured canonical sources retain their
identity and desired state, removed sources are retired, and newly configured
sources receive new identities. Restarting the server starts only configured apps whose
desired state is running. Bounded persistent
logs are available through the [log CLI and dashboard](local-logs.md).
## User-service definitions

`paraco service render --platform linux|macos --entry /absolute/paraco --config /absolute/config.json --state /absolute/state` renders a private user-service definition using absolute paths and a ten-second graceful-stop bound. `paraco service setup|remove|status --platform linux|macos --config /absolute/config.json` explicitly operates an owned user registration; setup atomically installs the stable `~/.paraco/service/paraco` launcher pointing to the current immutable executable and uses `~/.paraco/state`, so future service launches do not depend on PATH, terminal CWD, or a release-specific definition. Replacing that launcher does not affect a process already running its immutable executable. It never invokes `sudo`, refuses foreign definitions or launcher files, and removal preserves configuration and state. Native-manager invocations are bounded to ten seconds and report an actionable error when unavailable. `paraco open` opens the main dashboard of the running server.

A Linux systemd user unit normally starts when the user session is available; unattended boot requires user-controlled `loginctl enable-linger` and is not enabled by Paraco. A macOS LaunchAgent runs at user login, not before login. A current Linux systemd run observed startup, unexpected-death recovery, terminal independence, stopped-intent preservation, and graceful shutdown; see the [October verification record](verification-2026-10-25.md). Machine reboot and macOS manager behavior remain unverified, as does the public service setup/removal flow in a dedicated account.

Bounded task scheduling, run history, and local notifications are implemented;
the authorization, cron grammar, limits, and recovery contract are documented
in [Tasks and cron](tasks-and-cron.md). Cloud integration and health monitoring
remain future work. Bundle and installer tooling is local and unpublished; it
does not establish native release support.

Integration tests cover independent app control, repeated requests, changed
process identities, process reaping, failure repair, startup cancellation, rapid
command changes, slow shutdown, management access restrictions, malformed and
stalled clients, occupied sockets, and AI policy/restart paths. The Rust/unit
suite does not itself establish workerd process integration evidence.
Crash tests kill the runtime during startup and with a blocked app event loop,
restart immediately with multiple apps, verify workerd reaping and temporary-directory
cleanup, and preserve live or unexpected control endpoints. These tests run in the
Unix suite with npm-installed workerd `2026-10-26` and esbuild `0.28.2`; trusted
macOS development runs require `PARACO_ALLOW_UNSANDBOXED=1`. See [current status](status.md) for
current implementation commands and retained observations. September
Linux/macOS lifecycle evidence remains dated in
[the verification record](verification-2026-09-21.md); it does not establish
the current task/release gates or native macOS service and reboot behavior.

```sh
npm run check:core
npm run test:cron-runtime
npm run test:cron-browser
npm run test:installer
npm run test:bundle-contract
```

Cron runtime checks require the pinned workerd runtime and esbuild compiler; browser checks require the
configured Node/Playwright/Chromium tools. The Unix lifecycle suite requires
local sockets and subprocess signals.

Dashboard integration tests exercise authorization, cross-origin rejection, invalid
commands, process replacement, independent apps, and listener cleanup. JavaScript
tests use Node.js and a simulated DOM to check token handling, button requests,
polling, keyboard focus, and safe text rendering. The Chromium smoke test clicks dashboard app links and checks separate browser
origins. See [current status](status.md) for observed results.
