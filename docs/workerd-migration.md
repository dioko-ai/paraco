# workerd migration: implementation and release gates

The supported execution path now uses Rust, workerd 2026-10-26 and esbuild
0.28.2. Deno is not a runtime, build or test dependency. Old verification records
remain historical evidence, not evidence for this migration.

## Application and deployment contract

An application declares `paraco.json` and exports an object with
`fetch(request, context)`. Requests and responses use Web APIs. Rust constructs
`context` from the manifest and host grants: `basePath`, optional `ai.complete`,
`config`/`data` storage, and `tasks.command`. Provider credentials stay in Rust.
App declarations cannot create AI credentials or task-authoring grants.

Source applications are compiled to browser-targeted ES modules. No ambient
Node/Deno globals, filesystem access, subprocess API, runtime dependency downloads,
or top-level I/O is supported. The application compatibility date is deliberately
`2026-08-03`, before automatic Node compatibility. `disallow_importable_env`
prevents application imports or globals from exposing adapter bindings. This is
separate from the pinned runtime version. See [compatibility flags](https://developers.cloudflare.com/workers/configuration/compatibility-flags/). Include dependencies in the package and import
static assets with `with { type: 'text' }`. esbuild checks syntax and resolves the
bundle; it does **not** perform TypeScript semantic type checking. Developers
should run their own `tsc` checks. Linux app compilation uses a private copied
source tree and a separate namespace sandbox, so imports cannot read user files.

Prepared format 3 packages contain source, compiled `code/app.js`, and
`runtime/workerd`. SHA-256 digests are verified before launch. Older artifacts
must be prepared again. Source identity and Rust-owned SQLite state are retained;
changing execution runtimes does not create new installation identities.
Marketplace catalogs retain their host-approved digest and capability contract.
A source package must satisfy the new Web API and bundling requirements.

`examples/cloudflare` wraps the exact `examples/hello` handler for Workers.
`runtime/cloudflare.ts` defines the optional deployment-owned `PARACO` service
binding transport. A production Cloudflare capability backend (including D1)
and live deployment verification are still outstanding. The example is not proof
of deployment. Kubernetes and Lambda adapters are intentionally deferred.

## Host and workerd boundary

Each app has its own guardian and workerd process. Rust opens the listener and
passes its descriptor into workerd, avoiding ephemeral-port allocation races.
Readiness requires a successful private HTTP probe, not an application log line.
The guardian watches a host lifetime pipe and terminates/reaps the runtime after
host death. The gateway continues to provide independent app origins, lifecycle
controls and backend authentication. Logs retain the guardian PID; the inner
sandboxed process has a different PID.

workerd has explicit `globalOutbound` restrictions. It receives no disk service
or provider secrets. The host adapter has an authenticated HTTP service binding
over a private Unix socket. Rust translates that connection to the existing
per-deployment AI, storage and task services. Applications receive methods, not
service addresses or credentials. A loopback connection alone does not grant
access to a capability.

Tasks remain reviewed, immutable revisions with Rust-owned schedules, histories,
notifications and restart/interruption handling. Each execution gets a fresh
workerd process and one bounded HTTP invocation. The host enforces the existing
100 ms–1 hour wall deadline and concurrency limits, kills stalled executions,
and retains failure outcomes. Self-contained TypeScript is transformed without
resolving imports. Older cached task sources are rebuilt on first execution.
Long work must await completion inside the task invocation; detached background
work is not durable. Cloudflare duration limits are a separate deployment concern.

Task networking is mediated by Rust against the published revision's subset of
host grants. Each redirect is checked separately; ambient proxies are disabled.
Request bodies are limited to 64 KiB and responses to 1 MiB. Non-HTTP networking
is unsupported. Exact authorized hosts can include private services, so operators
must treat those grants as access to the service, not merely internet access.

## Isolation and limitations

Native policy construction now lives in platform containment adaptors and covers
task compilation as well as app compilation and runtime execution. The bundled
`paraco sandbox-probe` app displays independently collected native canary results
alongside guest checks. See [native containment](native-containment.md) for the
experimental macOS prototype, exact probe scope and validation instructions.

Cloudflare explicitly states that standalone workerd is not a complete sandbox
for malicious code. See the [workerd security notice](https://github.com/cloudflare/workerd#security)
and [configuration schema](https://github.com/cloudflare/workerd/blob/main/src/workerd/server/workerd.capnp).

Linux execution defaults to bubblewrap user, PID, mount and network namespaces.
The runtime sees its launch directory, public runtime libraries/certificates,
private temporary storage and minimal process/device mounts. It receives an
already-open application listener and its own capability Unix socket. It cannot
open a network route from the namespace or see the user's home/state directories.
No fallback occurs if containment cannot start. The system must provide
`/usr/bin/bwrap` and allow unprivileged namespaces. Open descriptors are bounded.

`PARACO_ALLOW_UNSANDBOXED=1` is an explicit **trusted development only** escape
hatch. It is not a marketplace security mode. macOS currently fails closed unless
this development opt-in is set: a tested native/VM containment adapter is still
required. Linux namespaces do not protect against kernel exploits or establish
Cloudflare's production security properties. Hard per-app memory and CPU quotas
(cgroups), denial-of-service tests, and broader sandbox auditing remain release
gates. Do not distribute this build as a hardened hostile-code host yet.

## Verification observed on Linux

The combined `npm run check` command passed. Its [retained log](../verification-artifacts/2026-10-26-workerd/check.log) and [summary](../verification-artifacts/2026-10-26-workerd/summary.json) distinguish verified checks from open gates.

- Rust library/binary suite: 85 tests passed with local socket access.
- Standalone runtime integration: 18 tests passed. Hosted-app integration: 34
  tests passed, including recovery, isolation and cleanup after forced host death.
- Real workerd task suite: publication, scope, success/error/timeout/cancel,
  notifications, archive, one-off execution, restart, forced host death and
  database-only restoration passed.
- Prepared package relocation: two launches with empty PATH, source removed,
  failed-preparation preservation and clean shutdown passed.
- Foundation workload: 1, 10 and 50 sandboxed apps served all 72 requests each;
  observed startup approximately 0.21, 0.61 and 1.80 seconds. Aggregate process
  RSS at readiness was about 80 MiB, 636 MiB and 3.04 GiB respectively. These are
  single-host observations, not service guarantees.
- Durable storage survived forced host death; cross-app storage isolation,
  stopped intent, stable identities and control URL rotation passed.

- Native archive installed and served the example using its private workerd and
  esbuild. The installed cron suite passed with an empty developer-tool PATH.
- Browser dashboard and cron lifecycle checks passed. Security probes verified
  denied networking/redirects, hidden bindings and rejected outside-package imports.
- Cloudflare entry point passed locally using the same core handler. Live
  deployment was deferred at the user's request and is tracked in `TODO.md`.

The migration is not release-complete until macOS containment, hard resource
quotas, remaining native-platform checks and the deferred cloud checks have
evidence. No public installer or release was published.
