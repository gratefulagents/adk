# Actual Go ↔ Rust MCP interoperability

Run from the repository root (Python 3.12+, Go, Rust 1.88+):

```sh
env PATH=../scratch/rust-1.88/bin:/usr/local/go/bin:/usr/local/bin:/usr/bin:/bin \
  LD_LIBRARY_PATH=../scratch/rust-1.88/lib \
  CARGO_HOME=../scratch/cargo-home CARGO_TARGET_DIR=../scratch/adk-target-188 \
  python3 scripts/mcp-interop/run.py
```

The runner resolves relative toolchain paths before Cargo changes directories.
Use `--sdk PATH` for another checkout and `--scratch PATH` for disposable builds.
The checkout HEAD must equal `1dc92b73900fac74dc357a938e4b5eee6392b418`.
No code is run from the checkout: like `scripts/mcp-reference/run.py`, this runner
extracts **git archive of that commit**, then copies only this helper into the
archived module. It does not modify/read expected outputs from the reference
harness. `go list -m -json` rejects dependency replacements and versions other
than `github.com/modelcontextprotocol/go-sdk v1.4.1`; `go mod verify` checks cached
module integrity. Module fetching may need network access; cross-wire traffic is
loopback only.

## Coverage / exact implementation boundary

| Client | Server | Transport | Behaviors |
| --- | --- | --- | --- |
| Rust configured `connection::connect` / `Client` | Actual pinned SDK `ServerMode` | Streamable HTTP | 7 |
| Rust configured `connection::connect` / `Client` | Actual pinned **go-sdk protocol** `Server` | stdio | 7 |
| Rust configured `connection::connect` / `Client` | Actual pinned **go-sdk protocol** `Server` | legacy SSE | 7 |
| Actual pinned SDK `Manager` | Rust policy-gated `ServerMode` | Streamable HTTP | 7 |

Each direction/transport has separate tests for tools/resources/prompts discovery,
tool invocation, resource read, prompt retrieval, and session shutdown. Discovery
before invocation preserves the Rust catalog gates. Each test uses an independent
session; only test-owned static credentials, explicit server/origin/private-network
grants and read-only tool allowlists are supplied. Server callbacks assert tenant
context and tool request digest; the Go wrapper's direct `Tool.Execute` panics if
policy is bypassed.

**Unsupported exact wrapper modes are not claimed as passing:** at this pin,
`pkg/agentsdk/mcp/server_mode.go:NewServerMode` constructs only
`mcpsdk.NewStreamableHTTPHandler`, and the wrapper's exported `ServerMode` methods
are `Handler` and `Close`. There is no exported wrapper stdio/SSE server entrypoint.
`surface_test.go` parses all archived non-test package files and asserts this
surface and constructor. The runner also executes the original pinned
`TestServerModeUsesPolicyBoundaryAndImmutableRequest` and
`TestServerModeRequiresPolicyAndTenant` tests. Stdio and legacy SSE rows explicitly
exercise `go-sdk v1.4.1`'s `StdioTransport` / `NewSSEHandler` instead. They do **not**
prove a nonexistent SDK wrapper server mode. The Go client row uses the actual SDK
Manager, not a protocol-client substitute.

## Lifecycle and evidence

Listeners bind `127.0.0.1:0`; the Go server prints its bound endpoint, avoiding
port reservation races and sleep-based readiness. Per-operation I/O bounds are
8 seconds, Rust case deadlines 30 seconds, Go context deadlines 40 seconds, and
runner commands have 600-second deadlines. Child handles kill on drop; Rust
listeners abort on drop; the runner kills its subprocess group on failure/timeout.
Go HTTP helpers close on stdin signal/EOF and confirm exit; successful Streamable
HTTP shutdown records the actual DELETE request. Go Manager shutdown must leave
zero Rust sessions. On Unix, stdio shutdown checks `waitpid` returns `ECHILD` for
the helper's recorded PID (the transport has already killed/reaped it). This is
not a claim of a protocol `shutdown` RPC: MCP ends these sessions via transport
closure/DELETE.

`fixtures/mcp/interop/results.json` is **generated observed evidence**, never an
input or precomputed expected output. It records full commands/output, per-case
actual returned data, child exit/session cleanup evidence, archived source and
protocol source SHA256s, module sums, helper binary and harness hashes, Rust source
hashes, Cargo.lock hash and compiler versions. Ports, PIDs, build paths/timings may
vary between successful runs; this is deliberately not a byte-identical golden.
Source hashes are checked again at completion to reject concurrent source drift.
The runner fails if any case fails or any of the 28 observations is missing, then
runs the entire ordinary `adk-mcp` suite without weakening existing gates.
`interopPassed` is separate from overall `passed` so unrelated regression-suite
failures remain visible. The ignored tests require explicit
`MCP_INTEROP_GO_BINARY` and `MCP_INTEROP_RESULTS`; normal cargo tests do not build Go.

This closes a **high-risk live cross-wire coverage gap** for the listed supported
surfaces. It is not broad protocol conformance/security testing: real TLS/OAuth,
external services, reconnection/fault injection, and unsupported wrapper server
transports remain outside this harness; existing focused tests continue to gate
those behaviors. No production implementation or Cargo manifest changes required.
