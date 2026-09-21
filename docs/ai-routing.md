# Local AI routing core

This increment retains the deterministic fake backend and adds an opt-in,
non-streaming OpenAI-compatible provider. The fake backend makes no network
requests and needs no credentials or accounts. A real provider is enabled only
by host configuration; the CLI and Deno host connect either backend to apps
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
after three seconds once connected. Requests are handled serially by the private capability transport. Real-provider
work has host-owned global and per-deployment semaphores, a bounded admission
queue, total deadlines, and a 1 MiB response bound. Cancellation or timeout drops
its permits. The real transport accepts HTTPS only, rejects URL userinfo and
fragments, disables redirects and ambient proxies, and redacts transport failures.
The listener is stopped and joined on normal exit, failed startup, and Ctrl+C.

## Loopback OpenAI-compatible HTTP subset

Each AI-capable launch also receives a separate loopback address and fresh
launch token in its private bootstrap (`ai.http`). It is not a management token,
provider credential, or deployment name. Clients send `Authorization: Bearer
<token>` to `POST /v1/chat/completions`. Only a single `user` message, optional
`model`, and `stream: false` (or omitted) are accepted. The response has the
non-streaming `choices[0].message.content` shape. `stream: true` returns
`text/event-stream` content-only deltas followed by `[DONE]`. All other paths, origins
(CORS is intentionally disabled), multiple messages, tools, caller
identity, provider URLs, and credential references are rejected. Requests are
limited to 64 KiB and use the same deployment-bound policy and provider bounds
as `context.ai.complete`; errors are sanitized JSON error objects.

Success is `{ selection: { provider, model }, text }`. Policy or transport
failures reject the promise. The transport does not log requests, responses,
configuration contents, or tokens. Applications can log their own data; this
mechanism does not prevent them from doing so.

The token represents the whole application process. This supervised Deno setup
is not a complete sandbox against malicious same-user processes or untrusted
apps. The capability does not contain provider secrets; real secret values are
not accepted in config, arguments, browser storage, or logs. They remain in
host-owned files outside application permissions.

Rust/unit coverage observes policy, decoder, credential-redaction, URL
validation, frame-bound, and listener-cleanup paths. Actual TypeScript/Deno
calls, stale-token renewal, cross-deployment HTTP, and external-provider
fixtures remain pending because this workspace has no Deno runtime; they are
not inferred from the local unit suite.

Provider SSE is decoded incrementally at byte boundaries (including split UTF-8)
and must terminate with `[DONE]`; malformed or truncated upstream events do not
produce a successful terminal event. Output and total duration use the existing
provider bounds; a client write failure aborts the upstream future and releases
its permits. TLS-provider integration against an external service is not
exercised in this repository; deterministic policy, decoder, credential
redaction, and URL validation tests are local. Multi-deployment saturation and
remote TLS fixture tests remain pending.
