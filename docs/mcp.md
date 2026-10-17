# MCP integration

Enable `adk`'s `mcp` feature, or depend on the standalone `adk-mcp` crate.
Neither enables a server, executes a subprocess, reads a platform secret, or
deploys anything automatically. See [Rust ecosystem research](research/mcp-rust.md)
for versions, licenses and the native bounded-session decision.

## Ownership and host authority

- Load `config::ConfigSnapshot` once at run start. It pins an absolute path,
  immutable bytes, SHA-256 and parsed configuration from the **same read**.
  Reads reject symlinks/nonregular files and are capped at 1 MiB. Missing config
  is pinned as absent; creating or deleting it later is a change. Use
  `verify_unchanged()` before any requested reuse/reload. Never silently replace
  the snapshot; a new snapshot is an explicit host reconfiguration decision.
- `.mcp.json` uses the SDK `mcpServers` shape, with `type` defaulting to `stdio`.
  Repository configuration is untrusted and does not authorize process launch,
  network access, credential access, approvals or remote read-only tools.
- Transport constructors are **trusted host APIs**, not model tools. Approve the
  pinned command, arguments, working directory and endpoint *before* creating a
  transport (legacy SSE connects immediately). Stdio is a local process, **not
  a sandbox**: the host must supply a sandbox launcher when containment is needed.
  It clears the inherited environment; pass only a per-server environment.
  `ServerConfig::filtered_env` drops credential names unless granted by the host;
  repository `allowEnv` cannot widen the host credential grant. Do not forward a
  whole process environment merely because credential-name filtering exists.
- Prefer `connection::connect` to compose these boundaries: it verifies the pinned
  snapshot and host server/origin grant **before I/O**, checks credential-tenant
  equality, filters the environment and initializes the session. Low-level
  constructors remain available for host adapters and test transports.
- Hosts embedding configuration directly can construct
  `config::ConnectionConfig::inline(config)` and use `connection::connect_config`.
  Inline inputs validate the same server shapes without fabricating a file path;
  `ConnectionConfig::snapshot(snapshot)` retains the original file-change checks.
  Neither form grants host authority or reads ambient environment variables.
- Host callbacks own policy, approval, break-glass and audit. Bind durable
  decisions to the immutable request digest, never server-controlled display
  strings. Native `ToolPolicy` authorization still applies to tools returned by
  `tools::build_tools`; use the builder's composed lane or explicitly enabled host
  extra tools, not canonical built-ins.
- Remote tools require host server enablement, `trustReadOnlyHint`, the server's
  `readOnlyHint` annotation and an exact host read-only name allowlist. Repository
  `allowedTools` further narrows exposure. Invocation checks the registered
  catalog again; guessed names do not bypass discovery filtering.

## Transport matrix

| Surface | Client | Server |
| --- | --- | --- |
| stdio / newline-delimited JSON-RPC | Yes, owned child process and stderr drain | Explicitly unsupported |
| Streamable HTTP JSON and POST SSE responses | Yes | Yes, JSON responses; GET event stream not required |
| Legacy SSE endpoint event plus POST | Yes, explicitly configured `sse` | Explicitly unsupported |

The protocol baseline is MCP 2025-03-26. Stdio and legacy SSE also accept
2024-11-05 negotiation; Streamable HTTP does not. Optional sampling, elicitation,
subscriptions, tasks and resumable notification streams are not advertised.
Bounded incoming JSON-RPC batches are supported, and clients answer server pings
while awaiting the original result. Peer replies are separate messages, not
replays; initialization requests may not be batched.
Discovery supports tools, resources and prompts, with at most 100 pages and
10,000 entries by default. Duplicate/malformed cursors and oversized messages
fail rather than producing a partially trusted catalog. Default message size is
8 MiB and operation deadline is 30 seconds; host limits may narrow these.
Resource/prompt caches default to a 30-second TTL; use
`Client::set_discovery_cache_ttl` (`Duration::ZERO` disables reuse) and
`Client::invalidate_discovery` to control refresh. The manager exposes scoped
invalidation (`""` means all servers), prompt listing and prompt retrieval.
Invalidation does not replace the pinned tool catalog. Aggregate discovery skips
servers without the requested capability, while explicitly selecting an
incapable server returns an error. Qualified-name collisions use deterministic
bounded suffixes; routing and authorization retain the original server/tool names.
`ClientManager::catalog()` returns each final definition with its original server
and tool identity; `connected_servers()` reports the negotiated capabilities.
Use consuming `select_tools(allow_all, allowed_names, resources)` to narrow an
assembled manager before publishing adapters. Allowlist entries match either raw
or final names. Filtering never reassigns collision suffixes, rejects unselected
dispatch as well as hiding definitions, and gates resource access independently.
Repeated selection can only narrow access. Select servers **before connecting**;
tool selection does not undo connection side effects or grant server authority.

For an already assembled manager, `session::OwnedMcpSession` owns one retained
shutdown task. Give tools its cloneable `McpHandle`, not lifecycle authority.
Explicit `close().await` revokes handles and in-flight calls, attempts each
client's shutdown, and returns the same completion result to repeated waiters.
Cancelling a close waiter does not cancel cleanup. Dropping the owner immediately
revokes handles and requests cleanup. Close before shutting down the Tokio
executor: drop cannot promise remote acknowledgement after executor shutdown.
`OwnedMcpSession::new` starts **after** acquisition and discovery.
The protected connection functions retain cleanup when initialization fails or
is cancelled. `ClientManager::new` takes cleanup responsibility for all supplied
clients immediately, even if its returned future is never polled, and requests
rollback on discovery failure or cancellation. Such rollback requires a running
Tokio executor. Hosts connecting multiple clients still own the interval between
successful individual connections and transfer into manager construction.

For a fully owned path, use `OwnedMcpSession::connect(ConnectionSet, servers,
ToolSelection, work_dir)` or the facade builder below. Every selected host grant,
remote origin, credential tenant and required remote option is checked before
any selected connection starts. Individual connection successes stay owned during
later acquisition/discovery; errors, build deadlines and cancellation request
rollback rather than publishing a partial bundle. Limits default to 32 selected
servers, 10,000 retained discovery entries across tools/resources/prompts and a
60-second build deadline, in addition to per-client limits. Raw entries are
charged before authority filtering; scoped resource queries share the same
budget, and invalidation/reconnect/drop release their cache reservations.
Shutdown attempts each client once with its
per-client timeout; the aggregate cleanup bound is the selected-server count
times that timeout. Cleanup needs a running executor, not an unpolled caller.

## Runtime builder composition

Enable Cargo features `builder,mcp`. Set `builder::Features.mcp` and pass explicit
`builder::McpInput` through `Builder::mcp(input)`:

| Selection field | Meaning |
| --- | --- |
| `enabled` | Activates MCP only when server and tool/resource selections also exist |
| `allow_all_servers` / `allowed_servers` | Select configured, enabled servers before connecting |
| `allow_all_tools` / `allowed_tools` | Select tools by raw or final qualified name without renaming routes |
| `resource_tools` | Independently include resource listing/reading adapters |

`McpInput` is the explicit `ConnectionSet`: validated configuration, `HostPolicy`,
per-server `ServerOptions` (remote transport authority and selected environment),
and limits. Configuration may be an explicit file snapshot or inline config.
No input discovers `.mcp.json`, environment variables, origins, tenant IDs or
credentials implicitly. Supplying input alone does not enable MCP. An active
selection without input or without the `mcp` Cargo feature errors explicitly.
Disabled/empty selections have no connection effects. Unmatched server names
yield an empty selection, not fallback to all servers.

Legacy `Config.enable_mcp` enables all configured servers, tools and resource
adapters only when `Config.features` is absent. Strict features always win.
MCP tools enter a separate host-composed registration lane, so they do not enable
`ExtraTools` or unrelated built-ins. Duplicate checks, configured allowed names,
prepared dispatch policy and read-only specialist ceilings still apply.
`Bundle::mcp_catalog` and `mcp_servers` expose owned selection metadata;
the agent's prepared tool list may be narrower due to policy. Bundle close/drop
revokes retained handles and starts MCP cleanup; explicit close awaits it.

Runnable offline builder and lifecycle cases (real stdio peers) are in
`crates/adk/tests/builder_mcp/mod.rs`:

```sh
cargo test -p adk --features builder,mcp --test builder
python3 scripts/mcp-runtime-reference/run.py --check
```

The independent pinned SDK `BuildToolBundle` oracle covers twelve selection,
resource-gating and raw/final identity cases. It uses a trusted offline peer with
an explicit SDK full-access local launcher; it does **not** assert sandbox parity.
Explicit native host authority, aggregate bounds and atomic rollback are
intentional native differences, not silently claimed Go behavior. See
`fixtures/mcp/runtime-observations.json` for source/harness hashes and launch policy.

Each mutable transport owns one serialized session: there is at most one active
request per transport, stricter than the reference's eight-request concurrency
cap. Stdio drops/close kill the owned process group on Unix and drain stderr
without exposing peer text or credentials in errors. Explicit close reaps the
child. The host should explicitly close clients during orderly shutdown.
`Transport::diagnostics()` / `Client::diagnostics()` expose the bounded stderr
tail separately to the host, including after failure/close. Use
`connection::connect_with_diagnostics` to retain startup diagnostics in a typed
`ConnectionFailure`; its ordinary `Debug`/`Display` omit peer text. The default
tail is 4,096 bytes, with bounded drain grace, control sanitation and known
credential redaction (history overflow suppresses diagnostics). This is still
untrusted, potentially sensitive host-only text, never automatic model/log input.

### Remote security

HTTPS and verified TLS are mandatory by default. Host `root_certificates` may
add private CA trust without disabling certificate or hostname verification.
Private addresses and plaintext
HTTP need explicit host opt-in for that server. Resolve and validate destinations
before credential lookup, then pin the checked addresses to that request's
connection. Redirect following, ambient HTTP proxies and automatic retries are
disabled. Configuration URLs cannot contain userinfo, query strings or fragments.
A legacy server's POST endpoint must stay on the pinned origin.

`transport::HeaderProvider` and `OAuthTokenProvider` receive tenant/server scope
for every request, allowing host refresh without sharing a credential pool.
OAuth audience, required scopes and expiry are checked before transmission.
Providers must isolate tenants themselves; an arbitrary tenant string is not
authentication. Reflection detection retains a bounded connection-scoped history
across token rotation (64 distinct patterns or the host item limit, whichever is
smaller, and the host byte budget). History exhaustion fails before dispatch;
patterns are never silently evicted. Header values and raw remote errors are
never diagnostic output.
A nonempty remote tenant is required even for anonymous connections, to keep
connection/audit provenance stable.

### Reconciliation, cancellation and reconnect

There is **no automatic request replay**, including after a lost response, 5xx,
expired session or cancellation. `Error::ReconciliationRequired` names the server
and operation without retaining transport details. Once dispatch may have begun,
a dropped future poisons its transport: the next request cannot consume a stale
reply or silently retry the first operation. Establishing an explicit new
transport does not reconcile old side effects; the host must record/resolve the
uncertain operation first. A native tool cancellation reports cancellation with
an explicit reconciliation warning; do not interpret it as evidence of no effect.

## Server mode

`server::ServerMode` accepts selected tool **definitions**, a mandatory
`ServerToolPolicy`, and a mandatory `TenantResolver`. It never receives an
executable `Tool`, so it cannot accidentally call `Tool::execute` around host
policy. All tools/resources/prompts execute through tenant-aware callbacks.
Selected tool input schemas are compiled at construction and validated before
policy execution, including within batches. Local references work; an explicit
offline resolver forbids HTTP/file schema retrieval, regardless of dependency
feature unification. Tool requests include a length-delimited SHA-256 of tenant,
tool and serialized arguments for approval/audit binding. Callbacks must apply the same guardrails,
quotas, approvals and audit as native ADK execution.

The Axum router is composable with host middleware. Authenticate before resolving
a tenant, enforce verified TLS in the host listener/reverse proxy, and configure
exact allowed Host/Origin values. Do not trust client tenant or forwarded headers.
Session IDs bind to a tenant; mismatches are denied. Limits are 1,024 sessions
in total, 128 per tenant and 30-minute idle TTL, with deletion and close cleanup.
The library does not deploy a listener or provision credentials.

## Results and reusable interfaces

`tools::ToolManager` separates descriptors, tool invocation and resources from
platform integrations. Descriptions are bounded, flattened, provenance-tagged
and labeled untrusted. Schema normalization preserves valid object schemas.
A single text tool result remains text; structured/multiple content remains JSON.
Text truncates at 256 KiB on a UTF-8 boundary with an explicit marker. A result
may contain at most 128 content blocks, checked before any blob files are created;
this bounds inode/I/O amplification from tiny blobs. ADK adapters offload bounded
formatting work from the async polling thread. Image/audio
and embedded-resource blobs are limited to 10 MiB and saved under `.mcp/blobs`
with descriptor-relative no-follow/exclusive writes and private permissions on
Unix. Unsafe blob directories return a safe formatting error rather than escaping
the workspace. Safe config loading and blob persistence fail closed on platforms
without the required filesystem primitives.

The complete no-I/O tool-result preflight also runs at the dispatched client
boundary, before a completed audit or session reuse. Malformed nested content,
icons or metadata therefore produce `ReconciliationRequired`, not a later
ordinary formatting error; the failed session cannot silently dispatch again.

CRD mapping, Kubernetes secrets, platform audit persistence and tenant identity
acquisition belong in platform adapters. There are no Kubernetes dependencies in
`adk-mcp` and no production deployment in this change.

## Verification

```sh
cargo test --locked -p adk-mcp --all-targets
cargo test --locked -p adk --no-default-features --features mcp --all-targets
cargo clippy --locked -p adk-mcp --all-targets -- -D warnings
```

The crate's tests cover real mock stdio/HTTP peers, policy-gated HTTP serving,
pagination/config attacks, credential scope, cancellation/ambiguous outcomes and
explicit reconnect, plus checked-in schema/result/config fixtures under
`fixtures/mcp`. The original fixtures are source-backed; the additional
`fixtures/mcp/reference` corpus is generated by executing the complete Go SDK at
an immutable commit and compared against Rust. See [running Go reference evidence
and contract matrix](mcp-reference.md) for commands, source/toolchain hashes,
intentional validation-layer differences, and precise coverage limitations.
The separate [pinned Go↔Rust runner](../scripts/mcp-interop/README.md) executes
28 live cross-wire cases over all three baseline client transports and the
reference wrapper's Streamable HTTP server mode, including transport shutdown.
This is tested baseline interoperability, not exhaustive protocol equivalence.
