# Local AI providers and routing


## Dashboard settings

Open the host dashboard and select **Configure AI** under **AI for your apps**.
Choose **All apps (global default)**, select **Choose provider**, and save a
provider, model ID, and connection. The global default grants that connection to
installed apps declaring the `ai` capability. AI starts disabled. Installing an
app without that capability never gives it AI access.

Select an installed app in **Apply to** to choose **Use global default**,
**Disabled**, or its own provider/model/connection/key. Overrides are keyed by the
host's installation identity, not the app-supplied name. Disabling an app does not
fall back to the global default. An app can only explicitly select the model and
provider configured for it; it cannot select another app's key or SDK.

Settings live under `<data-dir>/ai/settings.json` (normally `~/.paraco/ai/`).
Keys are separate owner-only files in that directory; the dashboard never reads
key values back. Leave the API-key field blank to keep a saved key. Switching
provider or endpoint requires a new key. Settings writes are atomic and serialized
across host processes. Replaced keys are removed. Keep this directory outside all
app packages. New requests pick up changes immediately; already-running requests
may finish using their previous settings. Standalone apps and cron also use these
settings when launched with the same data directory and without `--ai-config`.
An empty host configuration (`{"apps":[]}`) can serve the settings dashboard.

Existing `aiConfig` / `--ai-config` files take precedence over dashboard settings.
Those apps are marked as managed by a configuration file in the selector. Remove
that explicit setting and restart the app/host to use the dashboard. The explicit
file route/grant behavior below remains supported.

| Provider | API connection | Local coding SDK |
| --- | --- | --- |
| OpenAI / Codex | OpenAI Chat Completions with an API key | `@openai/codex-sdk`, signed in with ChatGPT |
| Claude | Anthropic Messages with an API key | `@anthropic-ai/claude-agent-sdk`, local Claude sign-in |
| Grok (xAI) | xAI Chat Completions with an API key | Not supplied |
| Other | Custom HTTPS OpenAI-compatible completion endpoint | Not supplied |

Model IDs are entered explicitly, since availability depends on the provider and
account. There is no silent switch from SDK/subscription mode to paid API calls.
API calls use the API account's billing. Subscription access and limits remain
provider-controlled: see [Codex authentication](https://learn.chatgpt.com/docs/auth)
and [Claude Agent SDK plan access](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan).

## Codex CLI setup

Choose **Codex CLI (use local sign-in)** in the dashboard and save. Paraco finds
`codex` on the host's PATH (also checking `~/.local/bin` and `~/.npm-global/bin`)
and checks its sign-in status. No Node.js path or separate JavaScript SDK is
required. Leave Model blank to use the Codex default, or enter an override.
If Codex is missing or signed out, the dashboard explains how to install or sign
in before saving. Installations must support `codex exec --ephemeral`.

Calls use the existing host sign-in, an empty temporary working directory,
read-only sandboxing, disabled tools, bounded output and a 120-second deadline.
They do not silently fall back to API-key billing. A saved connection confirms
local sign-in; provider eligibility and limits are checked when making a request.

## Optional SDK setup

Under **Advanced connection settings**, SDK mode requires Node.js and the relevant official JavaScript SDK installed on
the host, outside app packages. These optional dependencies are not included in
the Paraco release bundle. Install the package in a dedicated host directory,
sign in with its coding client, and enter the absolute Node executable and SDK
entry-module paths in the dashboard. For example, from that directory:

```sh
npm install @openai/codex-sdk@0.165.0
node -p 'process.execPath'
node --input-type=module -e 'import {fileURLToPath} from "node:url"; console.log(fileURLToPath(import.meta.resolve("@openai/codex-sdk")))'
codex login
```

For Claude, install `@anthropic-ai/claude-agent-sdk@0.3.312`, resolve that package's entry
path instead, and sign in through Claude Code. These are the package versions whose interfaces were checked for this adapter.
Use an SDK version supporting the options in `runtime/ai-sdk.mjs`; upgrade the SDK if its configuration options are
rejected. The current [Codex SDK](https://learn.chatgpt.com/docs/codex-sdk) and
[Claude SDK configuration](https://code.claude.com/docs/en/agent-sdk/configuration)
document the host-side integration.

Each call starts a fresh SDK session in an empty temporary directory and returns
only final text through the existing `context.ai.complete` contract. Claude tools
and filesystem settings are disabled. Codex uses read-only execution, disables
shell/image/connector tools, clears MCP/plugin overrides, and rejects interactive
approvals. SDK sign-in stores remain host-owned; ambient API keys and Node
injection variables are not forwarded. This is a text-completion adapter, not a
general-purpose coding workspace or app-controlled agent session.

SDK calls have a 120-second deadline, bounded output, and process-group cleanup
on timeout/cancellation. API calls retain the existing 30-second deadline. Hosted
AI apps have a 130-second gateway timeout. Anthropic and SDK responses normalize
to one final text delta on the compatibility streaming endpoint; OpenAI-compatible
APIs retain incremental SSE streaming. Provider failures are redacted.

Automated verification uses local TLS servers and mock SDK modules, including a
real dashboard-to-worker-to-SDK round trip. Live provider billing, subscription
sign-in, and compatibility with installed SDK versions require account-specific
verification; tests do not make paid provider requests.

## Explicit routing files

Explicit routing files retain the deterministic fake backend and support opt-in
real providers with completion and normalized streaming support. The fake backend makes no network
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
its manifest. Without explicit configuration or an enabled dashboard setting, AI calls are denied.

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
disables redirects and ambient proxies, and redacts transport failures. OpenAI-compatible provider
SSE is decoded incrementally and must terminate with `[DONE]`. External provider
compatibility, remote TLS fixtures, and production workload measurements remain
unverified.

## Verification

Run policy and adapter tests with:

```sh
cargo test
node --test tests/ai-sdk.test.cjs
npm run test:ai-browser
```

The local HTTPS fixture exercises real Reqwest completion and SSE through the
capability transport. Workerd integration tests cover standalone and hosted
grants and lifecycle renewal. See [current status](status.md) for observed
results.
