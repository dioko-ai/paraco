# Portable applications, datasets, and output artifacts

Paraco now supplies the initial portable platform foundation on top of the workerd
runtime. Execution target, installation identity, and dataset bindings are separate.
The runnable [Artifact Notebook](../examples/artifacts/README.md) publishes a report,
resolves the latest report into a durable execution input, and publishes an evaluation
with a link to that exact input.

## Implemented milestones

1. **Package compatibility:** optional `targets` and named `outputs` in manifest v1;
   local launch/preparation and `paraco check APP --target cloudflare` reject
   incompatible packages. Existing manifests work. Tasks imply local-only support.
2. **Identity and private persistence:** the existing durable deployment ID is the
   installation ID; manifest `name` is the application ID. Versioned JSON records,
   artifacts, and execution input snapshots are SQLite-backed locally and D1-backed
   in cloud installations. Different installation IDs never share private state.
3. **Explicit sharing:** host-owned aliases grant read or read/write access to logical
   datasets. Local hosts can use an authenticated HTTPS D1 service; no cloud database
   administration credential enters application code.
4. **Composition:** immutable named/dynamic outputs, deterministic discovery,
   permanent/query references, durable resolution snapshots, publication deduplication,
   and forward/reverse lineage. A reusable browser picker is available.
5. **Security and recovery:** record compare-and-swap, host-selected scopes, bounded
   requests/quotas, explicit approval of newly requested local capabilities, and
   SQLite snapshot backup including platform state.

The implementation deliberately uses bounded JSON records rather than exposing SQL.
Artifacts contain inline JSON or strings, including encoded small binary content;
this keeps metadata and content publication atomic. Large R2 objects are not yet
supported. This is an initial contract, not arbitrary SQLite/D1 feature equivalence.

## Application contract

```json
{
  "schemaVersion": 1,
  "name": "researcher",
  "entrypoint": "main.ts",
  "capabilities": ["storage"],
  "targets": ["local", "cloudflare"],
  "outputs": {
    "report": {"mediaType": "text/plain"},
    "findings": {"mediaType": "application/json", "schema": {"type": "object"}}
  }
}
```

`targets` must be nonempty and unique when supplied. Omitted targets mean local and
Cloudflare, except packages requesting tasks default to local. Cloudflare task
scheduling is rejected even if explicitly listed. AI on Cloudflare requires an
operator-provided AI capability service; the supplied D1 binding implements storage.
`check` validates package requirements, not the existence of provisioned infrastructure.
A package can have cloud-only targets; local launch and preparation then reject it.

Output declarations are optional. Declared media types are enforced; schema metadata
is descriptive for consumers, not a JSON Schema validation engine. Undeclared output
categories remain allowed. Product names should stay stable across releases. Local
updates that change the application name cannot reuse the old installation.

With the `storage` capability:

```ts
const dataset = context.datasets!.open(); // private, always installation-scoped
const current = await dataset.records.get('project');
const saved = await dataset.records.put('project', {title: 'New'}, current?.version ?? null);
await dataset.records.delete('project', saved.version);
// Deletion returns a tombstone with a NEW version. Read it before recreating.

const report = await dataset.artifacts.publish({
  output: 'report', mediaType: 'text/plain', content: 'Results…',
  project: 'alpha', tags: ['research'], status: 'complete',
  idempotencyKey: 'execution-123-report'
});
```

Missing records return null; stored JSON null is a record with a version. `put` and
`delete` require `expectedVersion`: null creates an absent key, otherwise the exact
current integer version is required. Concurrent stale writes fail with `conflict:`.
Tombstones prevent delete/recreate from silently admitting an old writer. Retry only
after inspecting the current value; errors never imply a successful write.

Existing `context.config` and `context.data` remain private, last-write-wins stores
for backward compatibility. New shared/concurrent code should use versioned records.
The two old namespaces are never rebound when a dataset is linked. Local quotas for
legacy storage remain 8 MiB combined; the cloud legacy adapter allows 8 MiB per
namespace. Records are not silently copied between the legacy and new APIs.

`context.identity` supplies application/installation IDs in the local adapter and
in cloud storage-enabled installations. A stateless custom cloud wrapper without a
capability service must manage its deployment identity itself. No identity is inferred
from a request header or hostname. The TypeScript API is in `runtime/paraco.d.ts`.

## Private and shared datasets

`open('private')` is always private. `context.artifacts` is shorthand for its artifact
registry. Opening an alias constructs a facade; authorization is checked on every
service operation. An ungranted alias fails rather than falling back to private data.

The operator owns datasets and their grant configuration. A grant permits access to
all records and artifacts within that dataset, across producing applications. Grant
only the collection an application needs; there is no per-row authorization or implicit
cross-app access. Read-only grants permit discovery, retrieval, lineage, and recording
the caller's own execution input snapshots, but not record/artifact writes.

For a **local** host, stop it and create `<state-root>/platform.json`, mode 0600:

```json
{
  "installations": {
    "INSTALLATION_ID_FROM_PARACO_IDENTITY": {
      "datasets": {"research": {"id": "team-research", "write": true}}
    },
    "OTHER_INSTALLATION_ID": {
      "datasets": {"reports": {"id": "team-research", "write": false}}
    }
  }
}
```

Applications use their own aliases (`open('research')` or `open('reports')`). Equal
logical IDs explicitly share one namespace inside this state database. Separate
SQLite state roots stay separate even if labels happen to match. The existing
`paraco identity APP --log-dir LOGS` command reports standalone installation IDs;
hosted `paraco status` reports hosted deployments. Use the actual hosted ID for a
hosted grant, not a standalone ID. State paths are documented in
[durable state](durable-state.md).

To use the **cloud dataset as authority**, add a `remote` member to that installation:

```json
{
  "datasets": {"research": {"id": "team-research", "write": true}},
  "remote": {"url": "https://datasets.example.com/", "token": "INSTALLATION_SCOPED_TOKEN"}
}
```

In this initial adapter, `remote` routes all non-private aliases for that installation
to the service. Private data stays local. The service independently binds that token
to the same installation, product, and dataset aliases/IDs; client claims cannot grant
access. The service also checks the host's expected application, installation, and dataset
identity, so a mismatched token or alias fails closed. Both host and server grants
must allow writes. Tokens belong only in the
owner-only host file and the service's secrets, never application sources or URLs.
HTTPS is mandatory, redirects are refused, requests time out after 30 seconds, and
there is no offline cache or writable fallback for cloud-backed datasets.

Restart to load configuration changes. Removing an alias unlinks access without
copying or deleting data. Changing an alias's dataset ID selects another dataset;
it is not migration. Revocation on the remote service also applies to already-running
clients. Removing an installation does not delete a shared dataset or another
installation's data. Retired private data is retained for backup. Automated dataset
purging/ownership transfer is deliberately absent; the operator retains lifecycle
control. Use new random installation IDs for unrelated installations and preserve
IDs through ordinary updates. Reusing another installation's ID/token transfers its
private authority and is not a supported linking procedure.

## Cloud-only provisioning

The checked-in configuration templates are in `examples/cloudflare/platform/`.
They require a configured Cloudflare account; tests do not deploy resources.

1. Create D1 (`wrangler d1 create paraco-datasets`) and put its database ID in
   `wrangler.service.jsonc`. Apply `runtime/platform-schema.sql` to the database
   using `wrangler d1 execute paraco-datasets --remote --file runtime/platform-schema.sql`
   with the corresponding service config. Keep the checked-in schema as migration 1.
2. Generate a stable random installation ID and a strong random bearer token for
   each installation. Set the service's `PRINCIPALS` secret to a JSON object whose
   token keys map to host-owned principals, for example:

   ```json
   {
     "STRONG_RANDOM_TOKEN": {
       "applicationId": "artifact-notebook",
       "installationId": "STABLE_RANDOM_INSTALLATION_ID",
       "outputs": {
         "report": {"mediaType": "text/plain"},
         "evaluation": {"mediaType": "application/json"}
       },
       "datasets": {"research": {"id": "team-research", "write": true}}
     }
   }
   ```

3. Deploy the service with `wrangler deploy --config wrangler.service.jsonc`.
   The default template has no public route. To connect local hosts, explicitly
   assign an HTTPS route to this authenticated service. Do not bind its DB to an app.
4. Deploy one trusted `cloudflare-binding.ts` worker per cloud installation, setting
   its `TOKEN` secret to that installation's token. It binds `SERVICE` to the dataset
   worker and exposes no public route. `wrangler.binding.jsonc` is a template.
5. Deploy the application with `wrangler.app.jsonc`, binding `PARACO` to its own
   capability worker. Only this binding reaches application code. Preserve
   `disallow_importable_env` and keep DB/secrets out of the application environment.
   The Artifact Notebook is a demo without end-user authentication; configure access
   control before placing private reports behind its public application URL.

`runtime/cloudflare.ts` wraps the same application handler as local workerd. Pass
`['storage']` and the manifest to that wrapper. It verifies declared target/capability
compatibility and creates the same dataset facade. The dedicated binding worker
replaces caller authorization, and the D1 service chooses identity from its secret
principal map. Neither a forged body nor a claimed installation ID selects authority.
Cloud updates cannot gain additional datasets unless the operator changes grants.
The service must have exclusive ownership of these tables: do not deploy arbitrary
SQL APIs alongside it.

The implementation uses D1 prepared statements and atomic batches as documented by
[Cloudflare](https://developers.cloudflare.com/d1/worker-api/d1-database/).
Record updates use one conditional SQL statement, publication uses unique constraints,
and dynamic selection/snapshot insertion is one SQL statement. Read results use the
normal primary D1 binding; a future replica/session adapter must preserve these
read-after-write guarantees. No multi-request transaction is promised to applications.

## Artifact references, ordering, and lineage

```ts
const artifacts = context.datasets!.open('research').artifacts;
const reference = {
  kind: 'dynamic' as const,
  query: {applicationId: 'researcher', output: 'report', project: 'alpha', limit: 5}
};
const inputs = await artifacts.resolve(reference, {
  executionId: 'evaluation-run-123', slot: 'research-inputs'
});
const evaluation = await artifacts.publish({
  output: 'evaluation', mediaType: 'application/json', content: {score: 9},
  inputs: inputs.map(a => a.id), executionId: 'evaluation-run-123',
  idempotencyKey: 'evaluation-run-123-result'
});
const chain = await artifacts.lineage(evaluation.id);
```

A permanent reference is `{kind:'permanent', ids:[artifactId]}`. A dynamic reference
is `{kind:'dynamic', query:{…}}`. Queries accept `applicationId`, `installationId`,
`output`, `project`, one exact `tag`, `status`, and `limit` (1–100, default 20).
Latest means `limit:1`. Results use database publication sequence descending, not
client clocks, so concurrent publishers have a deterministic order. Timestamps are
server metadata in milliseconds. IDs and content never change; revisions are new
artifacts with input links to their predecessors.

`list` previews the current result without freezing it. `resolve` atomically saves
identities under dataset + calling installation + execution ID + input slot. Retrying
the same slot/reference returns the original artifacts, including an originally empty
selection. Reusing a slot with a different reference is an error. Execution IDs are
application-supplied correlation IDs; they do not prove scheduler ownership. The
existing scheduler is not automatically connected to this API. Applications must
resolve once per execution and publish the consumed IDs in `inputs` to create lineage.

All `inputs` must exist in the same authorized dataset. Cross-application lineage is
supported within a shared dataset; cross-dataset lineage is not. `lineage` returns
input artifacts and up to the newest 100 dependents. There is no update/delete artifact
API; database triggers also reject mutations. A repeated publication idempotency key
from the same installation/dataset returns the original artifact only if the full
draft matches. A different draft conflicts. This deduplicates publication, not
arbitrary external side effects; applications must use provider idempotency for those.

For selection UI, import `mountArtifactPicker` from `runtime/artifact-picker.ts` into
a browser bundle and provide an authorized application API implementing `list/get`.
It supports metadata filters, latest/last N, previews, explicit permanent pins, and
saved references. It never needs host credentials. Selection is dataset-scoped: choose
the authorized dataset in the application and supply its API. Chronological browsing
and lineage details are demonstrated by Artifact Notebook.

## Limits, updates, and recovery

Platform requests are at most 64 KiB encoded JSON, including metadata. Identifiers
are 1–128 UTF-8 bytes without control characters; tags and inputs are bounded at 100.
Each dataset has 8 MiB of record storage (including key/row overhead and tombstones)
and 64 MiB of artifact storage (including metadata and idempotency fingerprints).
Each dataset/installation has at most 10,000 saved execution input slots. Quota
violations roll back the statement. Transport responses are bounded at 16 MiB.
There is no automatic retention deletion. Plan archival/new datasets before filling
an append-only dataset; large-file object storage and retention controls remain work
for a later milestone.

Schema v3 migrates v1/v2 state additively and preserves deployment IDs and existing
storage. Older binaries must refuse v3. On the first launch after upgrade, existing
installations establish their capability baseline from the current owner-installed
manifest. Later updates requesting extra local capabilities fail before launching.
After reviewing the changed manifest, stop the runtime and explicitly run:

```sh
paraco approve-capabilities /absolute/app --state /absolute/state-root
```

That command approves the current capability set for that source/installation. It
does not grant AI providers, task network destinations, or shared datasets; those
remain separately configured by the host. Removing a manifest capability stops exposing
it; use the approval command with the reduced manifest to also reduce the stored
baseline. No app request can alter the baseline.

`paraco backup --state STATE --output NEW_FILE` includes local records, artifacts,
resolution snapshots, and grant baselines. Restore into a fresh stopped runtime state
root as `state.sqlite3`, preserving source paths as described in durable-state docs.
`platform.json` is intentionally excluded because it may contain credentials; back
it up separately with owner-only protection. A local backup does not snapshot remote
D1. Use D1's export/recovery tooling for that authority and preserve principal secrets
separately. Never mistake a network error for a completed mutation: retry record CAS
or the same publication idempotency key and inspect the result.

## Verification and remaining scope

Rust regression tests exercise isolation, grants, CAS/tombstones, publication identity,
discovery, snapshots, lineage, migration, capability approval, and backup/restore.
`npm run test:platform` runs the D1 statements against real SQLite through a D1-shaped
adapter, including concurrent writes and unauthorized bodies. Browser picker and
local workerd demo tests cover actual consumers. These do not constitute a live
Cloudflare deployment or production load test.

Deferred as in the plan: bidirectional offline synchronization, database replication,
distributed scheduling, artifact-triggered tasks, visual workflow builders, vector
search, and other cloud providers. Rich dataset administration/ownership transfer,
large R2-backed artifacts, automatic retention, and cross-dataset lineage remain
subsequent features. Local scheduling and its execution history stay installation-
scoped even when datasets are shared.
