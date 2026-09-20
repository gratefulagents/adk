# Pinned, running Go MCP reference verification

## Scope and provenance

Reference repository: `https://github.com/gratefulagents/sdk`, immutable commit
**`1dc92b73900fac74dc357a938e4b5eee6392b418`**. The module is
`github.com/gratefulagents/sdk`; its actual `go.mod` requires Go 1.26.2 and
`github.com/modelcontextprotocol/go-sdk v1.4.1`. Evidence was generated with
**Go 1.26.8 linux/amd64**, not a substitute implementation or a module with its
Go version edited down.

- [`inputs.json`](../fixtures/mcp/reference/inputs.json): shared JSON inputs and
  compact large-input recipes. No Go expected outputs are supplied to the harness.
- [`observations.json`](../fixtures/mcp/reference/observations.json): actual Go
  schema/result/config observations, immutable commit, toolchain, command, SHA-256
  of every MCP and web-security package Go source/test file, `go.mod`, `go.sum`,
  runner, injected test, and inputs. The git commit also pins the remaining SDK
  sources; `go.sum` pins downloaded dependencies. Hashes describe archived source,
  not possibly edited working-tree files.
- [`baseline-tests.json`](../fixtures/mcp/reference/baseline-tests.json): actual
  original Go package test/subtest outcomes, command, exit code, toolchain, commit,
  and observations hash. Timings and temporary paths are excluded. The helper
  process entrypoint is skipped in the parent test process by design, and is
  exercised by the passing stdio child-lifecycle test.
- [`reference.rs`](../crates/adk-mcp/tests/reference.rs): Rust comparisons against
  those observations and provenance freshness checks. Existing `fixtures/mcp/*.json`
  remain source-derived fixtures; this separate corpus is independently executed.

The runner archives the exact commit into a temporary scratch directory, copies
`scripts/mcp-reference/reference_test.go` into that package, and runs `go test`
against the **complete actual SDK package and its pinned dependencies**. This
permits calling unexported `normalizeInputSchema`, `validateRemoteEndpoint`, and
connection guards without modifying `repos/sdk`. There is no extracted or
reimplemented Go normalization/policy logic. Required harness source stays under
`scripts/mcp-reference`; isolated source and generated blobs are removed on exit.
Build/module caches and raw baseline logs remain disposable under scratch.

## Reproduce

From the repository root (Python 3.12+ for tar extraction filters, Git, Go 1.26.8,
and Rust 1.88; Linux/Unix filesystem primitives):

```sh
# Generate observations and original Go package test evidence.
python3 scripts/mcp-reference/run.py --baseline-tests
# Re-run both; fail rather than overwrite if evidence/provenance changed.
python3 scripts/mcp-reference/run.py --check --baseline-tests

# Toolchain/target already available in this workspace:
export PATH="$PWD/../scratch/rust-1.88/bin:$PATH"
export LD_LIBRARY_PATH="$PWD/../scratch/rust-1.88/lib"
export CARGO_TARGET_DIR="$PWD/../scratch/adk-target-188"
cargo test --locked -p adk-mcp --test reference
cargo test --locked -p adk-mcp --all-targets
cargo clippy --locked -p adk-mcp --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/adk-mcp/tests/reference.rs
```

`--sdk` and `--scratch` override paths. `GOROOT` overrides `/usr/local/go`.
`GOTOOLCHAIN=local`, `GOWORK=off`, `GOFLAGS=-mod=readonly`, and scratch-local
`GOCACHE`, `GOMODCACHE`, `GOPATH`, `GOTMPDIR` prevent toolchain substitution,
workspace replacement, lockfile editing, and source-tree build artifacts.
First execution needs dependency download access; subsequent checks reuse caches.
No command contacts a production MCP service: endpoint corpus cases use literal
IPs and validation only, host-denial checks stop before transport creation, and
original remote tests use local test servers. No configured corpus command is
launched. The Go executable may print a telemetry `/proc/self/exe` warning in
this container even with telemetry disabled; the recorded tests still exit zero.

## Corpus contracts and exact comparison boundary

**25 schemas:** nil, scalar/boolean/array junk, empty objects, implicit/empty/object
and non-object types, non-string type, explicit null properties, constraints,
metadata and `$ref`/`$defs`. Actual SDK normalization is compared structurally,
not against expectations derived from Rust. This is normalization, not complete
JSON Schema validation. Go-only non-JSON values (marshal failures, typed nils)
are outside this wire-JSON corpus.

**165 results:** nil/empty, single and multiple text, `isError`, structured output,
image/audio, embedded text/blob resources, resource links, resource reads,
text-plus-blob, UTF-8 truncation, oversized blobs and symlink escape attempts.
The shared recipes generate 300,000-byte UTF-8 text and 10 MiB + 1 byte blobs.
For saved blobs, both implementations independently read back and compare byte
count, SHA-256, private `0600` permissions and workspace confinement. Symlink
cases assert the outside directory remains empty. Truncated text is compared by
byte count and SHA-256. JSON key ordering and ephemeral blob paths are normalized;
Go's timestamp/MIME-database-derived names and Rust's UUID/extension choices are
not claimed equal. Human-readable notes retain size/MIME and normalized path.
Go error diagnostics are retained in observations; Rust's intentionally redacted
blob errors are compared by error placement/count, not error wording.

The expanded corpus exercises missing/null/empty fields, malformed types,
base64 whitespace/padding, zero-byte media and empty resource fields. Both
implementations reject malformed protocol content before filesystem writes;
Rust now normalizes optional fields to the actual Go decoder/formatter behavior.
Nested tool-result content, icons, annotations and metadata are fully preflighted
even when their fields are discarded by the selected content kind. The final
review added 62 actual-Go cases: 57 rejections with empty workspaces and five
accepted nested-normalization cases. A shared 128-block budget and 32-level depth
bound prevent nested content from bypassing pre-I/O bounds.
This same no-I/O preflight runs before the client's terminal audit/session
restoration. Nested malformations are audited `OutcomeUnknown`, leave the client
failed, and cannot be reclassified as ordinary formatting errors or redispatched
on that session; the regression asserts exactly one tool dispatch.
`isError` in this corpus records the Go decoded result flag;
it is not a new end-to-end assertion of native tool adapter execution. Existing
Go `TestDynamicToolExecutesAndReturnsSingleTextContent` and Rust
`tool_adapter.rs` test the adapters separately. This corpus does not claim parity
for unknown future content kinds, all MIME types,
concurrent filesystem races, or every platform. Existing exclusivity/symlink tests
supply additional independent evidence; Rust's 128-block pre-I/O cap is stricter
than the Go renderer, not a shared limit.

**34 actual Go server requests:** selected tool/resource/prompt listing and tool
invocation, plus resource-read rejection cases, including absent/null/empty
params, cursors and arguments. Each runs in an actually initialized session.
Rust compares normalized response bodies and captures policy arguments, not just
success codes. Fixed differences include omitted empty descriptions/false flags/
prompt arguments, accepted empty resource names, and explicit null tool arguments
preserved at the policy boundary while validated as an empty object. These are
actual `NewServerMode` HTTP exchanges, not inferred wire fixtures.
Ordinary tool callback errors now produce the same sanitized MCP `isError`
result, while explicit reconciliation-required callbacks retain HTTP 502 rather
than becoming definitive failures. Resource-not-found and malformed URI fields
also match the actual reference error codes (-32002 and 0, respectively).

**21 configs:** valid stdio, userinfo/query/fragment/file URLs, public HTTPS,
plaintext HTTP, IPv4/IPv6 loopback, metadata IP, unsupported type, missing command,
invalid environment names/values, wrong field type, repository credential opt-in,
empty server name, symlink and >1 MiB config files, and mixed transport fields.
Each observation distinguishes:

1. **Go `LoadConfig`**: JSON parsing only; accepts all these cases except the wrong
   `allowedTools` field type. It follows symlinks and has no byte cap. Acceptance
   is not authority and not evidence that a server would connect.
2. **Actual Go remote policy**: `connectRemoteServer` without host enablement
   denies every remote case before I/O. `validateRemoteEndpoint` runs separately
   with/without private-network opt-in; it rejects userinfo/query/fragment, requires
   HTTPS by default, and invokes actual web-security URL/IP validation. Even a
   public HTTPS URL passing validation is not a successful connection.
3. **Actual Go dispatch**: unsupported transport, missing command and mixed
   transport fields are checked through `connectConfiguredServer`, without
   spawning anything. Invalid environment entries are intentionally only loaded,
   not executed; no Go process-launch rejection is claimed for them.
4. **Go environment filter**: `FilterCredentialEnv` honors repository `allowEnv`.
   Rust tests independently require an intersecting host grant before allowing
   the synthetic credential through.
5. **Rust**: `ConfigSnapshot` rejects syntactic/path/size hazards earlier, while
   network authorization remains at host/transport boundaries. The same URLs are
   checked by `HttpTransport::connect` in non-dispatching Streamable mode with both
   private-policy settings and compared to Go's endpoint observations. This does
   not test legacy SSE network exchange or a DNS-rebinding race.

**Gap discovered and repaired:** the initial Rust implementation ignored inactive
transport fields. The independently executed Go dispatch rejected both mixed-field
cases. Rust now rejects them during snapshot validation (including remote args,
env and allowEnv), and normalizes transport case/whitespace like Go dispatch. The
shared corpus compares both actual dispatch rejection and the earlier Rust gate.

## Exhaustive baseline transport/server-mode contract map

This maps the public manager/transport/server-mode surface in the pinned MCP
package, including dependencies delegated to the protocol SDK. It is a **contract
inventory, not a claim that every wire combination has a differential test**.

Evidence labels: **D** = running shared corpus and Rust comparison;
**G** = original Go test executed (exact outcomes in `baseline-tests.json`);
**R** = Rust test suite; **S** = inspected pinned source only for the named claim.
Go names below omit the `Test` prefix; Rust names refer to test functions/files
under `crates/adk-mcp/tests`. Source names refer to the pinned MCP package unless
otherwise noted. Separate G and R evidence does not establish Go↔Rust wire
interoperability by themselves. **I** below refers to the additional running
[cross-wire harness](../scripts/mcp-interop/README.md) and its
[28 observed cases](../fixtures/mcp/interop/results.json).

### Configuration, manager, discovery and adapters

| Contract | Pinned Go evidence | Rust evidence / precise delta or limitation |
| --- | --- | --- |
| Missing config, `mcpServers` fields, stdio/enabled defaults | D; G `LoadConfig_FileMissing`, `LoadConfig_Parse`; `config.go` | D; R `config.rs`; stricter load validation described above; both trim/case-normalize supported transport names |
| Snapshot path/content pinning, changed-file rejection | G `LoadConfigSnapshot_PinsPathAndContent`, `VerifyUnchanged_DetectsModification`, `LoadConfigSnapshot_FlagsAgentWritablePath`; `snapshot.go` | R `config.rs`; Unix descriptor-relative no-follow, byte cap and immutable parsed snapshot are stricter; no non-Unix safe-config support |
| Host process/network authority, enabled servers, sandbox options | G `ConnectStdioServerNetworkAccessAllowlist`, `ResolveManagerOptionsTrustsManagerWorkDir`; S `NewManagerFromConfig` | R `end_to_end::composition_rejects_host_denial_before_any_subprocess`; native host grants and approved sandbox launcher replace Go executor/options types; neither repository config nor transport alone grants containment |
| Per-server environment credential filtering | D; G `FilterCredentialEnv_*`; `env_filter.go` | D; R `config.rs`; repository cannot widen host grants; host explicitly supplies selected inherited environment. No automatic environment interpolation is implemented in the reference MCP package either |
| Initialize once, capabilities, protocol versions | G remote/stdio lifecycle tests exercise actual protocol SDK; S `connect*Server` | R `client::initializes_once_negotiates_and_gates_capabilities`, `legacy_protocol_is_only_negotiated_for_stdio_and_sse`; baseline 2025-03-26, legacy 2024-11-05 only stdio/SSE; not all protocol SDK versions/features |
| Tools/resources/prompts discovery and 100-page/10,000-item caps, repeated cursors | S `listAllTools`, `listAllResources`, `listAllPrompts`; G server-mode lists basic resources/prompts | R `client.rs` pagination/atomic-validation tests; repeated/malformed/duplicate entries fail atomically; Go pagination attack corpus not executed differentially |
| Resource/prompt cache TTL and explicit invalidation | G `RemoteStreamableHTTPUnauthenticated`; S `cachedResources`, `cachedPrompts`, `WithDiscoveryCacheTTL`, `InvalidateDiscovery` | R client TTL/disabled-cache/scoped-all invalidation tests: 30-second default, zero disables reuse, expiry measured from discovery start; tools stay pinned. Manager forwards prompt operations and aggregate discovery skips incapable servers |
| Qualified tool names and collision routing | G `NormalizeNameForMCP`, `BuildToolName_LengthAndPrefix`, `EnsureUniqueToolName`; `names.go` | R client/name tests: Go-compatible suffix/truncation/hash allocation, sorted server order and discovery-page order after policy filtering; exact original-name routing. Deliberate final-hash collision fails closed instead of overwriting a route |
| Exact repository allowlist; read-only hint requires opt-in; remote host read-only allowlist | G `TrustedMCPReadOnlyRequiresServerOptIn`, `RemoteHintAloneDoesNotExposeTool`, `RemoteStreamableHTTPReadOnlyAuthenticated`; `manager.go`, `remote.go` | R `client::remote_requires_all_four_readonly_conditions_and_breakglass_cannot_widen`, exact discovered-name invocation; hints alone never authority |
| Call tool, list/read resource, list/get prompt surfaces | G remote tests and `ServerModeUsesPolicyBoundaryAndImmutableRequest`; `manager.go` | R `client.rs`, `end_to_end.rs`; I exercises each operation across all three baseline client transports and the reference's supported server mode, with exact host resource/prompt allowlists |
| Break-glass catalog/request, permission/approval, immutable audit context | G `BreakGlass*`, `RequestBreakGlassTool*`, `BlockedMessages`; `breakglass*.go` | R client policy/break-glass/digest tests; host callbacks rather than a Go question/catalog or model-visible request-break-glass tool clone |
| Discovery/tool adapters, untrusted descriptions, errors | G `BuildToolsIncludesResourcesWhenAvailable`, `DynamicTool*`, `Sanitize*`; `tools.go`, `sanitize.go` | R `tool_adapter.rs`, `tools.rs`; native ADK types, cancellation/deadline handling, redacted operational errors; not byte-identical Go diagnostics |
| Schema/result rendering and filesystem effects | D; G `NormalizeInputSchema*`, `FormatCallToolResult*`, `CreateExclusiveBeneath*` | D; R `tools.rs`; normalization boundaries and untested optional-field cases stated above |

### Client transports and security

| Contract | Pinned Go evidence | Rust evidence / precise delta or limitation |
| --- | --- | --- |
| stdio newline JSON-RPC, initialization, notifications, correlation | G `ConnectStdioServerChildOutlivesConnectReturn`; protocol SDK transport | R `transports::stdio_scoped_env_notifications_stderr_and_reaped_shutdown`; I Rust client → pinned go-sdk stdio peer, discovery/invocation/reaped shutdown |
| Peer-initiated ping, unsupported requests, JSON-RPC batch correlation | S protocol SDK delegation in Go transports (no targeted shared corpus) | R `stdio_peer_ping_and_batches_preserve_ids_and_original_call`, `http_peer_ping_and_batches_use_separate_authenticated_posts`; bounded replies preserve IDs; unsupported peer requests get method-not-found, no sampling/elicitation callbacks |
| Child lifetime independent of handshake deadline; process group termination, close | G `ConnectStdioServerChildOutlivesConnectReturn`, `TerminateProcess_*`; `lifecycle*.go` | R stdio cancellation/reap tests; Unix group kill, explicit close reaps, drop kills; Windows process-group guarantees not established |
| stderr bounded and drained | G `StderrTailKeepsBoundedTail`, `ErrWithStderr`, `ConnectStdioServerReportsChildStderr` | R `diagnostics.rs` and tail unit tests reproduce trailing `defghXYZ` and startup `boom-traceback`; same 4,096-byte default. Host getters and `connect_with_diagnostics` retain tail after failure/close, with bounded drain, credential redaction and control sanitation; ordinary errors retain typed uncertainty and omit potentially sensitive peer text |
| Streamable HTTP request/session headers, JSON and POST SSE replies | G `RemoteStreamableHTTPUnauthenticated`, authenticated/OAuth tests; protocol SDK | R `http_json_and_sse_replies`, `http_session_initialize_notification_and_delete`; no standalone GET event stream or resumption in Rust; Go wrapper disables standalone SSE too |
| Legacy SSE GET endpoint event, correlated POST, same-origin endpoint | G `RemoteLegacySSECompatibility`, `RemoteTransportPinsOriginBeforeCredentials`; `remote.go` | R `legacy_sse_endpoint_query_posts_and_correlates`, `legacy_sse_cross_origin_endpoint_rejected`; I Rust client → pinned go-sdk SSE peer; explicitly configured SSE, no silent protocol fallback |
| HTTPS default, private-network opt-in, no userinfo/query/fragment | D; G `RemotePolicyFailsClosed`; actual `tools/web` validation | D + R `http_ssrf_url_policy_and_redirects`; literal-IP policy parity only, not every reserved range/DNS environment |
| DNS/address validation and origin pin before credentials, redirects/proxies disabled | G `RemoteTransportPinsOriginBeforeCredentials`, `RemoteRedirectRejectedWithoutCredentialLeak`; S `newRemoteHTTPClient` and web safe client | R `http_ssrf_url_policy_and_redirects`; per-request checked address pinning; no adversarial DNS-rebinding race or ambient proxy runtime test in shared corpus |
| TLS certificate/hostname verification and custom CA roots | G `RemoteTLSVerificationAndCustomRoots` | R `tls::explicit_ca_trust_preserves_certificate_and_hostname_verification`; no insecure TLS mode; no public PKI/real deployment smoke test |
| Anonymous/header/OAuth auth with tenant/server-scoped per-request providers | G `RemoteStreamableHTTPUnauthenticated`, `RemoteStreamableHTTPReadOnlyAuthenticated`, `RemoteStreamableHTTPOAuth`, `AuthenticatedRemoteRequiresTenantBeforeNetwork` | R OAuth tests and end-to-end; Rust requires nonempty tenant even for anonymous requests, stricter than Go anonymous mode; host provides identity/credential isolation |
| OAuth audience/scopes/expiry, reserved/idempotency headers and header bounds | G `OAuthTokenValidation`, `RemoteTransportDisablesHTTPReplayAndIdempotencyHeaders`; S header checks | R `oauth_claims_checked_for_every_request_and_tenant_server`, `reserved_headers_fail_closed`; host refresh callback, not browser OAuth/login/storage implementation |
| Credential reflection blocked/redacted including escaped forms and reconnect | G `CredentialReflectionDetection`, `ReconnectedResourceUsesFreshCredentialReflectionState` | R `oauth_credential_reflection_raw_and_json_escaped_rejected`; connection-scoped bounded credential history across rotation; R rotation/history-overflow regressions; no exhaustive encoding/exfiltration proof |
| Remote concurrency/backpressure and response/message/SSE bounds | G `RemoteBackpressureIsBoundedAndCancellable`, `RemoteResponseSizeIsBounded`; Go default 8 concurrent requests | R HTTP/SSE/stdio bounds tests; Rust serializes one request per session, stricter; no configurable 8-way pool; Rust 30 s/default 8 MiB vs Go 15 s connect timeout and separate remote bounds |
| Remote 5xx, disconnect, cancellation: no replay, outcome unknown | G `RemoteHTTP5xxIsOutcomeUnknownAndNeverReplayed`, `RemoteCancellationIsOutcomeUnknownAndNeverReplayed`, `RemoteTransportDisablesHTTPReplayAndIdempotencyHeaders` | R HTTP ambiguity/timeouts and client cancellation tests; drop poisons session, explicit reconciliation/fresh transport; no request replay |
| Definitive protocol and HTTP errors | S `remoteRoundTripper.RoundTrip` distinguishes 4xx and 5xx; G no-replay tests | R `stdio_preflight_bound_is_not_dispatched_and_remote_error_is_definitive`, HTTP ambiguous status tests; Rust HTTP non-success is conservatively unknown after dispatch, unlike Go definitive 4xx |
| Reconnect cooldown, concurrent replacement, closed-manager protection | G `ReconnectServer*`, `ShouldAttemptReconnect`, `ReconnectCannotResurrectClosedManager`; S `CallTool` | R explicit reconnect and catalog-drift tests; **no automatic reconnect/replay**. Go may reconnect for future remote calls and retry a closed stdio call; Rust deliberately never retries either |
| Attempt/completed/unknown audit and fail-closed callbacks | G `RemoteAuditFailsClosedBeforeResourceRequest`; S `auditRemote` call sites | R `audit_and_callback_errors_are_redacted_and_fail_closed`, `cancellation_during_terminal_audit_keeps_session_poisoned`; host persists audit; no database/platform evidence |

### Server mode

| Contract | Pinned Go evidence | Rust evidence / precise delta or limitation |
| --- | --- | --- |
| Selected tools only, mandatory policy + tenant resolver; never execute around policy | G `ServerModeRequiresPolicyAndTenant`, `ServerModeUsesPolicyBoundaryAndImmutableRequest`; `NewServerMode`, `addTool` | R `server::real_http_policy_boundary_and_immutable_digest`, selected-name tests; Rust accepts definitions, not executable tool objects |
| Tool argument digest includes tenant, name, serialized arguments using length prefixes | G `ServerModeUsesPolicyBoundaryAndImmutableRequest`; `serverRequestDigest` | R immutable digest tests; **no cross-language canonical JSON/digest parity claim**, especially HTML escaping/numeric representations |
| Streamable HTTP handler, init/initialized, session headers, JSON-RPC dispatch | G server-mode roundtrip through actual Go SDK; I archived wrapper AST/surface test | R `server.rs` and `end_to_end.rs`; I Rust client → actual Go ServerMode and actual Go Manager → Rust ServerMode. JSON responses, GET 405; reference wrapper itself exposes only Streamable HTTP server mode, proven by executable surface test, not a reduced transport scope |
| Selected resource list/read and prompt list/get, tenant-aware callbacks | G `ServerModeUsesPolicyBoundaryAndImmutableRequest`; `WithServerResources`, `WithServerPrompts` | R `selected_resources_and_prompts_are_tenant_aware_and_redacted`; only explicit entries, no templates/subscriptions/dynamic listing; constructor validates selection |
| Authentication before protocol, tenant-bound sessions, unknown/cross-tenant session denial | G server-mode unauthorized/cross-tenant cases | R `tenant_auth_session_binding_and_cleanup`; real HTTP tests; trusted tenant resolver is host responsibility, never a client tenant-header trust scheme |
| 1,024 global / 128 tenant sessions, 30-minute idle TTL, DELETE and close cleanup | G `ServerModeRequiresPolicyAndTenant`; S reservation/pruning/removal helpers | R `atomic_total_and_tenant_capacity_and_delete_releases_slot`, unit TTL pruning, close tests; Go handler underlying sessions expire on TTL after close; no 30-minute wall-clock or distributed/multiprocess quota test |
| Verified serving TLS, authenticated listener, Host/Origin DNS-rebinding protection | S Go `NewServerMode` host responsibilities; underlying protocol handler | R `dns_rebinding_origin_and_transport_matrix`; exact configured Host/Origin checks, host-owned TLS listener/middleware; no deployed TLS ingress integration and no automatic listener provisioning |
| Invalid request/batch/notification handling, framing, body/result bounds, deadlines | Go protocol SDK delegates wire handling; S wrapper has no corresponding Rust-specific caps | R `malformed_messages_never_execute_policy`, `handler_bounds_chunked_framing_and_incomplete_bodies`, `failure_redaction_request_result_bounds_and_timeout`, and server batch regressions; per-element policy/tenant gates, omitted notification responses, initialization prohibition and aggregate bounds; not complete Go protocol SDK wire conformance |
| Tool input JSON Schema enforcement | S Go `addTool` uses protocol SDK `AddTool` argument handling | Rust compiles selected schemas with jsonschema 0.33 and validates before policy, including batched calls. Explicit offline retriever rejects HTTP/file refs even under Cargo feature unification; local `$defs` refs work. R `tool_schema_validation_is_offline_and_precedes_policy_even_in_batches`; host still owns semantic authorization. No exhaustive cross-library/dialect equivalence claim |
| Safe tool/resource/prompt errors and `isError` results | S `addTool`, resource/prompt callback wrappers; G basic policy roundtrip | R server redaction/result tests; raw callback errors never returned; tool results remain single text plus error flag |
| Optional protocol surfaces | Go wrapper only configures selected tools/resources/prompts, protocol SDK may support more | Rust does not advertise sampling, elicitation, tasks, roots exchange, resource subscriptions/templates, resumable notifications, dynamic list-change events, or protocol extensions; no blanket Go SDK feature parity |

## Verification recorded for this change

The running reference corpus and original Go package tests are recorded in the
linked JSON artifacts, including their exact immutable provenance. Rust tests
compare current implementation output with that generated evidence; regenerating
from Rust is not an available runner path. The original Go suite reports **84
passing test/subtest outcomes and one intentional helper skip** (including the
injected corpus test). This is not 84 distinct integration scenarios.

Fresh Rust 1.88 verification after compatibility and review fixes:
`cargo test --locked --workspace --all-features --all-targets` passed **679 tests**,
including **97 MCP tests** and all four reference tests. The workspace reports
30 ignored entries: 28 interoperability cases executed separately by the pinned
runner, plus two pre-existing explicit Go/sandbox helper entries. A fresh run of
`python3 scripts/mcp-reference/run.py --check --baseline-tests` reproduced the
committed Go observations and baseline outcomes exactly. The pinned
`cargo-deny 0.20.2 --locked check` passed advisories, bans, licenses and sources;
strict workspace Clippy passed through the direct-driver workaround below.
Independent follow-up review found no remaining blocking regression in the
reviewed changes; this is not an exhaustive protocol-interoperability certification.

`rustfmt --check`
for `reference.rs`, `gofmt -l` for the injected test, and Python syntax compilation
were clean. The standard `cargo clippy` launcher failed before compilation because
this container lacks `/proc/self/exe`. The actual installed Clippy driver then
completed successfully with warnings denied via Cargo's wrapper interface:

```sh
RUSTC_WORKSPACE_WRAPPER="$PWD/../scratch/rust-1.88/bin/clippy-driver" \
CLIPPY_ARGS="-D__CLIPPY_HACKERY__warnings__CLIPPY_HACKERY__" \
cargo check --locked -p adk-mcp --all-targets
```

This uses the same Rust/loader/target environment above; it does not suppress
warnings or skip the modified test target. Final logs are in
`../scratch/pr24-final-workspace.log`, `../scratch/pr24-final-clippy.log`,
`../scratch/pr24-final-interop.log` and `../scratch/pr24-final-go-reference.log`.
These disposable logs supplement the committed observed/provenance artifacts.

## Compatibility follow-up: delivered behavior and remaining boundaries

The maintainer-identified observable differences are implemented and tested:
cache TTL/public invalidation, deterministic collision suffixing and routing,
bounded host-visible failure diagnostics, and the expanded optional/empty wire
corpus above. They are not deferred by documentation.

The independent cross-wire runner passed **28/28 cases** across stdio, Streamable
HTTP and legacy SSE clients, plus actual Go Manager → Rust Streamable HTTP
server. Shutdown observations include HTTP DELETE, zero remaining Rust sessions,
successful helper exits and a reaped stdio process. The exact Go wrapper supports
only HTTP server mode; an executable archived-source AST test proves its exported
surface and constructor. Stdio/SSE peers are explicitly the pinned go-sdk protocol
server, not an invented wrapper API. See the linked runner for commands and exact
provenance; this closes the previously reported absence of cross-wire evidence.

Remaining representational differences do not remove a baseline operation:

- Blob filenames are normalized because the Go implementation itself uses
  timestamps/MIME-derived names. Actual bytes, hashes, confinement and modes are
  compared, rather than an unstable generated path.
- Human-readable operational error wording is not copied into Rust. Both sides'
  acceptance/rejection and output placement are compared; Rust exposes typed
  errors. Stderr tail contents are restored through explicit host access (same
  bounded-tail/startup-failure reference cases), rather than copying peer text
  into ordinary errors that may reach a model or logs. This preserves diagnostic
  access and reconciliation classification without weakening credential controls.
- Earlier config rejection and stricter bounds remain explicit fail-closed
  security differences; they never grant permissions or hide a remote replay.

This is baseline compatibility evidence, not exhaustive future protocol conformance.
Unadvertised optional protocol extensions, every MIME/JSON combination, adversarial
DNS scheduling, non-Unix filesystem guarantees and production identity/ingress
integration are outside these tests. Production deployment was not requested or
performed. Host-owned sandbox launchers, identity, approvals and platform adapters
remain required boundaries, not new library authority.
