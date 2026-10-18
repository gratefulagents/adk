# Opt-in provider configuration

`adk_providers::factory::config` assembles declarative configuration into the
existing scoped `routing::Routes`. Through the facade it is available as
`adk::providers::factory::config` with the `providers` feature; integrating it
with `Builder::routes(...)` requires `builder`. It does not change production
provider defaults, the existing `RouteSpec` API, or runtime fallback policy.

## Resolve, bind, use

`Config` and `NamedRoute` support programmatic construction and strict serde
snake-case deserialization. Unknown fields are errors, not ignored options.
Neither type implements `Debug` or `Serialize`, because configuration may contain
credentials. Resolved credential seeds use the existing redacted `Secret` type.
Do not log the input document or expose `Secret::expose()` in diagnostics.

```json
{
  "provider": "anthropic",
  "routes": [
    {"prefix": "anthropic", "provider": "anthropic"},
    {"prefix": "team-bearer", "provider": "anthropic", "auth_mode": "oauth"},
    {"prefix": "gateway", "provider": "openrouter"}
  ]
}
```

1. Deserialize or construct `Config`, then call `resolve()`.
2. Inspect `ResolvedConfig.default` and `ResolvedConfig.routes`. Each route has a
   `RouteSpec` and an optional explicit `api_key` seed. A bearer token uses that
   same seed field with Anthropic `auth_mode: "oauth"`.
3. Call `ResolvedConfig::build(bind, refresh)`. The binding callback receives
   `&Scope` and `Option<&Secret>` and returns `Arc<dyn CredentialStore>` inside a
   `Result`. The host installs explicit seeds or binds its own credential source.
   Use the **entire scope** (route, endpoint, account, auth mode), not just a
   canonical provider name. Return a store even for currently missing credentials.
4. Pass the resulting `Routes` to `Builder::routes(routes)`, or call it directly
   through `Model` / `StreamingModel`.

Conceptually, given host implementations of credential storage and refresh:

```rust,ignore
let resolved = provider_config.resolve()?;
let routes = resolved.build(
    |scope, explicit_seed| host.bind_store(scope, explicit_seed),
    host.refresh_service(),
)?;
let bundle = adk::builder::Builder::new(runtime_config)
    .routes(routes)
    .build(&context)
    .await?;
```

Binding is synchronous and must not acquire credentials. Resolution and registry
construction do not call `CredentialStore::load`, `replace`, or `Refresh::refresh`.
Only a selected route's request acquires its store. There are no ambient environment
reads, implicit credential files, global stores, or parent-credential fallback.
Host stores may independently support rotation using the existing auth API; that
is not declarative OAuth file/reload support. `refresh` is host supplied; the
explicit static bearer path needs no refresh material or exchange.

## Supported resolution rules

- Empty `provider` selects OpenAI. A nonempty `routes` list promotes selection to
  a multi registry. Multi defaults use explicit `default_provider`, then the first
  slash-delimited `model` prefix, then configured single provider, then OpenAI.
  Single-provider configurations without named routes keep that provider, rather
  than interpreting `default_provider` or the configured model prefix as multi.
- Multi registers OpenAI, Anthropic, OpenRouter, Gemini, Groq, xAI, and Local even
  when credentials are absent. Missing secondary credentials are deferred to use.
- Nonblank normalized per-provider key, base URL, and API mode overrides win.
  Blank overrides fall through to the top-level value only for the configured
  single provider, or the default canonical provider when `provider` is `multi`.
  Keys and values are trimmed; provider names are ASCII case-normalized.
- Explicit per-provider Anthropic auth mode overrides the top-level mode. A
  top-level OAuth mode is scoped to the configured/default leg, not other legs.
  API-key spellings are empty, `api-key`, and `api_key`; `oauth` is supported only
  for explicitly supplied Anthropic bearer material.
- OpenAI and xAI default to Responses; OpenRouter, Gemini, Groq, and Local default
  to Chat Completions; Anthropic uses Messages. Compatible providers accept
  `responses`, `auto` (Responses), or `chat-completions`. Canonical endpoints come
  from `Kind::endpoint`; an empty Local key supplies the source's `local-key` seed.
  OpenAI API-key mode resets the exact default Codex backend URL to the public API.
- Top-level `model_fallbacks` configures only the canonical OpenRouter route.
  Chat Completions requests send the primary model followed by trimmed, nonempty,
  deduplicated fallbacks in order, omitting `models` when fewer than two distinct
  candidates remain. Responses never receives this extension. Other providers
  ignore this top-level list, and named routes (including an OpenRouter override)
  do not inherit it. Named-route `model_fallbacks` fields are rejected, matching
  the source declaration's lack of that field. Native explicit request-level
  `model_fallbacks` settings take precedence, including an empty list disabling
  configured fallbacks; this request-level override is a native extension.
- Named routes apply last, in declaration order; the final declaration replaces a
  canonical or earlier named prefix. Empty prefix defaults to its provider.
  Missing provider and nested `multi` are errors. Named routes use **only their
  own** credentials, endpoint, auth mode and API mode; omitted values use canonical
  defaults, never the parent. Shadowed routes are not bound to stores.
- Routing strips only the first prefix. `gateway/anthropic/claude-sonnet-4.6`
  reaches OpenRouter as `anthropic/claude-sonnet-4.6`. Existing `Routes` model alias
  handling, request-prefix matching, protocol and endpoint-security checks apply.

## Deliberate limits and differences from the pinned Go factory

This is a bounded native API, **not whole `ProviderSpec`, `ProviderRoute`, or
`NewProviderFromConfig` parity**. The immutable source pin is
`1dc92b73900fac74dc357a938e4b5eee6392b418` in `gratefulagents/sdk`.

- OAuth files, account paths, injected OpenAI auth sessions, automatic OAuth
  material inference/reload and Copilot configuration are unsupported. Their
  document fields are rejected, including inside named routes;
  OpenAI OAuth and Copilot provider selections are rejected. Unsupported auth/API
  mode overrides are rejected even when their canonical leg is not registered.
  Arbitrary named prefixes may still spell `copilot` if their *base provider* is
  supported; this is not a Copilot implementation.
- Go multi registers Copilot too; this subset does not. No unavailable-provider
  wrapper or eager credential-loading behavior is reproduced.
- **Go permits empty `Provider` plus named routes to spread top-level keys, URLs,
  and API modes across all canonical providers** (`factory.go:705–782`). This API
  rejects empty-provider promotion when any top-level key/URL/API/auth mode is
  nonblank. Set an explicit provider instead. This safety boundary is not claimed
  as exact Go parity.
- Go's normalized duplicate map keys can resolve nondeterministically because Go
  map iteration order is unspecified. This API rejects normalized duplicates,
  including blank entries; it does not invent a deterministic parity result.
- Unknown override provider names and unsupported API/auth mode spellings are
  errors instead of permissive Go fallbacks/ignored settings. This is not Go JSON
  field decoding compatibility; use the documented snake-case schema.
- Existing endpoint validation requires HTTPS except explicit loopback HTTP and
  rejects embedded credentials, queries, fragments, and slash-bearing route
  prefixes. Errors do not interpolate untrusted endpoint or credential values.
- General request-shaping details, cache behavior, retries, refresh, Copilot routing,
  and factory lifecycle are not established by this slice. The existing transport
  supplies behavior beyond the assertions below.

## Verification boundary

`scripts/provider-config-reference/run.py --check` extracts a disposable immutable
Go archive and reproduces `fixtures/provider-config/observations.json`, including
source/archive/harness hashes. It exercises source selection helpers and the real
factory, plus actual loopback requests for named API-key and Anthropic bearer
routes (key vs Authorization headers and OAuth beta), and OpenRouter's Chat
Completions path, opaque model and response text. The fixture excludes ephemeral
ports and includes only synthetic credentials.

`crates/adk-providers/tests/provider_config.rs` compares resolution with those
observations and performs real loopback requests, canonical replacement,
independent scope acquisition, missing-credential isolation and redaction/security
rejections. `crates/adk/tests/provider_config.rs` completes a real loopback request
through `Builder::routes(...)`, asserting zero credential reads during construction
and exactly the selected scope at request time.

```sh
python3 scripts/provider-config-reference/run.py --check
cargo test -p adk-providers --test provider_config
cargo test -p adk --no-default-features --features builder --test provider_config
```
