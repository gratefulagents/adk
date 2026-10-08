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

## Project-state blocking lifecycle

Local project-state composition uses explicit owner/revocable tool handles,
not Go callback/closer aliases. Tokio 1.53.1 (MIT; pinned in `Cargo.lock`) runs
synchronous operations on its blocking pool. An admitted-operation lease lives
inside that blocking operation, not inside the waiting future: cancellation
cannot abandon drain accounting or replay a write. Close revokes admission,
then drains leases; injected application-owned references remain independent.
See the pinned crate's `src/task/blocking.rs` documentation and
<https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html> for the
non-abortable blocking-task constraint. No new dependency was introduced.

Adopt explicit host path inputs and four independent feature switches. Reject
process-environment discovery during composition and enabling `ExtraTools` as
a side effect. Raw initialization/priming errors are not exposed because they
can contain private paths or store contents. These are native host-boundary
choices, not claims that SDK failure diagnostics or teardown behavior match.
The independent sixteen-case pinned builder fixture checks only store creation,
selected tool names and full-builder working-state text. Tool-only observations
execute the SDK's separate BuildToolBundle path; neither proves all project-state
contracts.

## Run-wide instruction composition

Adopt a single owned `RunnerConfig::additional_instructions` string, rather
than Go's deprecated `ModeInstructions` alias and callback-style mutation.
Composing at each model request is necessary: prepending text to the parent
agent loses the run-wide instruction after a handoff. Effective trimmed text is
also part of durable compatibility identity; changing it cannot resume a saved
request under different instructions. No dependency or platform host is needed.

The source contract is the GPL-3.0-only pinned SDK's
[`buildRunInstructions`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/runner.go)
and runtime builder's
[`additionalInstructions`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/runtime/builder.go).
Independent actual-run fixtures retain the exact separator and builder aggregate
whitespace. Reject copying an unverified summary of those helpers: this revision
uses `\n\n---\n\n`, not just blank lines between run-level sections.
MCP context has a separate Unicode-printability contract and remains separate
from this additional-instructions mapping.

## MCP Unicode prompt metadata

Adopt `unicode-general-category` **0.6.0**, Apache-2.0, no dependencies/no_std,
with an exact compatibility pin. The
[registry version metadata](https://crates.io/api/v1/crates/unicode-general-category/0.6.0)
confirms the unyanked release/license; the
[versioned source](https://docs.rs/unicode-general-category/0.6.0/src/unicode_general_category/lib.rs.html)
reports Unicode **15.0.0**, matching the pinned SDK's Go toolchain. The maintained
[upstream repository](https://github.com/yeslogic/unicode-general-category)
has newer Unicode releases, deliberately not adopted for this compatibility rule.
Cargo source checksum: `2281c8c1d221438e373249e065ca4989c4c36952c211ff21a0ee91c44a3869e7`.
No platform service or process dependency is added.

Reject `char::escape_debug`, ASCII-only control filtering, and a newer Unicode
category table: they do not implement Go's `unicode.IsPrint` contract. Exhaustive
scalar observations include code points printable in newer Unicode but unassigned
in the pinned version. The source's comment says bytes, but its actual truncation
uses runes; retain 64 scalar values and verify multibyte boundaries independently.
The native formatter preserves list order/duplicates; it is not a registration
policy or prompt-injection security boundary.

## Metadata compaction composition and Unicode lookup

Adopt the same pinned Unicode 15 category table for model-ID normalization. The
local 0.6.0 package manifest/license and Cargo checksum were rechecked; no new
package version is introduced. Reject raw Rust `char::to_lowercase` alone: the
independent pinned Go scalar digest failed until characters unassigned in Unicode
15 were preserved. Use simple, non-expanding lowercase for assigned characters.
The corrected implementation matches all 1,112,064 Unicode scalar observations.

Apply the same simple casing to static model-default selection. Forty-five
observations from the [pinned public helper](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/run_config.go#L199)
(GPL-3.0-only) exposed expanding lowercase selecting an unsafe larger budget for
`gpt-5.6-MİNI`. Keep Rust's associated constructor and explicit resolver injection
rather than reproducing Go's mutable public function-variable alias.

Adopt an explicit session-owned async resolver and single-flight
cache instead of process-global credential discovery or a blocking Go mutex.
Preserve successful-fetch lifetime, failed-fetch 30-second cooldown and the
builder's 15-second lookup bound. Expose raw cached lookup separately from runtime
threshold/static fallback, and return structured redacted diagnostics to the
embedding host instead of copying Go's global stderr logging. Mutable remote
metadata deliberately has no durable key. Reference:
[pinned resolver source](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/providers/openai/compaction.go),
GPL-3.0-only, executed by `scripts/metadata-compaction-reference/run.py`.
Twelve additional cold/warm cancelled/expired-context observations establish
that public lookup serves cached metadata regardless of context state and caches
failed fetches for the cooldown. Adopt those public lookup semantics; keep
cancellable waiting and pre-dispatch cancellation enforcement in the native
runner-facing threshold adapter instead of applying them to raw cached reads.

## Model-written local summaries

Adopt a native owned compaction plan with explicit protected/removed indices,
rather than rediscovering source identity by comparing message values. Repeated
messages can have different authors or approval boundaries. Keep pure planning
provider-free and perform optional summary calls in the runner, where active
model selection, cancellation, usage budgets and lifecycle are already owned.
Preserve the pinned SDK's default-on setting and out-of-band complete-call
semantics; reject process-global logging in favor of a redacted host observation.
Include the setting even at its default in durable identity, so an older
deterministic-only checkpoint cannot silently acquire new model-call behavior.
Reference: [SDK summary helper](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/compaction_llm.go),
GPL-3.0-only, independently executed by `scripts/llm-summary-reference/run.py`.

## Default managed child selection

Adopt the [pinned runtime's `defaultAsyncSubAgent` selection](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/runtime/builder.go#L802)
(GPL-3.0-only): prefer exact `agent`, otherwise lexical first nonblank name.
Use the native session scheduler's immutable registrations as the authoritative
catalog, not the parent name or separately configured handoff targets. Reject
implicit parent-to-child identity coupling: the parent need not be a registered
child. The disposable runtime reference harness executes eleven catalog cases;
native tests exercise the actual managed tool and scheduler plus explicit-name
overrides. Automatic owned scheduler construction remains separate work.

## Automatic child assembly

Adopt explicit owned child-host injection and an immutable per-session registry,
rather than the SDK's mutable scheduler reconfiguration. The embedding host may
supply an existing scheduler; borrowed owners are never changed. A private
`OnceLock<RunnerChildExecutor>` resolves tool/scheduler construction order and
is populated before the bundle exposes its handles. Child runners deliberately
omit parent session/admission callbacks, preventing an ownership cycle and
consumption of the parent's input queue. Keep native scheduler safety ceilings
and explicit persistent-store injection instead of copying unlimited defaults.

The pinned GPL-3.0-only [runtime assembly](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/runtime/builder.go),
[specialist construction](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/specialists.go)
and [child run configuration](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/subagent_registry.go)
are hashed in the disposable oracle. Twelve cases execute real SDK composition
and synchronous child calls; native tests compare selection/registration and
exercise additional safety/lifecycle guarantees. Generated named specialist tools
are not attached by the SDK runtime builder: it uses the managed task tools.
Do not claim missing automatic named/nested tools solely because the native
builder also leaves those to explicit composition.

## Named child output projection

Adopt the [pinned SDK agent-tool success contract](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/agent_tool.go)
(GPL-3.0-only): successful named calls return final text, empty extraction falls
back to final text, and empty final text becomes `(no output)`. Thirty-six actual
SDK calls cover raw/empty/whitespace/JSON/Unicode text and parsed structured results
with absent, empty, final-text and full-value extraction. Use an explicit per-registration `Arc<ChildOutputExtractor>`
in `RunnerChildExecutor` rather than a second unmanaged execution path. The
callback receives the full native `RunResult`, only on completion; host-selected
registration aliases distinguish named-only projections from managed ones.
Preserve native pending/failed-task envelopes and scheduler panic containment;
those stronger lifecycle/error contracts are not claimed byte-compatible.

## Analysis-record nulls and open labels

The [pinned snapshot records](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/llm_snapshot.go)
and [model settings](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/model_settings.go)
(GPL-3.0-only) use Go zero-valued slice elements and a string item label.
Independently executed decoding/encoding matrices cover 54 response and 34 request
records. Adopt Serde optional-element decoding followed by zero-value conversion,
not dropping nulls or writing a second JSON parser. Reject a closed Rust enum for
analysis labels: preserve opaque strings in `Other(String)` and represent the
empty label explicitly. Keep strict known-kind admission at executable recovery
and transcript boundaries, where accepting opaque labels would fabricate state.
These fixtures close field representation claims, not every decoder behavior or
provider response profile.

## Settings merge helper

Expose value composition on the existing typed compatibility settings record,
without changing native builder precedence. The pinned `ModelSettings.Merge`
uses present optionals, positive integer limits and nonempty strings/lists;
whitespace strings are not normalized. Thirty-six independent source executions
cover three bases and twelve override sets. Adopt an owned Rust result with no
input mutation or pointer/slice aliases; reproducing Go aliasing would undermine
safe standalone composition without improving the value contract. Provider range
validation remains separate from merging. The executable settings/routing example
passes the resulting map through actual builder mode and role routing.

## Builder structured output

Forward the existing native `AgentConfig` schema/name/strict/parser fields through
`builder::Config`, rather than replicating Go's function field or adding a second
parser abstraction. Keep the [pinned builder's parent-only placement](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/runtime/builder.go#L699)
and [custom-parser precedence](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/output_schema.go)
(GPL-3.0-only). Retain native construction-time schema checks and observational
schema diagnostics; do not pretend Go's JSON-syntax fallback performs JSON Schema
validation. Fourteen normal/streamed pinned observations compare composed schema
placement, request fields and parsed final values, including parser failure.

## User-input helper composition

Expose the pinned SDK's inspection helpers under `adk::codec::userinput`, rather
than introducing Go callbacks or a CLI-owned pause loop. Hosts retain the decision
to pause. Owned `QuickAction` strings and `Option<Vec<u8>>` preserve the distinction
between absent actions and present JSON `null`; `marshal_quick_actions` accepts an
optional slice to retain nil-versus-empty encoding. The decoder uses in-place
field/array updates because Go's repeated-key slice reuse is observably different
from replacing a Serde vector. Do not generalize this private decoder to unrelated
SDK types without independent evidence. See [usage, limits and pinned source
provenance](../user-input.md) and the 51-case independently executed fixture.

## Final-answer verifier ownership

Adopt an async, owned `FinalAnswerVerifier` trait with borrowed JSON output and
host-owned diagnostics, rather than copying a Go `func(context.Context, string)`
callback and process-global logger. This preserves structured output without a
serialize/parse round trip, and keeps private error text behind the existing
capture-policy boundary. The SDK's invocation order, once-per-run guard, blank
feedback, error acceptance, confirmation reset and feedback prompt remain
independently tested. Parent cancellation remains authoritative in Rust.

For durable use, adopt the existing stop-gate contract: explicit stable identity
and pure/replay-safe behavior. Persist the invocation flag, but reject claims of
exactly-once execution before a result checkpoint commits. A live critic is not a
pure callback. Source: pinned SDK `internal/agent/runner.go` and
`internal/agent/run_config.go`; hashes, version and GPL-3.0-only provenance are in
`fixtures/verifier/observations.json`. These changes do not close the separate
`NewCriticVerifier` helper or the full `RunConfig` ledger obligation.


### Owned critic helper

Adopt a dedicated owned runner plus host and original task, rather than a Go
closure retaining nullable runner/agent pointers. Rust ownership eliminates the
nil-pointer configuration state and prevents mutation of the caller's agent.
Retain the pinned default prompt, verdict parser, read-only tool policy,
12-turn limits and forced summary. Reject automatic durable replay identity for
a live critic; read-only model calls are not deterministic pure checks.
The inherited cancellation token bounds critic work without a detached task.
The source/version/license and file hashes are independently recorded in
`fixtures/critic/observations.json` (SDK commit
`1dc92b73900fac74dc357a938e4b5eee6392b418`, GPL-3.0-only,
https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/verifier.go).
The helper is advisory, never an authorization or evidence-authentication gate.

### Per-attempt instruction providers

Adopt an owned `InstructionProvider` trait and borrowed `InstructionContext`
rather than the Go `func(*RunContext, *Agent) string` field. An async fallible
boundary supports host cancellation/deadlines without detached callback work;
static fallback and dynamic precedence remain exact. Re-evaluate after retries
and on destination-agent handoffs, not just once per run. Compose policy and
structured-output directives after resolving the base prompt. Require explicit
pure-provider identity for durable execution and include it in the agent catalog
fingerprint; do not pretend arbitrary dynamic services are replay-safe.
Source: SDK GPL-3.0-only at
https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/agent.go
and `internal/agent/runner.go`; pinned file/version/license provenance and 16
independent normal/streamed observations are in
`fixtures/dynamic-instructions/observations.json`.

### Per-agent tool-result finalization

Retain SDK exact-name stopping and first-result selection after a complete batch.
Use `BTreeSet<String>` and `ToolFinalOutput::{FirstTool, Json(Value)}` rather than
nullable Go structs containing a boolean and potentially invalid raw bytes.
Do not infer stopping merely from an output selection. Preserve the observable
raw-JSON versus text distinction with explicit result-origin metadata: Go
`json.RawMessage("\"quoted\"")` serializes as a JSON string but is not a Go string
for `FinalText()`. Treating all JSON strings as ordinary text would be wrong.
Keep output guardrails on the stop path, including a null final value, without
reapplying model output-schema parsing to explicit tool-result selection.
Native checkpoints preserve the turn decision; reject ambiguous Go post-tool
recovery rather than infer it from lossy history. Source is GPL-3.0-only SDK
`internal/agent/agent.go`, `runner.go`, and `run_result.go` at
`1dc92b73900fac74dc357a938e4b5eee6392b418`; the 34 normal/streamed observations and
source hashes are in `fixtures/tool-stopping/observations.json`.

### Owned handoff construction

Adopt `Handoff::new(Arc<AgentConfig>)` plus explicit mutable definition fields
instead of nullable target pointers and Go functional options. Preserve target
identity, case-sensitive default naming, metadata, schema and flags. Do not
reuse the catalog naming helper: the pinned SDK uses two different algorithms.
Preserve original role/generic target descriptions independently of the catalog's
trimmed/overridden tool description. Reject a standalone no-op transfer wrapper:
actual transfer belongs to the runner lifecycle. SDK GPL-3.0-only sources are
`internal/agent/handoff.go`, `agent_tool.go`, `pkg/agentsdk/specialists.go` and
`runtime/builder.go` at `1dc92b73900fac74dc357a938e4b5eee6392b418`; file hashes,
version/license and 44 independently generated cases are in
`fixtures/handoff-constructor/observations.json`. Callback/gating/filter parity
is explicitly separate and remains incomplete.
