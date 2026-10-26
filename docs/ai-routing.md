# Local AI routing core

This increment retains the deterministic fake backend and adds an opt-in
OpenAI-compatible provider with completion and streaming support. The fake backend makes no network
requests and needs no credentials or accounts. A real provider is enabled only
by host configuration; the Rust host connects either backend to apps
requesting `ai` through `context.ai.complete({ prompt, provider?, model? })`.

The trusted host constructs `Config` and supplies the caller identity separately
from `Request`. An app request contains only optional provider/model selection
and a prompt. The core does not authenticate callers. The local transport authenticates a
host-issued token and supplies the launched app identity, never an app claim.

Configuration contains credential identities and their owning providers, allowed
routes, per-app capability requests and credential grants, app/runtime defaults,
and provider default models. It contains no real secret values. Real-provider
configuration additionally maps a credential identity to an absolute, host-owned
secret file and a provider to an HTTPS OpenAI-compatible completion endpoint.
Secret files must be outside the app directory and are read only after capability
and grant authorization succeeds. Unknown credential references and routes
referencing another provider's credential are rejected at construction.
Configuration is immutable once the proxy is constructed.

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
missing. Deployment-specific defaults are configured with host-owned deployment IDs.

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
`cargo test` with workerd and esbuild on `PATH` on Unix.

## Local capability transport

Before granting a standalone app, ask the host for its durable deployment ID:

```sh
paraco identity ./examples/ai
```

Use that exact output as the key in `apps` in the host-owned AI configuration.
Replace `REPLACE_WITH_PARACO_IDENTITY_OUTPUT` in `examples/ai-config.json`, then
run `paraco run ./examples/ai --ai-config /path/to/ai-config.json`.

The identity is stable for the same canonical source. Configuration is read only
when explicitly passed and must live outside the app package. Unknown fields and
invalid references fail before workerd starts. An app cannot declare grants in
its manifest. Without configuration, AI calls are denied.

For an AI-enabled launch, the workerd adapter exposes the scoped capability over
its private HTTP binding. Requests cross a private Unix socket to a Rust bridge,
which authenticates the launch and translates them to the existing host AI
transport. The host applies deployment grants, credential routing, deadlines,
and admission limits. Apps without `ai` receive no binding. Provider secrets
remain in host-owned files and are never passed to an app.

Capability requests use the deployment identity selected by the host; a request
cannot claim another identity. Configuration and launch tokens do not appear in
command arguments, application environment variables, or app-visible logs. The
AI transport does not log prompts, responses, configuration contents, or tokens.
Applications can still log data they choose to print.

The application process is not isolated from malicious programs running as the
same OS user. Linux bubblewrap containment is enabled; secure macOS containment is
not implemented and normal execution fails closed there. Trusted development may
explicitly set `PARACO_ALLOW_UNSANDBOXED=1`. See [workerd migration limits](workerd-migration.md).

The real transport accepts HTTPS only, rejects URL userinfo and fragments,
disables redirects and ambient proxies, and redacts transport failures. Provider
SSE is decoded incrementally and must terminate with `[DONE]`. External provider
compatibility, remote TLS fixtures, and production workload measurements remain
unverified.

## Verification

Run policy tests with:

```sh
cargo test --offline --test ai
```

The local HTTPS fixture exercises real Reqwest completion and SSE through the
capability transport. Workerd integration tests cover standalone and hosted
grants and lifecycle renewal. See [current status](status.md) for observed
results.
