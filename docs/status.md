# Implementation and verification status

This is the current status record. `spec.md` describes project intent; dated
audits record historical findings. Milestone plans describe future work and do
not establish verification evidence.

## Audit repair scope — 2026-09-21

The repair retains the Rust supervisor, embedded Deno adapter, and plain
JavaScript dashboard. It adds no account requirements, automatic credential
grants, feature flags, or cloud services.

| Audit finding | Repair |
| --- | --- |
| 1. Launch bootstrap stalls | Share the guardian bootstrap delimiter and exercise public launch/request/shutdown. |
| 2. Runtime resolution | Resolve executable PATH entries to absolute paths; recognize bundle metadata explicitly. |
| 3. Unusable AI grants | Persist host-owned standalone identities and expose identity discovery before granting credentials. |
| 4. Provider networking panic | Share an I/O-enabled runtime and provider admission; align client deadlines and test a local provider fixture. |
| 5. Bundle contract mismatch | Use the builder's complete metadata and target-qualified archive root throughout installation and launch. |
| 6. Drifted acceptance fixtures | Compile all tests and discover deployment identities/URLs through public interfaces. |
| 7. Dashboard app navigation | Permit safe top-level document navigation while rejecting cross-origin subresource requests; click links in Chromium. |
| 8. Name-based deployment identity | Bind host identities to canonical source locations, retire removed sources, and preserve identity across ordinary source updates. |
| 9. Failed persistence changes memory | Commit proposed state before publishing it; synchronize writes and remove unused app storage scaffolding. |
| 10. Source substring import checks | Validate Deno's resolved dependency graph instead of scanning application text. |

Identity state now uses format 2 in separate namespaces: hosted state is keyed
by canonical server-config location, and standalone state by canonical app
location. The prior shared name-keyed state is not reused or automatically
migrated; copying a format-1 file into a new namespace is rejected. Existing
installations therefore receive fresh IDs and initially running desired state.
Retain the old state before upgrading, rediscover IDs, explicitly update grants,
and reapply any intended stopped state. Editing code at the same canonical
source within a new namespace preserves its identity. Use a different source
location, or remove it from a hosted configuration and restart before re-adding
it, for a fresh installation.

Deployment state currently uses one JSON store. Before scheduling is added,
migrate that store and scheduling metadata together to SQLite; do not introduce
more custom stores. Application storage remains deferred until a concrete app
interface and use case are implemented.

## Verification

Observed on Linux x86-64 with Rust 1.96.1, Deno 2.2.5, Node 22.22.3,
and Playwright 1.52.0 / Chromium 136:

- `npm run check` passed: formatting, strict Clippy for all targets, 110 Rust tests,
  schema/type declarations, management JavaScript, and real Chromium navigation.
- `npm run test:installer` passed, including hostile archive-path rejection.
- `tests/bundle-smoke.sh` built a release archive, installed it, served the hello
  response through the installed bundled runtime, and verified shutdown.
  This local test used fixture notice inputs and is not a publishable release.
- Standalone, hosted, and prepared launches served real Deno requests; tests
  exercised explicit grants, stop/restart, crash recovery, and stubborn cleanup.
- A local HTTPS provider fixture exercised actual Reqwest completion and SSE
  through the authenticated capability transport with certificate validation.
- Shell syntax and `git diff --check` passed.

Independent core checks do not require Deno or Chromium. The complete CI job
retains exact Node/Deno pins; the Linux job also runs archive composition.

Native macOS, additional CPU architectures, service-manager installation,
signing/notarization, and clean-machine package publication remain separate
release checks. Local fixture provider tests do not make paid-provider requests.
