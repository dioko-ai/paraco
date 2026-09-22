# Durable state and user services

Runtime state uses SQLite schema version 1 (`PRAGMA user_version`). First open
imports the existing format-2 `state.json` in the same transaction that creates
the schema. IDs and stopped/running intent are preserved. The original JSON is
left untouched as migration evidence and is never reimported after migration.
Invalid JSON, unsupported legacy versions, constraint failures, and newer SQLite
schemas fail closed. Migration rollback and reopening are regression tested.

The runtime keeps its exclusive ownership lease while app guardians clean up.
Stored process IDs are never ownership proof. Writes commit before lifecycle
commands acknowledge success. SQLite uses full synchronous durability; executable
artifacts, provider credentials, and application data remain separate.

State directories are private to their owner. The default hosted database is
`<log-directory-parent>/state/<sha256(canonical-server-config-path)>/state.sqlite3`.
Standalone applications use their canonical source identity under that state
parent. Changing a canonical configuration/source path changes its namespace.

## Scoped storage for cron drafts

An owner-installed manifest can request `"storage"` alongside `"ai"`. This grants
its own configuration and data namespaces. `context.config` and `context.data`
each expose asynchronous `get(key)`, `set(key, JSON value)`, and `delete(key)`.
Missing keys return null. Keys are 1–128 UTF-8 bytes; values are at most 64 KiB;
the combined deployment quota is 8 MiB including per-key accounting. Quota failure
rolls back the write. Concurrent sets use last-committed-write semantics.

The host binds the deployment ID to a private per-launch token. Requests cannot
choose another ID. Apps do not receive database files, general filesystem write
access, or provider credentials. The host bounds request size and read/write
waits. Configuration is app-owned configuration, not a secret vault or a way to
grant additional capabilities.

`examples/cron-drafts` demonstrates storing draft source and timezone across
restarts. This stores drafts only; scheduling, publication, task history, and AI
approval workflows remain subsequent milestones. Draft data does not authorize
execution. Keep executable code and accepted revisions outside this mutable store.

## Update, removal, and backup

Updating source at the same canonical deployment path preserves its ID, desired
state, grants keyed to that ID, and storage. A failed prepared build publishes
nothing; build a new artifact directory and validate it before selecting it.
Prepared format 2 includes a digest-verified private Deno executable and can be
relocated. Format-1 artifacts must be prepared again. Execution uses a disposable
copy of the verified dependency cache, `--cached-only`, and a frozen lockfile.

Removing an app from a hosted configuration retires its deployment identity.
Data remains in the database for backup/export, but a later re-add receives a
new ID and cannot silently inherit old data or grants. Stop preserves identity
and data. There is no automatic data purge or scheduler archive operation yet.
Removing a user service removes its registration and preserves state/config/data.

Stop the service before taking an export:

```sh
paraco backup --state /absolute/path/to/runtime-state --output /safe/path/backup.sqlite3
```

The destination must not exist. The export is a private, consistent SQLite
snapshot containing deployment intent, identities, configuration, and data.
It excludes app source/artifacts, external AI configuration/credentials, and
rotating logs; retain those separately as needed. Restore into a fresh private
state directory as `state.sqlite3` while the runtime is stopped, then use the
same canonical configuration/source paths. The migration tests exercise backup
restore. Never replace a live database or restore over a newer schema.

## Browser entry and native operation

`paraco open [--port N]` asks the private same-user control socket for the current
authenticated management URL and launches the default browser. `--print` supports
headless/manual use. Treat that output as a credential. Gateway HTTP requests and
app-scoped capabilities cannot retrieve it. It rotates on restart.

User services are explicit opt-in. Linux uses a systemd user unit; macOS uses a
LaunchAgent in the GUI login domain. Definitions use absolute executable/config
paths and a stable launcher, with logs/state beneath the selected service state
directory. Launchd restarts unsuccessful exits; systemd uses `Restart=on-failure`.
Neither setup enables unattended pre-login operation. Linux lingering requires a
separate administrator decision; macOS LaunchAgents require a logged-in user.

Use `paraco service remove` before replacing an active service registration, then
`service setup` with the selected release. This is an explicit downtime update;
removal preserves app state. Setup refuses foreign definitions before changing
the launcher and restores previous files when manager activation fails. It does
not certify application health or roll back external app side effects.

Native verification uses `scripts/verify-service.py`, with a unique temporary
service name that it removes afterward. Current observations and remaining
machine-restart/platform gates are in [verification evidence](verification-2026-09-21.md).
