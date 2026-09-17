# Local AI routing core

This increment adds `paraco::ai`, an in-memory Rust policy core with a deterministic
fake backend. It makes no network requests and needs no credentials or accounts.
The CLI and Deno host connect this core to apps requesting `ai` through
`context.ai.complete({ prompt, provider?, model? })`.

The trusted host constructs `Config` and supplies the caller identity separately
from `Request`. An app request contains only optional provider/model selection
and a prompt. The core does not authenticate callers. The local transport authenticates a
host-issued token and supplies the launched app identity, never an app claim.

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

## Local capability transport

`paraco run ./examples/ai --ai-config ./examples/ai-config.json` starts a fake AI
example. Configuration is read only when explicitly passed; it must be outside
the canonical app directory so ordinary app read permissions do not include it.
The example configuration illustrates credential identities (not secret values),
routes, app grants, and optional app/runtime/provider defaults. Unknown fields
and invalid references fail before Deno starts. An app cannot declare grants in
its manifest. Without configuration the interface exists but calls are denied.

The Rust host opens an ephemeral IPv4 loopback TCP listener per AI-enabled run.
A cryptographically random 256-bit token binds requests to that launched app.
The host delivers address and token over stdin, consumed before app import;
neither is supplied in command arguments or inherited environment variables.
The adapter grants network access only to the app listener and capability port.
Apps without `ai` receive no AI interface or capability listener.

This is a private transport, not the planned OpenAI-compatible HTTP endpoint.
Each connection carries one JSON request and response, each prefixed by a
four-byte big-endian length. Frames are limited to 64 KiB. Unknown request
fields, including caller identity and grants, are rejected. The host uses a
one-second total read deadline and bounded writes; the adapter closes calls
after three seconds once connected. Requests are handled serially for this fake
backend. Concurrency and real-provider timeouts belong to a later increment.
The listener is stopped and joined on normal exit, failed startup, and Ctrl+C.

Success is `{ selection: { provider, model }, text }`. Policy or transport
failures reject the promise. The transport does not log requests, responses,
configuration contents, or tokens. Applications can log their own data; this
mechanism does not prevent them from doing so.

The token represents the whole application process. This supervised Deno setup
is not a complete sandbox against malicious same-user processes or untrusted
apps. The capability does not contain provider secrets, and host configuration
files contain only fake credential identities in this milestone.

Verification includes actual TypeScript calls, absent and cross-app grants,
absent capability, invalid authentication, forged identity, token renewal,
frame bounds, and listener cleanup alongside the existing policy/runtime suite.

Real providers, secret storage, HTTP compatibility, streaming, cloud integration,
and packaging remain later increments.
