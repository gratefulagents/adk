# Provider adapter work — issue #5

## Delivery status

PR20 remains **draft**. The mapped provider/auth/route surface below is implemented
and covered by deterministic tests. Exact-head independent review and check
results are recorded on PR20; passing CI or the author's local checks alone is
not approval. Live provider calls and real OAuth exchanges are **UNVERIFIED**;
no credentials were loaded.

The facade exposes opt-in `providers` and `providers-runtime` features. Core
traits, typed history and pull-owned async streams remain independent of HTTP
clients and secret storage. See the compiled example in `adk_providers` and the
[client research](migration/provider-research.md).

Hosts requiring Chat Completions only can call
`adk_providers::factory::validate_chat_completions_model` before selecting a model.
It uses `supports_chat_completions` and returns an actionable `InvalidInput` error
for Codex-family names. Unlike the Go diagnostic, it does not echo caller-supplied
model text. This is explicit host validation, not automatic request-path rejection.

## Baseline capability and evidence map

Reference: SDK `1dc92b73900fac74dc357a938e4b5eee6392b418`,
`pkg/agentsdk/providers`, `internal/openai`, `internal/anthropic`.
Merged #4 runner interfaces are used directly; no unmerged runner dependency.
Test paths below are relative to `crates/adk-providers/tests`.

| Baseline provider/auth/route | Implementation | Deterministic evidence |
|---|---|---|
| OpenAI Responses and explicit Chat / API key | `factory`, `openai`, `wire`, `client` | `contracts`: Go-executed request goldens, cache accounting, structured output, continuation; `http`: captured wire/stream |
| OpenAI Codex Responses / OAuth | `auth`, `material`, `oauth`, Codex rewriting; no Chat OAuth | `auth_parity`, `material`, unit request tests; stable account cache keys in `ownership` |
| Anthropic Messages / API key and OAuth | Generation-dependent thinking, subscription identity/betas, output schema, bounded shape/effort healing and per-model learning | `contracts`, `http::anthropic_thinking_repair_is_learned_only_for_its_model`, `auth_parity` |
| OpenRouter / API key, default Chat or explicit Responses | Scoped attribution, nested model IDs, reasoning-details replay, ordered/deduplicated Chat fallback `models` | `contracts`, `routes_cost_parity` |
| Gemini, Groq / API key; xAI / API key, Responses default | Canonical endpoints/protocols through explicit compatible wire adapter | Factory combination table in `contracts`; route/default/alias tests in `routes_cost_parity` |
| Local / optional API key | Explicit loopback HTTP, anonymous material mode; default Chat | Factory/auth scope tests, loopback HTTP captures |
| Copilot / GitHub OAuth exchanged for API token, or host-supplied valid API-token material | Claude → Messages, GPT-5/Codex → Responses, otherwise Chat; explicit override, allowed token-derived endpoint hints and identity headers | `http::copilot_factory_routes_models_and_preserves_wire_identity`, factory/Copilot unit tests, `auth_parity` cooldown/expiry vectors |
| Named/multi-provider routes | Independent `RouteSpec` registrations, replacement overrides, defaults/size aliases, opaque nested IDs, lazy credential lookup | `routes_cost_parity`: complete/stream selection, isolated stores, actual fallback-binding cost |

| Cross-cutting capability | Evidence |
|---|---|
| Refresh serialization/CAS, rotation, revocation, account isolation, stale rejection, cancellation | 23 `auth_parity` vectors plus `contracts`, `ownership`, material and OAuth request unit tests |
| SSE framing, transport errors and close/drop ownership | Every UTF-8/CRLF/BOM byte boundary, bounded/truncated framing, real chunked HTTP, cancellation/consumer-drop tests |
| Reasoning, tools/results, images and PDF | Signed/redacted/encrypted/gateway replay and image tool results in `contracts`; PDF is native Anthropic only, matching baseline conversion; unsupported content fails explicitly |
| Responses phases and explicit end-turn | `http::runner_consumes_http_phase_and_false_end_turn_before_final_answer`; done-only/multipart/incomplete/error vectors in `contracts` |
| Native compaction and retained history | Dedicated compact endpoint in `http`; retained user/tool/opaque replay in `contracts`; Anthropic compaction delta replacement; origin-aware codec/runtime tests |
| Usage, prices and retry | Go-executed 81 OpenAI and 12 Anthropic cost vectors, six header vectors; `retry_parity` status/error/date/reset/cap vectors; additive vs subset cache semantics |
| Metadata/model selection | Explicit bounded scoped `/models` fetch, one OAuth rejection refresh, normalization/deduplication/picker/compaction thresholds in `routes_cost_parity` |
| Host secrets and cache affinity | Sensitive headers, redacted material/errors, HTTPS/loopback restrictions, redirects/proxies disabled; endpoint/account/auth-scoped cache tests |

Fixture provenance and differential limits are in `fixtures/providers/README.md`.
The Go-executed Chat and sparse-terminal Responses SSE fixtures are compared at
every byte split; remaining protocol/error/OAuth vectors are synthetic Rust contract tests, not falsely
labeled Go differential tests. A fresh independent review must assess whether
that evidence closes every acceptance edge case.

Transient retry scheduling and model fallback remain runner-owned. Adapter
retries are limited to OAuth rejection recovery and baseline HTTP-400 shape
healing before output is exposed. OpenAI effort healing follows bounded
max→xhigh→high and none→minimal→low ladders. Anthropic heals at most once per
request and remembers shape/effort caps per provider/model instance. Foreign
compaction retains its context summary but cannot replay an incompatible
encrypted blob. Unknown opaque reasoning fails closed. `NativeCompactor` and
`BaselineCosts` are opt-in runner adapters. Unknown prices remain `None` through
`CostEstimator::estimate_cost` into generation traces (`cost_known: false`), while
numeric accounting still charges zero. A known free price remains `Some(0.0)`.
Existing numeric-only estimators default to a known estimate and are evaluated
once per response. Retained completions from failed streams preserve the same
distinction and exact per-response amount, rather than subtracting cumulative
floating-point totals. Provider-specific fallback prices still apply: an unknown
Anthropic model can have a known default price; an unknown OpenAI model does not.
`NativeCompactor` takes the runner's `Routes` registry and resolves named routes
and size aliases before sending a provider-local model ID. Register its shared
`Arc<Provider>` with `Routes::register_kind` (or `register` for literal IDs).
Only bindings selecting that exact provider instance can compact; a fallback to
another instance is rejected before HTTP rather than crossing credential scopes.
Cost estimation still receives the original selected binding, not the wire ID.

Interactive browser login and OS credential discovery are host-owned, not part
of the pinned SDK OAuth package's material/refresh boundary. No deployment work or
additional providers were added. The pinned source contains unused endpoint-family
fallback predicates, not an active automatic protocol-switch contract. It has
scoped prompt-cache affinity, not a learned prompt-cache capability registry.
Neither unused predicates nor a hypothetical registry are counted as missing
baseline features.

## Ownership/security contract

The host supplies a `CredentialStore`, preserves secrets and revisions, and
shares one `Session` per actual credential scope. Multiple unrelated sessions
for one rotating refresh token are **not** a distributed refresh lock. Storage
implementations must enforce their own cross-process single-flight semantics.
Store and refresh trait errors must be sanitized by the host.

For OpenAI OAuth, configure `Session::with_codex_client_version("0.999.0")`
before wrapping the session in `Arc`, then share that session with the provider
and metadata resolver. Values are trimmed; blank values use the pinned
`metadata::DEFAULT_CODEX_CLIENT_VERSION` (`0.153.4`). The same immutable setting
adds `client_version` to ChatGPT metadata, inference and compaction URLs,
including retries; it is not an authentication header or credential-material
field. Refresh/rotation does not replace it, and credential/prompt-cache scope
does not change. API-key routes and unrelated hosts do not receive the query.
Configured endpoint query strings remain rejected; this option does not relax
endpoint validation or read process-global environment variables.

Copilot uses the pinned factory's fallback heuristic (Claude → Messages,
GPT-5/Codex → Responses, others → Chat), not an implicit `/models` network lookup.
An explicit `RouteSpec.protocol` overrides that heuristic, including the baseline
force-Chat rollback behavior without reading an environment flag. Token `proxy-ep`
hints can select only the individual/business/enterprise GitHub Copilot API hosts
and only when the configured endpoint is a canonical default. Custom endpoints
are never overridden. Cache keys include the actual selected endpoint. Material
lookup/persistence remains under the host's original logical credential scope.
Copilot Chat uses flat `reasoning_effort`, drops OpenRouter nested reasoning
controls/history, and retains `reasoning_text`/`reasoning_opaque` continuation.
Even buffered `complete` callers use SSE internally so Copilot tool calls are
assembled rather than lost; other Chat providers retain buffered JSON requests.

All HTTP clients disallow redirects and automatic environment proxy discovery.
Endpoints require HTTPS except explicitly configured loopback HTTP; URL userinfo,
query and fragments are rejected. Responses/errors never retain raw server error
bodies or reqwest error sources, which may include secrets/URLs. Unknown or missing
credential material never falls through to another route's token.

Cancellation drops pending HTTP/lock futures; no spawned task survives a consumer.
A cancellation after an OAuth server consumes a single-use refresh token but before
host persistence is inherently ambiguous. This implementation does not claim to
recover such a token; the host must reconcile persisted material or reauthorize.
There is no filesystem/OS credential lookup or persistence in this crate.

## Offline checks

```
cargo test --locked -p adk-providers
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
```

Tests include all SSE byte split positions (UTF-8/BOM/CRLF), truncation/limits,
refresh lead adaptation, same-session races and host rotation, cancellation,
account isolation, redacted diagnostics, cache-token subset versus additive
semantics, retry-after caps, loopback request capture, refusal to follow redirects,
real HTTP chunked transfer and clean/failed stream closure. See fixture provenance
in `fixtures/providers/README.md`. Successful Rust-authored tests do not imply full
Go parity.

## Verification environment and remaining gates

Local verification uses installed Rust 1.97.1, not pinned 1.88. The sandbox has no
`/proc/self/exe`; direct compiler/component binaries, explicit runtime library
paths and system bfd linker are required. Newer Clippy's two pre-existing
collapsible-style lint families are allowed locally; other warnings are denied.
Pinned-1.88 CI is a separate gate, not a replacement for independent review.
Exact check results and head are recorded on PR20.

Independent review must assess the exact implementation head, not infer parity
from green checks. The first full review requested corrections to Copilot Chat
shaping, buffered tool-call preservation and native-compaction route resolution;
focused loopback regressions now cover those paths. Reviewer dispatch is available
again after the earlier runtime limitation. Current-head review results are
recorded on PR20. A human may still need to change the draft PR to Ready for review;
that UI transition is separate from implementation and review readiness.

## Controlled live verification protocol

All live rows above are **UNVERIFIED**, not passed. No credentials were loaded or
external inference requests sent during this work. Complete independent offline review before exercising the live legs.

A host-owned test application should inject an explicitly selected scope/material,
set a short deadline and a minimal output-token budget, and call one text completion,
one stream and one tool continuation per supported provider/auth/wire leg. Separately
exercise expired-token refresh with a disposable test account and verify persisted
revision/account without logging material. Opt in separately to native compaction,
image requests and routing fallback. Record provider/model/API mode, test timestamp,
status and normalized usage only; never record authorization, refresh bodies,
account identifiers, raw prompts or unredacted server errors. Do not run this matrix
in ordinary CI or silently substitute API keys for an unavailable OAuth account.

### Explicit image analysis

`adk::providers::client::{Provider, ImageAnalysisRequest}` exposes an explicit
image-analysis helper, separate from the `AnalyzeImage` tool (which only attaches
loaded images to a conversation). Given an owned provider and request context:

```rust,ignore
let mut request = ImageAnalysisRequest::new(
    "gpt-5.5", image_bytes, "image/png", "What is shown?",
);
request.detail = "low"; // Constructor defaults to high.
let answer = provider.analyze_image(&context, request).await?;
```

The helper always posts to Responses, even if ordinary model calls use Chat. It
shares the existing HTTP client and credential session; it performs no discovery,
login, implicit credential lookup or detached work. Empty model uses the pinned
chat default; supply a provider-local model, not a route-prefixed name. Defaults
are 4096 output tokens, automatic truncation, no storage, in-memory prompt-cache
retention and high detail. `low`, `original` and `auto` are recognized after
case/whitespace normalization; other details use high. MIME parameters are
removed; blank or malformed MIME labels fall back to `application/octet-stream`.
Images are base64 data URLs. The helper returns trimmed, newline-joined nonblank
text or a safe error. Anthropic-protocol instances reject this OpenAI-specific
operation before dispatch.

The 11 independently executed pinned public-helper cases in
`fixtures/image-analysis/observations.json` compare exact request JSON, validation,
text and no-output behavior against local HTTP servers. Native regressions also
cover cancellation before dispatch and during an open stream, unsupported
protocols, and redacted HTTP failures. As with other native providers, transient
retry scheduling is host-owned rather than copying the Go client's retry loop;
error strings are sanitized, not byte-identical. Codex mode retains the existing
Responses rewriting/session machinery, but this helper's credentialed OAuth and
live image understanding remain **unverified**.

### Ordered diagnostic snapshots

`ModelResponse::raw` still contains native provider data. The independent optional
`snapshot_raw: Option<adk_core::JsonDocument>` carries an ordered, normalized raw
response for compatibility trace snapshots. `JsonDocument::new` validates JSON
without rewriting member order, number spellings or duplicate keys. Its native
serde representation is a string, so a checkpoint round-trip through
`serde_json::Value` does not silently reorder it. The codec embeds its content as
JSON when producing a compatibility snapshot; custom models may leave it `None`
and retain the existing native-raw fallback (including explicit JSON null).
Neither field authorizes disclosure: full trace capture is still opt-in.

Providers additionally set `snapshot_projection: Option<SnapshotProjection>` to
explicitly identify the pinned OpenAI or Anthropic normalized-message format.
When tagged, the compatibility codec derives items, usage and end-turn from that
ordered document, not from native execution fields. This preserves the SDK's
omitted end-turn, canonical compaction content and raw tool-argument spelling
without discarding native information. Missing or malformed tagged documents
produce a bridge error. Untagged custom documents retain the previous fallback;
the codec never guesses a format from JSON shape. Both the tag and ordered JSON
survive checkpoints, including `serde_json::Value` serialization.

The pinned Go fixture executes real public providers against local HTTP servers.
Two Chat completion and seven public streaming raw documents now have exact-byte
Rust checks, including raw tool-argument order/numbers, SDK omission/default rules,
SSE-only Type/stop-reason behavior, and Anthropic block-start versus delta content.
The native response remains separate: for example, the SDK's streamed Responses
raw document omits unclosed compaction blocks, without deleting native compaction
items or the original provider payload. An id-less Anthropic start still counts
as a start for duplicate/ordering validation; Responses tool-argument events may
identify an existing item by their explicit output index.

This is **not complete provider snapshot parity**. The fixture also records
`GetResponse` separately from `StreamResponse`: both may use SSE internally but
produce different documents. In particular, complete-method Anthropic SDK
accumulation canonicalizes tool arguments and has a distinct compaction-content
representation; standard Responses completion drains fields differently. All eight recorded complete-method profiles now have exact-byte checks through
Rust's actual HTTP provider, in addition to the public streaming checks. Standard
Anthropic/Responses complete calls consume SSE incrementally, while Copilot keeps
its protocol-specific transport choice. Complete Anthropic arguments preserve Go's
Float64 canonicalization (including negative zero), and its compatibility-only
compaction union encoding does not replace the native summary. The eight complete
and seven public-stream cases also compare full response snapshots, including
items, aggregates, usage and end-turn, through serialization and completed-run
checkpoint recovery. Unrepresented provider cases remain unverified; these
fixtures do not close entire provider capabilities. The fixture's
`streaming` flag describes transport; `method` identifies the public SDK call.
