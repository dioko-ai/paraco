# Foundation pivot and first useful application

Status: proposed implementation plan, 2026-09-18. This document plans future
work; it does not describe shipped functionality. It translates the
[foundation audit](audit-2026-09-18.md) and the owner's product decisions into
ordered, verifiable milestones. The product principles in `base_spec.md` and
`spec.md` continue to apply.

## Confirmed direction

- First release: Linux and macOS.
- CLI installation; everyday operation through the browser.
- Permissive open source, allowing commercial reuse and closed-source forks.
- A community marketplace is a future objective, not a launch prerequisite.
- The first useful app manages one-off and recurring TypeScript cron jobs, shows
  logs and outcomes, retains code history, and uses Paraco notifications.
- AI can create and rewrite jobs. A person must accept a proposal before it
  becomes a published version eligible for execution.
- Published jobs are active or archived. Archiving turns a job off; unarchiving
  turns it back on. There is no additional enabled/disabled toggle.
- No deadline or existing app/data preservation constraints were identified.
  Access to particular native CI machines has not been established.

Keep Rust, supervised Deno, portable handler contracts, and cloud independence.
Use explicit draft APIs while contracts evolve. Do not add another execution
engine, cloud orchestration, or marketplace implementation in this plan.

## Proposed defaults

These resolve unspecified behavior for planning; they are not additional owner
requirements. They can be changed before the affected milestone.

- Target x86-64 and ARM64 for both operating systems, but publish support only
  for combinations tested natively. Establish minimum OS/libc versions in M0.
- Launch accepts owner-installed/reviewed code. AI approval means authorization
  to execute, not proof that generated code is safe. Implement browser isolation
  and scoped grants now; hostile third-party hosting needs another security gate.
- Recommend Apache-2.0. It includes an explicit contributor patent grant and
  redistribution conditions; MIT is a shorter permissive alternative. Record
  the exact choice before adding LICENSE, rather than treating “permissive” as
  an already selected text. See the official
  [Apache-2.0 license](https://www.apache.org/licenses/LICENSE-2.0.html) and
  [MIT license](https://opensource.org/license/mit).
- Use a host-owned SQLite database for runtime state, transactional scheduling,
  and task metadata, with versioned migrations. Keep executable artifacts,
  application data, and credential storage logically separate.
- A job is a bounded, fresh Deno task process. It is not a long-running process
  that independently watches a clock. A portable task exports an async handler
  receiving scoped capabilities and execution metadata.
- Notifications first provide a durable local inbox. Add native desktop delivery
  as a best-effort presentation channel; headless users still receive inbox events.

## Product behavior for the cron app

### Jobs, versions, and approval

The app has an active list, an archive, an editor, version history, execution
history, and a notification view. The active list distinguishes scheduled,
currently executing, and completed one-off jobs without introducing a pause flag.

Creation and editing produce drafts, either manually or with AI. Drafts are not
scheduled jobs and cannot execute automatically. For an existing job, its current
published version continues running while a new draft is reviewed. New AI
proposals never replace it automatically.

Each proposed revision records source, dependency lock/artifact, schedule,
timezone, requested capabilities, and execution limits. The review screen shows
the code diff, schedule change, and any new access. “Accept and publish” refers
to that exact immutable revision. Editing after review invalidates that approval.
Dependencies must prepare and validation must pass before activation.

Publication atomically activates the prepared revision and its schedule. Use an
expected-current-revision check so concurrent tabs or late AI responses cannot
overwrite newer work. Authorizing a code version does not silently grant broader
credentials or host access; new grants require explicit user action in the same
review workflow. An archived job remains archived when a revision is published.

Every execution pins its revision when admitted. Publishing new code never
changes an execution already running. Restoring an old version creates a new
reviewable revision referencing that content; history is not rewritten. Retain
published source and dependency identity independently of rotating execution
logs. Unpublished AI drafts have a separate bounded retention policy.

### Archive and execution semantics

| Action or condition | Proposed behavior |
| --- | --- |
| Publish a new job | Becomes active with the accepted schedule; an immediate one-off is admitted once |
| Archive | Atomically prevents new scheduled/manual admissions and cancels pending dispatch; an already running execution finishes |
| Cancel execution | Separate explicit operation; terminates that run without changing archival state |
| Unarchive recurring job | Schedules the next future occurrence, without replaying archived time |
| One-off completes | Remains visible with its outcome; it has no further scheduled occurrence |
| Unarchive completed one-off | Does not rerun it; use an explicit Run again action |
| Unarchive overdue, never-started one-off | Shows that it will run once now before confirming unarchive |
| Run now / Run again | Explicit invocation of the published revision of an active job; does not consume a future scheduled occurrence |
| New revision while running | Applies to future admissions; current run remains pinned |

Archived jobs preserve code, configuration, and retained history. Permanent
deletion is a separate explicit operation and is not needed for the first working
slice. The UI must explain that “active” means eligible to run, not continuously
executing. A completed one-off is not automatically archived.

### Schedule and failure semantics

- Start with five-field cron syntax and a required IANA timezone; prefill the
  machine's zone and display it. Preview upcoming run times. Specify supported
  syntax and day-of-month/day-of-week behavior in the public contract.
- Proposed DST behavior: skip nonexistent wall-clock occurrences; run a repeated
  wall-clock occurrence once. Test this with a controllable clock and explicit
  occurrence identities, not assumptions about timestamp uniqueness.
- After downtime/sleep, skip missed recurring occurrences and record a bounded
  summary. An active overdue one-off that was never admitted runs once on resume.
  Re-evaluate on clock changes; do not execute a burst of missed work.
- No overlap per job initially. A recurring occurrence due while a run is active
  is recorded as skipped. Manual execution while busy is rejected clearly.
- Global concurrency, run duration, log volume, and queued work are bounded.
  Defaults are visible and configurable. No automatic retry initially: scripts
  may cause external side effects. Later retries require an explicit policy.
- Use unique durable occurrence IDs and transactional claims. After a runtime
  crash, reconcile child ownership before accepting more work. An admitted run
  whose result cannot be established is marked interrupted/unknown and is not
  automatically replayed. Do not promise exactly-once external side effects.
- Distinguish success, error, timeout, cancellation, interruption, and skipped
  occurrence. Return/resolution marks success; an uncaught error or unsuccessful
  process exit marks failure. Store timestamps, revision, and bounded diagnostics.

Paraco cannot execute while the machine is off. The app must explain this and
show the last scheduler activity and the next eligible occurrence.

### Logs, notifications, and AI

Each run has a stable ID, job/deployment/revision identity, timestamps, outcome,
stdout/stderr, and lifecycle events. Browser users can inspect current and past
runs. Store outcome metadata separately from log content so rotation cannot erase
whether a run succeeded. Show truncation, dropped output, and expired log content
explicitly. Provide per-deployment retention limits so a noisy app cannot evict
every other app's logs. Define defaults and pruning tests in M5.

Expose a scoped notification capability for scripts. Paraco also supports
user-selected success/failure notifications, deduplicated by execution/event ID.
Notification delivery failure does not retroactively fail successful work; it has
its own delivery status. Do not put secrets or full logs in desktop notifications.

AI generation uses the existing host-owned AI policy and user-granted credentials.
Keep provider credentials out of scripts and browser storage. Send source or logs
to a provider only as part of the user's requested generation/rewrite/debug action;
make the submitted context visible and do not include secrets automatically.
Generation failure leaves the active job untouched. Manual editing and execution
remain fully usable without AI credentials or external connectivity.

## Runtime boundary

| Paraco runtime/framework owns | Cron application owns |
| --- | --- |
| Deployment identity, grants, configuration and data capabilities | Job editor, list/archive views, schedule preview and review UX |
| Dependency preparation and immutable executable artifacts | Drafts and AI editing conversations through scoped storage |
| Reusable task definitions/revisions, schedule activation and run records | Creating/publishing its owned tasks through the framework API |
| Clock evaluation, durable claims, dispatch and bounded task processes | Presenting version history, run results and logs |
| Logs, notifications, credential proxy and authorization | Asking the user for AI assistance and displaying proposals |

The cron app is a separately packaged reference app using public capability APIs.
It does not read Paraco's internal database, contain its own scheduler daemon, or
receive unrestricted access to other deployments' tasks. Its task-authoring grant
is explicit, scoped to its deployment, and cannot mint broader task grants.
Server-side publication authorization must not rely solely on disabled UI buttons.

Introduce stable runtime, deployment, task, revision, occurrence, and execution
IDs. Names and URL labels are mutable display fields. Scope grants/data to stable
IDs. Centralize published task/schedule metadata in one runtime transaction so
application storage and scheduler state do not need a distributed transaction.
Keep drafts in app storage; use an idempotent publication request to bridge them.

Portable task code uses scoped configuration/data, logs, notifications, AI where
granted, and explicit outbound-network access. No ambient filesystem, shell, or
credential access. Validate dependency and network grants at preparation and
execution. Host-specific capabilities can follow with compatibility declarations.

Keep one Rust crate initially. Extract lifecycle/domain logic from HTTP handlers,
and introduce small modules for platform integration, state, artifacts, tasks,
scheduling, and capabilities as those milestones require them. Avoid a speculative
plugin framework. Share TypeScript types and contract fixtures across HTTP/task
adapters; runtime validation remains necessary for untyped inputs.

## Ordered milestones and exit criteria

### M0 — Contracts and repeatable verification

Work:

- Record platform baseline, trust policy, chosen license, stable IDs, and manifest
  schema version in short architecture decisions. Add LICENSE/metadata when the
  exact license is selected, plus contribution and security-reporting guidance.
- Pin tested Rust/Deno versions, declare Rust compatibility, and create native
  Linux/macOS CI. Isolate all tests' control endpoints and temporary roots.
- Add JSON schema, shared TypeScript types, format/lint/type checks and a small
  browser test setup. Correct stale feature documentation.

Exit: a fresh checkout has documented repeatable checks; both OS families execute
real launch/request/shutdown tests. Any unavailable architecture is explicitly
unverified, not silently declared supported. Existing Linux checks remain green.

### M1 — Lifecycle hardening and browser isolation

Work:

- Fix blocked stdout/stderr shutdown using bounded output handling and loss
  reporting. Keep child pipe draining independent of slow presentation sinks.
- Establish cross-platform child ownership, parent-death cleanup, single-runtime
  ownership, and safe stale-socket recovery without replacing a live instance.
- Choose stable per-deployment browser origins before adding app browser data.
  Prototype local hostname resolution and cookie isolation on the target browser/OS
  matrix; ports alone are not a sufficient cookie policy. Reject shared mutable
  cookie domains. Keep management separate and backends inaccessible as an
  unauthenticated bypass of gateway policy. If a resolver/certificate setup is
  needed, make it an explicit installation decision.
- Fix canonical host behavior and verify origin, CORS, CSRF, service workers,
  browser storage, management authorization, and cross-app access in browsers.
- Measure 1/10/50 idle apps and noisy-output workloads to establish budgets.

Exit: forced parent death and blocked sinks cannot leave managed work indefinitely
alive; restart recovers without manual socket deletion. App A cannot read app B's
browser data or authenticated responses. Management remains responsive under load.
Record precisely which trusted-code limitations remain.

### M2 — Prepared apps and a minimal release bundle

Work:

- Separate explicit prepare/install from execute. Support a documented configuration
  subset, import aliases, locked dependencies, artifact digests, and host-owned
  dependency storage. Execution uses a prepared artifact without implicit downloads.
- Locate an exact private Deno relative to the bundle, including launcher symlinks;
  keep development overrides explicit. Do not fall back silently to system Deno.
- Build a portable archive per verified target with notices, version metadata,
  checksums, and recorded source revision. Keep installers for M8.

Exit: a dependency-bearing app launches offline from the archive without Rust,
Node, or system Deno. Changed working directories and unrelated PATH runtimes do
not affect it. Failed preparation leaves the installed revision usable.

### M3 — Durable state and service lifecycle

Work:

- Add transactional state/migrations, stable IDs, persistent desired state,
  scoped grants, application configuration/data storage, and backup/export.
- Define install/update/remove and data-preservation behavior. No prior user data
  migration is required, but migrations become tested contracts from this point.
- Reconcile desired state after restart; never treat stored PIDs as ownership proof.
- Add explicit opt-in Linux and macOS service setup, using their native service
  managers. Define user-login versus unattended operation on each platform.
- Add a discoverable `paraco open` or equivalent authenticated browser entry flow;
  background users must not retrieve rotating bearer links from service logs.

Exit: stop/archive intent and data survive runtime/machine restart. Service shutdown,
unexpected death, backup restore, failed migration, and update failure have tested
outcomes. Browser management is accessible after restart without exposing tokens
to apps or unauthenticated requests.

### M4 — Bounded TypeScript task execution

Work:

- Add the task entrypoint alongside HTTP fetch. Share artifact, grant, logging,
  and process ownership infrastructure, while keeping task completion distinct
  from a long-running HTTP app's crash/restart policy.
- Implement immutable revisions and publication, scoped task authoring, run IDs,
  timeouts/cancellation, network grants, and concurrency limits.
- Validate code/dependencies without executing the job's top-level code in the
  privileged host. A draft test run, if later added, is a separate explicit action
  with the same grant and side-effect disclosure as real execution.

Exit: a manually invoked published task succeeds, fails, times out, and cancels
correctly; access is denied outside its grants. Concurrent publication cannot
change its pinned source. Other apps and management remain responsive.

### M5 — Durable scheduler, history, and notification capabilities

Work:

- Implement the schedule/archive semantics above using transactional activation,
  occurrence claims and restart reconciliation. Avoid separate duplicate timers
  inside each app. Expose deterministic clock inputs for scheduling tests.
- Add run-history queries, retention limits, per-run log viewing, and durable
  notification inbox/events; native delivery follows behind the same interface.
- Test DST transitions, sleep/wake, clock changes, overdue one-offs, overlaps,
  archival races, and crashes between claim, spawn, side effect, and completion.

Exit: each scenario produces the documented next occurrence and outcome; no
unbounded catch-up/retry occurs. Archived jobs do not admit new work. Notifications
are deduplicated and independent of execution success. Uncertain outcomes are
visible and never silently replayed.

### M6 — Cron app without AI

Work:

- Package the reference app using the public storage/task/scheduler/log/notification
  contracts. Add manual editing, schedule preview, review/publish, archive/unarchive,
  Run now, cancellation, version restoration, and run history.
- Give users readable validation errors, capability prompts, next-run status,
  empty states, keyboard access, and clear offline/runtime-stopped indications.
- Run full browser workflows against real runtime processes on both platforms.

Exit: a user creates and publishes a recurring task, receives a notification,
inspects success/error logs, publishes another revision, restores a prior version,
archives/unarchives, restarts the service, and retains correct state/history.
One-off completion and unarchive do not accidentally schedule duplicate work.

### M7 — AI drafting with explicit publication

Work:

- Split AI authorization/routing, provider execution, and transports. Add one
  real provider, host-owned credential storage, bounded concurrency, cancellation,
  error handling and streaming deadlines suited to generation.
- Implement a narrow documented SDK/HTTP subset on the same policy core. Test
  denied grants, disconnects, malformed/partial streams, and secret redaction
  using deterministic provider fixtures; paid calls are not required in CI.
- Add create/rewrite flows, context selection, streamed drafts, code/schedule/grant
  diffs, and explicit revision-bound acceptance. Cancellation preserves the current
  version. Source, logs, and model output cannot authorize publication.

Exit: AI can propose a job or rewrite, but no proposal executes before acceptance.
Stale approvals, concurrent tabs, and late responses cannot replace newer code.
Generation is optional; the complete manual app still works offline.

### M8 — First release and installation

Work:

- Add the CLI installer around verified archives, explicit service opt-in, a
  first-run browser flow, and clear support/minimum-version documentation.
- Verify upgrades, rollback, removal and data preservation; reject incompatible
  schema downgrades clearly. A binary rollback must not corrupt newer state.
- Establish artifact authenticity, signing/notarization requirements as applicable,
  third-party notices, dependency vulnerability review, and release provenance.
- Validate desktop and headless use, basic accessibility, resource budgets,
  diagnostics/export, and recovery instructions on clean target machines.

Exit: a fresh Linux/macOS user installs Paraco, completes the cron workflow,
restarts, upgrades, and removes software without losing data or leaving processes.
Support claims match actual native evidence. Homebrew/native graphical packages
can follow after the direct installation path is proven.

## Scope and sequencing controls

Implement each milestone as small reviewable changes with its own acceptance
checks. M0 through M6 are the core dependency sequence; AI proxy work can begin
after M3, but AI product acceptance depends on the M6 publication workflow. Do not
replace scheduling/state work with additional dashboard polish or providers.

Before a marketplace: separately review hostile-code isolation, resource quotas,
artifact provenance, publisher identity, grant escalation during updates, browser
boundaries, and revocation. Marketplace readiness is not implied by passing the
trusted-code launch gates. Leave interfaces extensible without building these
systems prematurely.

No calendar estimate is assigned without execution evidence and team/CI capacity.
The first implementation slice is M0 plus the isolated blocked-output regression
and fix from M1. License selection, native CI availability, and the origin prototype
are early decisions to resolve; they do not block unrelated lifecycle fixes.
