# Prepared application artifacts

`paraco prepare APP --output ARTIFACT --deno /absolute/path/to/deno` copies an
application into a new host-owned artifact, validates `paraco.json`, creates a
Deno lockfile, and caches its dependency graph. It then asks Deno for the
resolved graph and accepts file modules only when their resolved paths remain
inside the copied application tree; HTTPS and JSR modules are recorded in the
lock/cache. Preparation never starts the application entrypoint. The output
directory must not already exist: preparation uses staging and only publishes it
after Deno succeeds.

Run it with `paraco run-prepared ARTIFACT --port 3000`. Prepared execution
checks source, lock, cache, and private-Deno digests plus the Deno version,
copies dependencies into a disposable per-launch cache, and invokes Deno with
`--cached-only --frozen --lock`; it cannot
implicitly download dependencies. Keep the artifact directory private to the
host and do not edit it after preparation.

The supported Deno configuration is deliberately limited to an app-root
`deno.json` containing only string `imports` aliases. Symlinks, special files,
and other Deno configuration fields are rejected. The existing `paraco run APP`
command remains a development path and uses its own temporary cache; it is not
a substitute for a prepared deployment.

Artifact format 2 carries Deno at `runtime/deno`, relative to the artifact. It
can be relocated without the preparation machine or its runtime installation.
Format-1 artifacts require preparation again. Execution never writes into the
verified dependency cache. `scripts/verify-prepared.py` checks repeated relocated
launches and preservation of the previous revision after failed preparation.
