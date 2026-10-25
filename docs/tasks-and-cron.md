# Tasks and the cron reference app

Paraco hosts the cron reference app separately from its management dashboard.
The app uses public, deployment-scoped task and AI capabilities; it never opens
Paraco's database. Manual editing and execution require no AI account.

```sh
paraco cron
```

For a source checkout, use `cargo run -- cron`. The released binary embeds the
reference app; no repository checkout or app download is required. Its generated
files use a stable private directory beside the log directory, preserving job
identity across launches and software updates. Edited or unowned generated files
are refused rather than overwritten.

Open the Cron link on the local dashboard. Create a job, save it for review,
inspect the source diff, schedule, timeout and network access, check approval,
and publish. Saving a draft does not execute it. Manual jobs run only when Run
now is selected. Recurring and one-off jobs become eligible after publication.
Paraco and the machine must be running for work to execute.

## Host authorization

The manifest requests `tasks`; the host must independently grant `taskAuthoring`
in its server configuration. The authoring capability can only manage its own
deployment's jobs, revisions, runs and inbox. Each launch receives a private
transport credential. Cross-deployment IDs are rejected by the host.

```json
{
  "apps": [{
    "path": "./cron",
    "taskAuthoring": true,
    "taskNetwork": []
  }]
}
```

`taskNetwork` is the host's maximum outbound network allowlist for tasks. Each
revision requests a subset of these explicit `host[:port]` entries. Empty means
no network. A revision cannot expand the host grant. Adding a host grant requires
editing host configuration and restarting the server. Publishing a revision
with requested network access requires reviewing that exact revision.
The cron app itself does not receive those task network permissions.
Use `paraco cron --allow-task-network example.com:443` for the embedded app,
or configure `taskNetwork` for a separately packaged app.
Tasks are currently hosted through `cron` or `serve`; standalone `run` cannot grant task
authoring. The trust boundary remains owner-installed/reviewed code.

## Task contract

A task is a self-contained TypeScript module exporting a default async function:

```ts
export default async (ctx: {
  log: (...values: unknown[]) => void;
  notify: (message: string) => void;
}) => {
  ctx.log("Starting job");
  ctx.notify("Job reached its checkpoint");
};
```

`ctx.execution` contains stable deployment, job, revision and run IDs. A fresh
Deno process runs every invocation. There is no ambient filesystem, environment,
shell, credential, or unrestricted network permission. A resolved return marks
success; a thrown error or failing process marks error. Timeout, cancellation,
interruption and skipped occurrences have distinct outcomes.

Publication type-checks without executing the module, validates its resolved
module graph, and prepares an immutable offline artifact. This first task
contract accepts self-contained modules; external task imports are rejected.
HTTP applications retain their existing prepared dependency support. Task AI,
configuration/data and native desktop notifications are not exposed by this
initial task adapter; the authoring app has its separately granted AI capability
and notifications persist in the local inbox.

## Revision and scheduling semantics

A draft receives an immutable revision ID and digest. Publication requires that
ID and the expected current published revision (null for a new job). Conflicts
require another review. Edits invalidate browser approval. Restoring an old
revision creates an editable draft; it never rewrites history. Running work
remains pinned to the revision admitted for that run.

Schedules are `manual`, `once` with a UTC epoch millisecond timestamp, or `cron`
with five numeric fields and an IANA timezone. Cron supports wildcard, comma
lists, inclusive ranges and steps; Sunday is 0 or 7. When both day-of-month and
weekday are restricted, either matches. Seconds, named weekdays and named months
are not supported. Preview shows the next five eligible occurrences.

Nonexistent DST wall times are skipped; repeated wall times run once. After
sleep or downtime, missed recurring occurrences are summarized as skipped,
without a catch-up burst. An overdue one-off that has never been admitted is
eligible once. No automatic retries occur. After a crash, guardians reap children
before state ownership is released; unfinished run records become interrupted
and are not silently replayed. External side effects are not exactly-once.

Archive prevents new admissions and cancels pending dispatch. Already running
work finishes unless explicitly cancelled. Unarchive resumes future recurring
occurrences. It does not rerun a completed one-off; Run again is explicit. The
browser warns before unarchiving an overdue one-off.

## Limits and retention

The host allows four concurrent tasks, no overlap within a job, a default
60-second timeout and maximum one-hour timeout. Manual requests while busy fail
clearly; scheduled conflicts record skipped outcomes. Task output is bounded to
64 KiB per run. History uses bounded previews; full retained logs are requested
for an individual run. Output loss and expired content are explicit.

Per deployment, authoring is bounded to 100 jobs, 2,000 revisions and 32 MiB of
revision source. Up to 100 unpublished revisions are retained. Published history
is not silently deleted to make room; capacity errors require operator action.
Each job retains up to 1,000 run records and logs for its latest 100 runs. The
local inbox retains 500 notifications per deployment. Outcome and script events
use stable deduplication IDs. Notification presentation never changes execution
success.

Runtime state, task source, approval metadata, history and inbox share the SQLite
backup. Missing executable artifacts can be recreated offline from the stored
approved self-contained source. Schema version 2 is required; older binaries
must refuse it. Keep a pre-upgrade backup when testing a binary rollback.

## Optional AI drafting

Use `paraco cron --ai-config /absolute/host/ai.json`, or configure `aiConfig`
for a hosted reference app, together with an explicit deployment grant using the existing
[AI routing contract](ai-routing.md). AI receives only the context previewed in
the browser: the user's request and explicitly selected source/log context.
Provider secrets remain host-owned. Applying a suggestion changes only the
editor. It still needs save, review and publish. Late or cancelled responses
cannot overwrite newer edits or publish a revision. AI failure leaves the
published job untouched.

## Verification

`npm run test:cron-runtime` exercises real Deno processes through the public
reference app: publication conflicts, success/error/timeout/cancel, no overlap,
archive, restart, SIGKILL recovery, notification delivery and one-off deduplication.
`npm run test:cron-browser` exercises the browser workflow in Chromium.
`cargo test --bin paraco tasks` covers deterministic clock and state semantics.
Native machine reboot and final signed release installation are separate gates;
these tests do not establish those claims.
