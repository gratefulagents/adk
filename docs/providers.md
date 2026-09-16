# Provider adapter work — issue #5

## Delivery status

PR20 remains **draft and not approved**. The mapped provider/auth/route surface
below is implemented and covered by deterministic tests, but full-scope readiness
has **not received a fresh independent verdict**. Passing CI or this author's
local checks is not approval. Live provider calls and real OAuth exchanges are
**UNVERIFIED**; no credentials were loaded.

The facade exposes opt-in `providers` and `providers-runtime` features. Core
traits, typed history and pull-owned async streams remain independent of HTTP
clients and secret storage. See the compiled example in `adk_providers` and the
[client research](migration/provider-research.md).

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
`BaselineCosts` are opt-in runner adapters; unknown cost remains `None` in routing
and becomes zero only at the runner's documented numeric estimator boundary.

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

Copilot uses the pinned factory's fallback heuristic (Claude → Messages,
GPT-5/Codex → Responses, others → Chat), not an implicit `/models` network lookup.
An explicit `RouteSpec.protocol` overrides that heuristic, including the baseline
force-Chat rollback behavior without reading an environment flag. Token `proxy-ep`
hints can select only the individual/business/enterprise GitHub Copilot API hosts
and only when the configured endpoint is a canonical default. Custom endpoints
are never overridden. Cache keys include the actual selected endpoint. Material
lookup/persistence remains under the host's original logical credential scope.

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

**Independent review is blocked in this resumed runtime:** the tool surface has
no reviewer/subagent dispatch or status capability. Persona advice is not an
independent code review. Platform report `04b0b814-49a5-476f-80b0-974e79986d12`
records that limitation. A maintainer must dispatch a fresh reviewer against the
new head; no APPROVE verdict has been manufactured. If review establishes
readiness, a human may still need to change the draft PR to Ready for review.

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
