# Prepared application artifacts

`paraco prepare APP --output ARTIFACT --workerd /absolute/path/to/workerd` copies
an application into a new host-owned artifact, validates `paraco.json`, and
uses bundled esbuild to compile its TypeScript entrypoint and local imports to
browser ES modules. Preparation records the pinned workerd runtime but never
starts the application entrypoint. The output directory must not already exist:
preparation stages the artifact and publishes it only after compilation and
validation succeed.

Run it with `paraco run-prepared ARTIFACT --port 3000`. Format 3 records the
source tree and `app.js` under `code/`, along with a hash of the private runtime
at `runtime/workerd`. Launch verifies those hashes and uses the compiled app;
it does not invoke the compiler. Keep the artifact directory private to the
host and do not edit it after preparation.

Preparation accepts the app entrypoint declared by `paraco.json` and bundles
its local imports into browser ES modules. Symlinks, special files, and source
modules that resolve outside the app tree are rejected. The existing
`paraco run APP` command remains a development path and is not a substitute for
prepared deployment.

Format 3 is relocatable and contains no compiler. Older prepared formats require
preparation again. Execution never writes into the verified artifact. The
prepared-artifact verification script checks relocated launches and preservation
of the previous revision after failed preparation.

Prepared artifacts contain a native executable and are trusted host-generated
outputs, not marketplace source packages. Their self-recorded hashes detect
changes; they do not authenticate a downloaded runtime or publisher. Use verified
Paraco releases and the approved source-package installation path for distribution.
