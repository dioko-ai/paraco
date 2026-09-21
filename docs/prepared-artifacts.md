# Prepared application artifacts

`paraco prepare APP --output ARTIFACT --deno /absolute/path/to/deno` copies an
application into a new host-owned artifact, validates `paraco.json`, creates a
Deno lockfile, and caches its dependency graph. Preparation never starts the
application entrypoint. The output directory must not already exist: preparation
uses staging and only publishes it after Deno succeeds.

Run it with `paraco run-prepared ARTIFACT --port 3000`. Prepared execution
checks the artifact metadata/source digest and Deno version, sets `DENO_DIR` to
the artifact cache, and invokes Deno with `--cached-only --lock`; it cannot
implicitly download dependencies. Keep the artifact directory private to the
host and do not edit it after preparation.

The supported Deno configuration is deliberately limited to an app-root
`deno.json` containing only string `imports` aliases. Symlinks, special files,
and other Deno configuration fields are rejected. The existing `paraco run APP`
command remains a development path and uses its own temporary cache; it is not
a substitute for a prepared deployment.
