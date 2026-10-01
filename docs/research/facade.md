# Issue #11 facade research — pre-implementation snapshot

This document records the issue #11 audit at Rust revision
`489b1886aa760b6a2d4680d42bd3dce0a1397407`. It is a research and traceability
snapshot, not acceptance of concurrent builder, observability, or example work.
Those changes require their own final command output and acceptance-ID evidence.

The source-parity baseline remains SDK **v0.0.115** at
`1dc92b73900fac74dc357a938e4b5eee6392b418`. The local `repos/sdk` checkout
observed during this audit was **v0.0.116** at
`63afe2ed8cc5f13ca7469054f2c1cb812fcac801`; it contains computer-use metadata
changes after the baseline. Do not use that checkout to silently update the
ledger or to establish v0.0.115 parity.

## Facade boundary

`crates/adk/src/lib.rs` is the public, feature-gated entry point. The audit
observed these associations; they identify code to inspect, not semantic
equivalence to every Go record routed to a similarly named SDK module.

| Facade surface | Feature | API reference | Test reference | Audit interpretation |
|---|---|---|---|---|
| Native contracts | always available | `adk_core::{contracts, types}` | `crates/adk-core/tests/contracts.rs` | Native model, tool, host, run, error, and policy contracts. |
| Runner | `runtime` | `adk::runtime`, `adk-runtime/src/runner.rs` | `crates/adk-runtime/tests/runner.rs`, `crates/adk/tests/tool_runtime.rs` | Runner and lifecycle ownership association only. |
| Tool composition | `tools` | `adk::tools`, `adk-tools/src/bundle.rs` | `crates/adk-tools/tests/registry.rs`, `registry_matrix.rs` | A registry/bundle test does not close every routed tool, schema, registration, or permission record. |
| Provider adapters | `providers`, `providers-runtime` | `adk::providers`, `adk-providers/src/factory.rs` | `crates/adk-providers/tests/contracts.rs`, `retry_parity.rs` | Provider mapping requires source-specific wire, auth, retry, and live evidence. |
| Durable state | `durable` | `adk::durable`, `adk-durable/src/lib.rs` | `crates/adk-durable/tests/contracts.rs`, `adk-runtime/tests/durable.rs` | Persistence/recovery association only; no crash or exactly-once conclusion follows. |
| MCP | `mcp` | `adk::mcp`, `adk-mcp/src/lib.rs` | `crates/adk-mcp/tests/interop.rs`, `reference.rs` | Protocol references do not close every MCP discovery, policy, or dynamic-schema obligation. |
| Execution boundary | `execution` | `adk::execution`, `adk-security/src/policy.rs` | `crates/adk/tests/execution.rs`, `adk-security/tests/security.rs` | API association; OS/process and authorization semantics remain record-specific. |
| Observability | `observability`, `otel` | `crates/adk/src/observability.rs` | No test path recorded at the audit revision | Concurrent implementation target, not evidence at the audit revision. |

The same table is machine-readable in
[`issue-11-overlay.json`](../migration/ledger/issue-11-overlay.json) under
`module_reference_map`. It is deliberately separate from each acceptance
entry's `semantic_closure`: references have no power to auto-verify or close an
acceptance ID.

## OpenTelemetry research

The in-progress facade manifest requests `opentelemetry = "0.31"` with default
features disabled and `trace`; tests request `opentelemetry_sdk = "0.31"` with
default features disabled and `trace,testing`. The audited lockfile resolves
both to **0.31.0**:

| Locked crate | Exact evidence | License and compiler declaration | Relevant verified API |
|---|---|---|---|
| [`opentelemetry` 0.31.0](https://crates.io/api/v1/crates/opentelemetry/0.31.0) | `Cargo.lock` checksum `b84bcd6ae87133e903af7ef497404dda70c60d0ea14895fc8a5e6722754fc2a0` | Apache-2.0; declared Rust 1.75.0 | [`trace::Tracer`](https://docs.rs/opentelemetry/0.31.0/opentelemetry/trace/trait.Tracer.html), including `build_with_context`, is available with `trace`. |
| [`opentelemetry_sdk` 0.31.0](https://crates.io/api/v1/crates/opentelemetry_sdk/0.31.0) | `Cargo.lock` checksum `e14ae4f5991976fd48df6d843de219ca6d31b01daaab2dad5af2badeded372bd` | Apache-2.0; declared Rust 1.75.0 | [`trace`](https://docs.rs/opentelemetry_sdk/0.31.0/opentelemetry_sdk/trace/index.html) exposes `SdkTracerProvider`, tracer/processor types, and the testing in-memory exporter. |

Registry metadata reports both releases as published on 2025-09-25; the
version-specific rustdoc pages establish the listed API surface. This evidence
is not a legal approval, transitive-license approval, exporter configuration, or
a claim that the optional bridge compiles on the project's Rust 1.88 baseline.

### Boundary decision recorded by the audit

Adapt the tracer boundary, not the OpenTelemetry SDK lifecycle:

- The embedding host supplies a tracer and explicit parent context.
- The embedding host configures exporters, owns its tracer provider, and flushes
  or shuts it down.
- The facade must not install a global provider, discover environment settings,
  open a network connection, or select an exporter as an import side effect.
- Use an in-memory exporter only for offline bridge assertions. It is not live
  exporter evidence.

This is a scoped API decision. It does not prove ordering, redaction, metrics,
logs, exporter behavior, or semantic parity with SDK telemetry records.

## Framework comparison retained from baseline research

The broader migration research remains in
[`docs/migration/rust-research.md`](../migration/rust-research.md). Its
version-pinned findings remain applicable to issue #11 only as patterns:

| Candidate | Exact inspected source | License | Audit use |
|---|---|---|---|
| [Rig `rig-core` 0.42.0](https://docs.rs/crate/rig-core/0.42.0/source/src/lib.rs) | [portable tools](https://docs.rs/rig-core/0.42.0/src/rig_core/tool/mod.rs.html) and [scripted completion test utilities](https://docs.rs/rig-core/0.42.0/src/rig_core/test_utils/completion.rs.html) | MIT | Adapt portable-tool and scripted-fake patterns; do not infer a complete agent runtime or add mandatory framework coupling. |
| [genai 0.6.5](https://docs.rs/crate/genai/0.6.5/source/examples/c20-tooluse.rs) | Version-specific tool-use example | MIT OR Apache-2.0 | Retain explicit call-ID correlation and application-owned dispatch; the example is not a complete multi-call dispatcher. |
| [ADK-Rust `adk-core` 2.2.0](https://docs.rs/adk-core/2.2.0/src/adk_core/model.rs.html) | Version-specific model source | Apache-2.0 | Do not select for the Rust 1.88 baseline: registry metadata declares Rust 1.95. Adapt typed-context ideas only. |

The first two rows are prior research, not dependencies selected by this
facade. Do not substitute a same-named external type for a source-parity
acceptance obligation.

## Issue #11 builder, embedding and telemetry comparisons

The audit additionally inspected these maintained versioned APIs (publication
activity is evidence of maintenance, not an SLA). Patterns are adapted without
copying external framework implementation or adding a framework dependency.

| Inspected version and license | Source and maintenance evidence | Adopt/adapt | Reject |
|---|---|---|---|
| Rig `rig-agent` **0.42.0**, MIT | [AgentBuilder](https://docs.rs/rig-agent/0.42.0/rig_agent/agent/struct.AgentBuilder.html), [registry](https://crates.io/api/v1/crates/rig-agent/0.42.0); published 2026-08-17 | Fluent owned composition; distinguish supplied tool servers from builder-added tools; content telemetry off by default | Copying turn-zero semantics or silently bypassing memory without a conversation ID. Current agent builder is in `rig-agent`, not the older `rig-core::agent` surface. |
| ADK-Rust **2.2.0**, Apache-2.0 | [facade](https://docs.rs/adk-rust/2.2.0/adk_rust/), [registry](https://crates.io/api/v1/crates/adk-rust/2.2.0); published 2026-09-01, Rust 1.95 | Component injection, fallible builder, optional capability boundaries | Mandatory whole-framework dependency on Rust 1.88; copying defaults that enable Gemini/runner/sessions into a minimal contract facade |
| `adk-telemetry` **2.2.0**, Apache-2.0 | [initialization source](https://docs.rs/adk-telemetry/2.2.0/src/adk_telemetry/init.rs.html), [registry](https://crates.io/api/v1/crates/adk-telemetry/2.2.0); published 2026-09-01, Rust 1.95 | Separate telemetry/exporter features and explicit full-content capture | Library-owned global subscriber/provider initialization and global replacement as a session lifecycle |
| `tracing-opentelemetry` **0.33.0**, MIT | [layer API](https://docs.rs/tracing-opentelemetry/0.33.0/tracing_opentelemetry/), [registry](https://crates.io/api/v1/crates/tracing-opentelemetry/0.33.0); published 2026-05-18, Rust 1.75 | Host-composable tracer and explicit parent context | Adding it to this OTel 0.31 bridge: it targets OTel 0.32. Also not an OTel log exporter |

CLI ergonomics follow the same explicit-host boundary: runnable examples select
named offline scenarios; invalid selections fail instead of silently loading
credentials or switching into a worker. The existing development binaries retain
their documented narrow interfaces. The prohibited worker CLI is not reproduced.

### YAML parser

Host configuration uses `serde_yaml_ng` **0.10.0**, imported locally as
`serde_yaml` to keep parser references readable. [Registry metadata](https://crates.io/api/v1/crates/serde_yaml_ng/0.10.0)
reports MIT, Rust 1.64 and an unyanked release (2024-05-26); [source](https://github.com/acatton/serde-yaml-ng)
is a fork rather than the deprecated `serde_yaml` crate. Input remains trusted
host configuration with single-component mode/role names, not an unrestricted network
payload or repository-authorized policy source. The lockfile pins the resolved
parser and libyaml binding; this choice does not waive dependency checks.

## What remains to validate

Final validation belongs to the parent implementation work. It must attach
acceptance-ID-specific symbols, Rust test identifiers, features/targets, command
output, and approved divergences to the overlay. It must also separately report
offline versus credentialed/live provider and operating-system evidence.
