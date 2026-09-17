# Read-only LSP port

## Baseline evidence

- `repos/sdk/pkg/agentsdk/tools/lsp/lsp.go`: pinned operation/schema contract (43), routing (`getManager`), execute/document synchronization (`executeRequest`), document closure after each operation (357), validation and one-based UTF-16 conversion (476), bounded JSON output (`marshalResult`).
- `repos/sdk/pkg/agentsdk/tools/lsp/manager.go`: host configuration (33), stable result fields (161), initialization/capabilities negotiation (327), read-only server request handling (530), workspace-scoped sessions and bounded queues.
- `repos/sdk/pkg/agentsdk/tools/lsp/protocol.go`: Content-Length framing, 8 KiB headers, inbound/outbound message bounds.
- `repos/sdk/pkg/agentsdk/tools/lsp/results.go`: normalization (34), confined file URIs (116), hover (139), symbol recursion/node bounds (202), diagnostics (271).
- `repos/sdk/pkg/agentsdk/tools/lsp/lsp_test.go`: aliases, UTF-16, routing, confined results, fake-server protocol and lifecycle tests.

## Wiring (parent-owned)

Expose `#[cfg(any(target_os = "linux", target_os = "macos"))] pub mod lsp;` from `adk-tools/src/lib.rs`. The implementation uses the existing Linux secure workspace reader and `write::resolve_existing`; no shared source was changed by this work. `adk-sandbox` must be an adk-tools dependency.

Construct `lsp::tool(lsp::Config { executor: Arc<adk_sandbox::Executor>, servers: Vec<lsp::ServerConfig>, discoverer: Option<Arc<dyn lsp::ServerDiscoverer>> })`. It returns `Arc<lsp::LspTool>`, coercible to `Arc<dyn adk_core::Tool>` for `Registry::build`. Keep the concrete handle and call `close().await` during host shutdown. Drop requests background cleanup; the Tokio runtime must remain alive, as required by ProcessSession.

`ServerConfig` has host-owned absolute `command`, `args`, `env`, `id`, `language_id`, `file_patterns`, startup/request durations and message/output/stderr/queue limits. Defaults match the Go limits; an executable is deliberately not guessed. `ServerDiscoverer::discover(context, workspace, request)` may return additional host-trusted candidates. Missing/ambiguous matches fail. Sessions are keyed by canonical workspace and complete selected configuration.

## Safety and behavior

- Definition is cloned from the pinned LSP manifest, retaining schema, read-only flag, aliases and approval behavior.
- Only confined `Executor::start_session` is used, always read-only, network-denied and piped. Local/unconfined executors are rejected by sandbox policy. Model arguments cannot select executable, arguments, environment or sandbox policy.
- The host environment remains subject to the sandbox's locale/terminal-only allowlist. There is no unsafe environment or PATH fallback.
- Background transport drains stdout/stderr, frames bounded messages, handles server requests and published diagnostics, and treats any ProcessSession truncation as fatal. Startup/request timeout, cancellation, future drop and host close request process-group cleanup; normal error returns await cleanup.
- `initialize` advertises UTF-16; incompatible server encoding fails. Each operation opens current UTF-8 file contents and closes the document afterward, matching the baseline (therefore subsequent edits are sent in a fresh didOpen, not a persistent didChange).
- All eight normalized read-only operations, both aliases, pull/push diagnostics and published fallback are implemented. Results use one-based UTF-16 ranges, omit empty optional fields and filter escaped/non-file URI locations/symbols.
- Source files use secure workspace opens, regular/single-link checks, UTF-8 validation and a 16 MiB limit. Symlinks in source paths are rejected by the existing secure reader; result URIs resolve existing ancestors and reject workspace escapes.

## Deterministic Go/Rust differential corpus

Pinned SDK commit: `1dc92b73900fac74dc357a938e4b5eee6392b418`.

- `lsp-cases.json`: 34 result inputs (26 happy/null/filtering cases, 8 malformed-result rejection cases) and 15 operation inputs (10 accepted, including both aliases; 5 rejected).
- `lsp-generate.go`: executes the actual pinned private `parseResult` and `normalizeOperation` functions and public tool metadata/schema. It does not reimplement the parser or spawn a language server.
- `lsp-generate.py`: checks the SDK commit, injects the Go test using a temporary Go overlay, and leaves the SDK tree untouched.
- `lsp-expected.json`: real Go outputs. Temporary workspace prefixes alone are replaced with `@ROOT@`; diagnostic filePath is attached as the Go execute layer does. Successful JSON, all definition fields/schema, and operation/error mappings are compared exactly. Malformed-result cases compare rejection, **not cross-language parser error strings**; the original Go errors remain in the fixture.
- Rust `pinned_go_differential_corpus` replays all 50 contract checks (34 results + 15 operation mappings + 1 complete definition) through the actual Rust implementation, without sandbox availability gates.

Regenerate from the repository root:

```sh
python3 fixtures/tools/lsp-generate.py
cargo test -p adk-tools --lib lsp -- --nocapture
cargo test -p adk-tools --test lsp -- --nocapture
ADK_REQUIRE_SANDBOX=1 cargo test -p adk-tools --test lsp -- --nocapture
```

Two actual Go generations produced identical SHA-256:
`69110a13961c890ce957704e6dd06e96e2693b4372d1ddd4e4aec0e00876ac50`.

## Fresh verification and remaining limits

- LSP unit tests: **12 passed, 0 failed, no skips**. This includes the 50-check differential replay; existing scripted initialization/all ten input operation mappings/didOpen/didClose; framing and result bounds; and new cancellation/deadline/timeout/close interruption, drop stop signaling, awaited cleanup-error propagation, EOF/RPC-error/missing-result rejection. Scripted/channel tests do not demonstrate OS process cleanup.
- Integration harness reports **8 passed**, but **4 explicitly SKIP** before their cases because Bubblewrap cannot read `/proc/sys/kernel/overflowuid`. Actual fully exercised integration tests: **4**, covering routing/source validation/terminal close, trusted discovery confinement, Local-backend refusal, unsafe environment rejection, and model attempts to override access/network/command. The forbidden executable has a write marker, checked absent to catch raw-process fallback.
- Overall Rust: **16 test functions exercised, 4 skipped**, not 20 verified tests.
- Running the integration binary with `ADK_REQUIRE_SANDBOX=1` produced **4 passed, 4 failed** (all four failures were required-backend availability). Thus supported CI cannot silently count these skips as passes. Linux/macOS integration cfg is enabled; the default executor is already `Backend::Auto`. Python is selected only from host-owned absolute system/Homebrew paths.
- `rustfmt --check` passed on all four LSP Rust files. Strict Clippy passed for the selected adk-tools library and LSP integration targets with `--no-deps -D clippy::all`. Including dependency linting exposed pre-existing `collapsible_if` errors at `adk-durable/src/filesystem.rs:94` and `store.rs:137`; no unrelated files were changed.
- The container has no `/proc/self/exe`. Verification used the installed Rust toolchain directly, its library directory in `LD_LIBRARY_PATH`, and `RUSTFLAGS='-C link-arg=-fuse-ld=bfd'`. Cargo-Clippy itself also requires proc; direct `RUSTC_WORKSPACE_WRAPPER=.../clippy-driver` with `CLIPPY_ARGS='--no-deps__CLIPPY_HACKERY__-Dclippy::all'` ran the same target linting. Go required `GOROOT=/usr/local/go GOTOOLCHAIN=local`; its telemetry sidecar warning did not prevent successful generator tests.

Still unverified here: real confined stdio operations/server requests/write denial; real pull/push/fallback diagnostics; process timeout/restart/encoding/framing/truncation; cancellation/deadline/host close and process-group cleanup; macOS runtime behavior; actual gopls/rust-analyzer compatibility. The four enforcing integration tests cover those scripted server scenarios only when an OS backend works. This corpus is parser/schema/operation-normalization differential evidence, **not** a Go/Rust full-transport differential. Exact external error phrasing, all possible malformed payloads, and routing/discovery cross-language differential coverage remain outside it. No unconfined fallback was introduced.
