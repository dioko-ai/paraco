# Artifact Notebook

This small app demonstrates portable artifact workflows. It publishes reports,
resolves the latest matching report into an evaluation run, publishes that
evaluation with the report ID in its `inputs`, and shows lineage for either
artifact. The browser list is chronological, and all displayed values use
`textContent`.

The dataset field accepts `private` or a host-granted shared dataset alias.
The host controls which aliases are available; opening an ungranted alias
returns an error. Local and Cloudflare runs use the same `main.ts` handler.

Run locally with `paraco run examples/artifacts`. For Cloudflare, bundle
`worker.ts` through the repository's Worker adapter setup. See
[`docs/platform-evolution.md`](../../docs/platform-evolution.md) for the
platform contract and rollout notes.
