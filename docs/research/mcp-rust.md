# Rust MCP research and implementation decision

Research date: 2026-09-19. Registry releases and framework `main` observations are
separated deliberately; a framework dependency is not proof of protocol parity.
No third-party MCP implementation source is copied into ADK.

| Candidate | Version / license checked | Decision |
| --- | --- | --- |
| [Official rmcp registry](https://crates.io/api/v1/crates/rmcp) | 3.4.0 (2026-09-15), Apache-2.0, Rust 1.88 | Adapt ownership and typed error design; do not adopt its transport worker as the security boundary in this baseline. |
| [rust-mcp-sdk](https://crates.io/api/v1/crates/rust-mcp-sdk) | 2.0.0 (2026-08-27), MIT, Rust 1.80; registry also lists 1.1.0 | Reject 2.x for this compatibility baseline: its documented target is 2026-07-28 stateless MCP. Consider 1.x separately if replacing the native implementation. |
| [Rig workspace](https://github.com/0xPlaygrounds/rig/blob/main/Cargo.toml), [rig-rmcp adapter](https://github.com/0xPlaygrounds/rig/blob/main/crates/rig-rmcp/src/lib.rs) | main manifest 0.42.0, MIT, rmcp major 2 | Adapt separation between tool descriptors and execution, preserving structured results; do not import an agent framework. Published rig-rmcp version was not verified (registry 404). |
| [Goose workspace](https://github.com/block/goose/blob/main/Cargo.toml), [integration](https://github.com/block/goose/blob/main/crates/goose/Cargo.toml) | main manifest 1.51.0, Apache-2.0, Rust 1.94.1, rmcp requirement 3.2.0 | Reject framework dependency; toolchain exceeds ADK MSRV. Useful integration reference, not a verified shipped release. |
| [rig-mcp](https://crates.io/api/v1/crates/rig-mcp) | 0.2.5, MIT OR Apache-2.0 | Reject: distinct ForeverAngry/rig-compose bridge, not official Rig's adapter. |

## Exact protocol and security observations

[rmcp 3.4.0 StreamableHttpClientTransportConfig](https://docs.rs/rmcp/3.4.0/rmcp/transport/streamable_http_client/struct.StreamableHttpClientTransportConfig.html)
exposes authorization/custom headers, SSE event-size limits, channel capacity,
POST concurrency, control/recovery timeouts, retry policy and
`reinit_on_expired_session`. Default POST concurrency is 16, but a POST ceases to
count when it opens an SSE stream: this does not bound all active calls/streams.
Expired-session recovery can initialize again and retry an ordinary POST once;
other failures/control POSTs and old timed-out POSTs are not blindly replayed.
Stream reconnection must not be confused with request replay.

Current published rmcp features include child-process/stdio, Streamable HTTP and
Unix-socket HTTP, not a dedicated legacy HTTP+SSE endpoint-event transport. SSE
parsing inside Streamable HTTP is **not** legacy SSE support.
[Retry source on main](https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/src/transport/common/client_side_sse.rs)
has `NeverRetry` and backoff configuration; main's defaults are not a verified
release guarantee. OAuth/TLS features alone do not establish DNS pinning, SSRF,
redirect, reflection-redaction or total response-byte policy.

[rust-mcp-sdk's protocol matrix](https://github.com/rust-mcp-stack/rust-mcp-sdk/blob/main/README.md)
designates 2.x for 2026-07-28 stateless and 1.x LTS for 2025-11-25. Advertising SSE
is not enough evidence to assume every client/protocol combination.

## ADK decision

Implement a deliberately bounded native JSON-RPC/MCP baseline for the SDK's
stdio, Streamable HTTP and explicitly configured legacy SSE client transports.
This is not a translation of the Go SDK's session internals. Owned mutable
sessions serialize operations, use bounded I/O and typed failures, and prohibit
implicit retry/reconnect altogether. Legacy endpoint validation, credential
providers, policy checks, scoped subprocess environment and pinned DNS remain
inside ADK's auditable boundary. A future rmcp integration can implement the same
`Transport` interface, but must first pass the same hostile-peer and no-replay
suite with recovery disabled; current SDK knobs were not assumed sufficient.

This choice costs protocol maintenance. ADK does not claim complete coverage of
all revisions or optional sampling/elicitation/task features. The baseline pins
2025-03-26, the Streamable HTTP introduction, and accepts only explicitly tested
negotiation versions. Server mode follows the Go reference: Streamable HTTP,
not legacy SSE or stdio server mode. No automatic legacy fallback follows an
arbitrary error.

Reference requirements were independently inspected locally in
`repos/sdk/pkg/agentsdk/mcp/{manager,remote,server_mode,snapshot,tools}.go` and their
tests. In particular: repository configuration is not host authority, remote
tools require three read-only gates, credentials are tenant/server scoped,
redirects and proxy discovery are off, and uncertain calls require reconciliation.

[Official transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
distinguishes stream resumption from replay, and discusses legacy fallback only
for specific initialize errors (400/404/405). Tests must count actual dispatches,
not merely assert that an error was returned.
