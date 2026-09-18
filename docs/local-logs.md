# Persistent local logs

Both `paraco run` and `paraco serve` persist application stdout, stderr, and
supervisor lifecycle events as JSON Lines (UTF-8, one JSON object per line).
They also continue forwarding app output to the foreground terminal. No cloud
service, background daemon, or log collector is required.

## Location

Resolution order:

1. `--log-dir <directory>` on the CLI, before or after the subcommand.
2. `PARACO_LOG_DIR` environment variable.
3. The user's home directory plus `.paraco/logs`.

The default is `$HOME/.paraco/logs` on Linux and macOS, and
`%USERPROFILE%\.paraco\logs` on Windows. Use an override for a platform-specific
service or system location; native service integration remains future work.
An unset home variable requires an explicit override. Relative overrides resolve
against the invoking process's working directory. The runtime prints the resolved
path on stderr at startup. Only Linux is currently verified.

```sh
cargo run -- serve --config ./examples/server.json --log-dir /tmp/paraco-logs
cargo run -- logs hello --tail 100 --log-dir /tmp/paraco-logs
```

Use the same override when reading logs. If using the default location:

```sh
cargo run -- logs hello --tail 100
cargo run -- logs hello --port 3000 --tail 500
cargo run -- logs --tail 1000
```

The `logs` command prints only JSONL records to stdout, oldest to newest among
the selected tail. It works after the server exits. App and port filters are
optional; `--tail` defaults to 100 and accepts 0–10000. No matches produce empty
output. Errors go to stderr with a nonzero exit status. `--port` filters the
recorded public port, without connecting to a server.

## Record contract

```json
{"schema_version":1,"timestamp_unix_ms":1789660800000,"app":"hello","mode":"serve","port":3000,"run_id":"41d735a402bb4cf692289f4839ab7cf0","pid":12345,"stream":"stderr","event":"output","message":"Error: request failed","truncated":false}
```

- `timestamp_unix_ms`: host capture time, milliseconds since the Unix epoch.
- `app`: manifest name; `mode`: `run` or `serve`; `port`: public listening port.
- `run_id`: random ID for each launch attempt, including retries and restarts.
- `pid`: Deno process ID, or null before spawning.
- `stream`: `stdout`, `stderr`, or `supervisor`.
- `event`: `output`, `starting`, `running`, `failed`, `launch_failed`, `stopped`,
  or `read_error`. `stopped` means resources were cleaned up, including after a
  failure; use the preceding events to diagnose why. `launch_failed` can report
  manifest revalidation failure before spawning. Initial manifest/configuration
  validation errors remain on the terminal because no app launch exists yet.
- `message`: text, JSON-escaped. Application-emitted JSON remains a string here.
- `truncated`: true if the line exceeded the 16 KiB message limit.

Apps' multi-line stack traces produce multiple records with the same run ID.
Invalid UTF-8 becomes the replacement character. Oversized lines retain their
first 16 KiB and discard the remainder through the next newline. Partial final
lines are captured at EOF. Ordering is capture/write order; stdout and stderr
are independent pipes, so exact ordering between streams is not guaranteed.

For example, with `jq` installed:

```sh
paraco logs hello --tail 1000 | jq -c 'select(.stream == "stderr" or .event == "failed")'
```

## Rotation and pruning

Each log directory contains one combined store for all apps and runtime
instances using that directory:

- `current.jsonl`: current output.
- `archive-1.jsonl` through `archive-4.jsonl`: newest to oldest archives.
- `.lock`: an empty coordination file; do not remove it while runtimes are active.

Each JSONL file is capped at **2 MiB**; at most **five** are retained, for a
**10 MiB total log-data bound per configured directory**. Rotation happens before
a record would exceed the limit. The oldest archive is deleted automatically.
Records are never split across files. These limits are fixed for this increment,
so multiple writers cannot disagree on retention settings. Creating multiple
log directories creates separate budgets.

This is size-based retention, with no age-based deletion or per-app reservation.
A noisy app can displace another app's older logs. Fixed filenames mean old apps,
launches, and server ports do not accumulate additional files. Files unrelated
to this store are never pruned. No cleanup timer needs to stay running after exit.

An OS file lock coordinates independent runtimes, output threads, and CLI reads.
Lock acquisition waits at most 250 ms. An incomplete trailing record left by an
interrupted writer is removed before subsequent appends or reads. Writes go to
the OS on each record; there is no per-record fsync guarantee against power loss.
Use a local filesystem with OS file locking. External log rotation is unsupported.

## Access and failure behavior

The store must live outside all configured application directories. Deno receives
no additional filesystem permission for logs. On Unix, newly created directories
use 0700 and files use 0600. Existing storage must be owned by the current user
and inaccessible to group/other users. Symlinked store directories/files and
hard-linked files are rejected on Unix. Windows uses the account's directory
ACLs; explicit Windows ACL hardening and verification are future work.

Only app output and app supervisor events are captured. Readiness markers,
browser management credentials, and host AI bootstrap credentials are excluded.
The runtime does not independently log AI prompts or responses. Anything an app
explicitly prints is retained, including any sensitive data it chooses to print.

An unusable store at startup prevents application launches and reports an error.
A later write/rotation/lock error prints a warning once per launch and leaves the
app running; subsequent records retry storage, but failed records are lost.
Storage is bounded, not a lossless audit trail.

## Verification and next increment

Tests cover bounded rotation and pruning, safe storage permissions, concurrent
threads and independent runtime processes, incomplete-record recovery, line
limits, invalid UTF-8, CLI path precedence and filters, startup/request failures,
per-launch identity, and retrieval after shutdown. Integration fixtures use
isolated log directories and do not write into the user's default store.

The next increment is a log viewer through the authenticated management dashboard.
Cloud collection, live log following, and packaging remain deferred.
