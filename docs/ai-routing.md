# Local AI routing core

This increment adds `paraco::ai`, an in-memory Rust policy core with a deterministic
fake backend. It makes no network requests and needs no credentials or accounts.
It is not yet connected to the CLI or Deno host: manifests requesting `ai` still
fail as unsupported until that capability is wired end to end.

The trusted host constructs `Config` and supplies the caller identity separately
from `Request`. An app request contains only optional provider/model selection
and a prompt. This module does not authenticate callers; the future transport
must derive caller identity from authenticated host-issued access, not an app's
claimed name.

Configuration contains credential identities and their owning providers, allowed
routes, per-app capability requests and credential grants, app/runtime defaults,
and provider default models. It contains no real secret values. Unknown credential
references and routes referencing another provider's credential are rejected at
construction. Configuration is immutable once the proxy is constructed.

Requesting AI and receiving a credential grant are separate requirements. A grant
allows that app to use configured routes referencing that credential. Apps cannot
supply or change grants through a request.

| Request | Resolution |
| --- | --- |
| Neither provider nor model | First permitted configured app default, then runtime default; otherwise fail |
| Provider only | That provider's configured default model, with an authorized route |
| Provider and model | That exact pair, with an authorized route; no substitution |
| Model only | Exactly one provider among the caller's authorized configured routes; ambiguity requires a provider |

An unavailable or ungranted automatic app default can fall back to the runtime
default. Explicit selections never fall back. Multiple authorized credentials for
one provider/model pair do not make a model ambiguous: the first permitted route
in configuration order wins. No arbitrary provider is selected when defaults are
missing. Deployment-specific defaults are deferred until deployments exist.

Success returns the actual provider/model and the constant `Fake AI response`.
The fake backend does not interpret or echo prompts. Responses and error messages
do not contain credential identities or secrets. The core performs no logging;
request and response types do not derive `Debug`, avoiding accidental content
logging through that mechanism.

Run the offline policy tests with:

```sh
cargo test --offline --test ai
```

Tests cover every selection form, default precedence and fallback, absent routes
and defaults, ambiguous models, duplicate provider/model routes, blank selections,
invalid configuration, unknown callers, capability/grant separation, cross-app
credential denial, and deterministic response content. The full suite remains
`cargo test` with Deno on `PATH` on Unix.

Next: connect a running TypeScript app to this core through an authenticated local
capability transport. Real providers, secret storage, HTTP compatibility, streaming,
cloud integration, and packaging remain later increments.
