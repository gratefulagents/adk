# Provider adapter work — issue #5

## Delivery status

**Experimental, incomplete implementation. Issue #5 acceptance is not met.**
This crate is deliberately not re-exported by the default facade. Do not treat
its protocol coverage as a complete provider/authentication compatibility matrix.
No live provider calls or real OAuth refresh exchanges have been verified.

Implemented building blocks:

- Explicit Responses, Chat and Anthropic request/response conversions for text,
  tools, tool results, image references and structured-output schemas.
- Pull-based HTTP streams owning their response, with cancellation/deadlines,
  byte-boundary-safe SSE framing, bounded frames/aggregate data, terminal-marker
  checks and latched close-after-error. No detached producer or automatic replay
  of visible output. Retry decisions remain in the runner.
- Named first-slash routing and preservation of gateway model IDs with slashes;
  replacing a route replaces its independently constructed model/session.
- Host-owned credential store and revision-checked replacement, per-session
  refresh serialization, external-rotation reloads, account checks, sensitive
  HTTP headers and redacted credential Debug implementations.
- OpenAI/Anthropic/Copilot refresh HTTP exchange implementations (unverified
  against live services), host-supplied material codecs, bounded 401 recovery and
  late-rejection isolation, with no redirect following or transparent retry of
  potentially single-use refresh tokens.
- Provider-specific cache usage normalization, baseline price tables, retry
  header floor/precedence/capping and sanitized transport error categories.

## Explicit baseline map and remaining work

Reference factory: `repos/sdk/pkg/agentsdk/providers/factory.go`; model registry:
`repos/sdk/internal/agent/multi_provider.go`. Current Rust base includes merged
PR #19 (`28dcc3a`); this work does not depend on an unmerged runner patch.

| Baseline leg | Baseline wire/auth | Current boundary / work still required |
|---|---|---|
| OpenAI | Responses or Chat / API key | Basic explicit protocol paths implemented; full golden parity pending |
| OpenAI | Codex Responses / OAuth | Refresh exchange and bearer/account headers exist; host-supplied material parsing and encrypted continuation implemented; full Codex shaping parity pending |
| Anthropic | Messages / API key | Basic wire path implemented; model-dependent thinking, prompt-cache breakpoints, full metadata and output configuration pending |
| Anthropic | Messages / OAuth | Refresh exchange exists; material parsing and OAuth headers implemented; full Claude Code shaping/tool conventions and fallback parity pending |
| OpenRouter | Chat default or explicit Responses / API key | Generic explicit protocol can target endpoint; typed factory, host-scoped attribution and reasoning-details replay implemented; fallback `models` parity pending |
| Gemini | Chat / API key | Canonical typed factory implemented; full provider fixtures pending |
| Groq | Chat / API key | Canonical typed factory implemented; full provider fixtures pending |
| xAI | Responses default / API key | Canonical typed factory implemented; model settings and fixtures pending |
| Local | Chat / optional API key | Explicit loopback HTTP and anonymous scope supported; canonical factory/defaults implemented |
| Copilot | Metadata-selected Messages → Responses → Chat / GitHub OAuth exchanged for API token | Refresh exchange exists; material parsing implemented; model discovery/selection, token-derived endpoint restrictions pending; Copilot identity headers implemented |
| Named/multi | Above under independent prefixes, route overrides | Explicit registry and typed per-route factory implemented; baseline implicit ProviderSpec inference and delayed unavailable secondary legs pending |

Unsupported audio/files/handoffs fail explicitly. Native `RunItem::Reasoning`
and `RunItem::Compaction` retain signed/redacted/encrypted continuation through
codecs and history. Responses replay retains compaction IDs and uses empty reasoning summaries;
Anthropic signed/redacted thinking and OpenRouter reasoning details are replayed.
Cross-protocol encrypted continuation fails rather than silently losing history.
`Provider::compact` uses the dedicated Responses compaction endpoint; the optional
`runtime` feature exposes `NativeCompactor` with host-selected pricing. Local
compaction protects encrypted compaction items. Full differential parity is pending.

Other outstanding evidence/work: protocol-specific request DTOs beyond the typed
core input, full stream event/order validation, every error/retry/fallback vector,
Go-executed request/SSE/cost differential fixtures, model metadata and automatic
model selection, public facade/example, complete OAuth material/refresh race
coverage, endpoint/auth-scoped prompt-cache capability state.
Prompt-cache keys are scoped at send time to endpoint/account/auth material, with
captured-request tests; there is no learned cache capability registry yet. Runner fallback integration has not been exercised with
these HTTP adapters. Cost functions are callable by a host estimator but not
installed automatically in runner configuration.

## Ownership/security contract

The host supplies a `CredentialStore`, preserves secrets and revisions, and
shares one `Session` per actual credential scope. Multiple unrelated sessions
for one rotating refresh token are **not** a distributed refresh lock. Storage
implementations must enforce their own cross-process single-flight semantics.
Store and refresh trait errors must be sanitized by the host.

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

## Recorded local verification (2026-09-16)

- Workspace all-feature tests/doctests: **161 passed, 1 pre-existing Go-dependent
  test ignored**, including **24 provider tests** (1 unit, 16 contract, 5 loopback
  HTTP/routing, 2 HTTP cancellation/consumer-drop).
- Strict Clippy for provider sources/all targets: passed using clippy-driver as a
  Cargo workspace wrapper; strict workspace rustdoc: passed.
- Direct rustfmt and `git diff --check`: passed. Purity graph: platform-free.
- Pinned cargo-deny 0.20.2: advisories, bans, licenses and sources passed after the
  documented exact-version TLS exceptions.

These runs used installed **Rust 1.97.1**, not the repository's pinned 1.88.
The sandbox has no `/proc/self/exe`, so rustup, cargo-fmt/cargo-clippy launchers and
bundled lld fail. Direct compiler/component binaries, explicit runtime library
paths and the system bfd linker were used; no repository toolchain requirement
was changed. Pinned-1.88 CI and full issue acceptance remain unverified. The
research note is explicit about source-only versus executable evidence.

## Controlled live verification protocol

All live rows above are **UNVERIFIED**, not passed. No credentials were loaded or
external inference requests sent during this work. Complete offline protocol and
OAuth-material support before exercising the currently incomplete legs.

A host-owned test application should inject an explicitly selected scope/material,
set a short deadline and a minimal output-token budget, and call one text completion,
one stream and one tool continuation per supported provider/auth/wire leg. Separately
exercise expired-token refresh with a disposable test account and verify persisted
revision/account without logging material. Opt in separately to native compaction,
image requests and routing fallback. Record provider/model/API mode, test timestamp,
status and normalized usage only; never record authorization, refresh bodies,
account identifiers, raw prompts or unredacted server errors. Do not run this matrix
in ordinary CI or silently substitute API keys for an unavailable OAuth account.
