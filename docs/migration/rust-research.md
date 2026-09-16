# Rust framework research — issue #3 baseline

## Scope and pins

Actual published Rust source was inspected via version-specific docs.rs pages, not just READMEs. No Rust compilation, tests or live providers were exercised. Versions are those observed during inspection; no claim that they existed at an earlier historical baseline is intended.

| Candidate | Exact inspected version | Maintenance evidence |
|---|---|---|
| Rig | `rig-core = "=0.42.0"` | [Registry metadata](https://crates.io/api/v1/crates/rig-core): published 2026-08-17. |
| genai | `genai = "=0.6.5"` | [Registry metadata](https://crates.io/api/v1/crates/genai): stable alongside newer 0.7 prereleases. |
| ADK-Rust | `adk-core = "=2.2.0"`, ecosystem `adk-rust = "=2.2.0"` | [Registry metadata](https://crates.io/api/v1/crates/adk-rust): published 2026-09-01. |

Publication activity supports maintenance assessment, not an SLA. Commit SHAs and target compiler compatibility were not independently established. This is bounded source research, not a selected dependency set.

## Rig: model contracts, portable tools, deterministic testing

The inspected [library source](https://docs.rs/crate/rig-core/0.42.0/source/src/lib.rs) separates provider-neutral contracts from the classic agent runtime: the builder/run loop belongs to sibling `rig-agent`, not rig-core. Older agent-import examples are version-mismatched. `rig-agent` itself was not pinned or inspected.

Models follow client → `CompletionModel` → completion request → typed `AssistantContent`. The [tool module](https://docs.rs/rig-core/0.42.0/src/rig_core/tool/mod.rs.html#1-14) exports `PortableTool`, `PortableDynamicTool`, `ToolOutput`, `ToolExecutionError` and `ToolResult`. Its declared boundary deliberately excludes mutable execution context and an executor.

**Adapt:** isolate reusable tool logic from application-owned authorization, state and scheduling. This does not claim every portable-tool method was inspected.

The actual [mock implementation](https://docs.rs/rig-core/0.42.0/src/rig_core/test_utils/completion.rs.html#50-347) provides `MockTurn::text`, `tool_call`, `error`, `request_error`, scripted completion/streaming queues, captured requests via `requests()`/`request_count()`, usage/provider IDs, and an error on queue exhaustion. The [test module](https://docs.rs/crate/rig-core/0.42.0/source/src/test_utils/mod.rs) also exports recording/sequenced HTTP clients and failing/counting memory helpers. Downstream users need `test-utils` (`cfg(test)` alone does not enable dependency helpers).

**Adopt pattern:** a scripted fake model records every request and fails unexpected extra calls; tool call/result IDs remain real relationships rather than normalized-away values.

The [memory contract](https://docs.rs/rig-core/0.42.0/src/rig_core/memory.rs.html#84-116) exposes async ordered-message `load`, `append`, `clear`; successful-turn appends include call/result pairs. This is not crash-atomic side-effect persistence. The [in-memory implementation](https://docs.rs/rig-core/0.42.0/src/rig_core/memory.rs.html#328-398) uses shared locked process memory and loses history on restart.

## genai: explicit transport and continuation

The inspected [tool-use example](https://docs.rs/crate/genai/0.6.5/source/examples/c20-tooluse.rs) declares a JSON Schema tool, calls `Client::exec_chat`, extracts calls, simulates execution, constructs `ToolResponse` with the original `call_id`, appends calls/results and continues with `exec_chat_stream`.

**Trap:** the example handles only the first call while appending all returned calls; it is not a complete multiple-tool dispatcher. Application code must define execution policy and correlate every applicable result.

The [request implementation](https://docs.rs/crate/genai/0.6.5/source/src/chat/chat_request.rs) derives Serde for messages, tools, `previous_response_id` and `store`; provider response storage is explicit opt-in. `append_tool_use_from_stream_end` prefers captured content to preserve thoughts/text/call order, with a calls-only fallback. Persisting only visible text is insufficient for all continuations.

**Adapt:** use a provider adapter behind a project-owned model boundary; preserve explicit dispatch, loop limits and ordered multimodal history. A serialized request is not a serialized executor/checkpoint. No genai mock harness was inspected.

## ADK-Rust: richer contracts and serialization traps

This is [zavora-ai ADK-Rust](https://crates.io/api/v1/crates/adk-rust); its name establishes neither Google ownership nor cross-language parity.

The actual [model source](https://docs.rs/adk-core/2.2.0/src/adk_core/model.rs.html#9-68) defines `Llm: Send + Sync`, async `generate_content`, a pinned boxed result stream and provider schema adaptation.

**Persistence trap:** `LlmRequest` derives Serde but `tools` has `#[serde(skip)]`. Restore must rehydrate tools from trusted configuration or a separate versioned descriptor. JSON round-trip success alone is not request equivalence.

The [core module](https://docs.rs/crate/adk-core/2.2.0/source/src/lib.rs) documents `Arc<dyn InvocationContext>` and `Arc<dyn ToolContext>` agent/tool boundaries. **Adapt:** shared immutable context and typed interfaces, not serialization of runtime objects. Introductory prose still says “What's New in 0.6.0” in package 2.2.0; implementation declarations are stronger evidence.

The [event implementation](https://docs.rs/adk_core/2.2.0/src/adk_core/event.rs.html#26-100) flattens `LlmResponse`, renames event provider metadata `event_metadata`, and serializes state deltas, artifact versions, transfers and confirmations. This establishes representation, not transactional persistence or restart recovery.

The [final-response predicate](https://docs.rs/adk_core/2.2.0/src/adk_core/event.rs.html#362-385) is true for skip-summarization or long-running-tool IDs, otherwise checks function calls/results and partial output. It allows multiple final responses from participating agents. **Reject direct mapping** to whole-application completion.

## Provisional direction and rejected alternatives

Preserve application-owned behavior and versioned persistence DTOs; evaluate adapters behind that boundary. Rig offers the strongest directly inspected offline test primitives here; genai demonstrates application-controlled continuation; ADK offers richer context/events with semantics requiring validation.

Reject:

- Whole-framework substitution justified by matching names/types.
- Treating genai tool declarations as automatic execution.
- Treating Serde or in-memory history as durable resumability.
- Applying pre-split Rig examples to 0.42.0.
- Selecting genai 0.7 beta from 0.6.5 evidence.
- Claiming uninspected alternatives are unmaintained.

## Behavioral versus structural compatibility

Structural compatibility means roles, content parts, arguments, IDs and state can be represented. Behavioral compatibility requires compatible ordering, stopping, errors, approvals, side effects and recovery for identical inputs.

| Surface | Required behavioral proof |
|---|---|
| Models | Captured request equivalence, partial/final ordering, cancellation and error classification |
| Tools | Unknown/invalid arguments, multiple calls, correlation, authorization-before-execution, retry side effects |
| State | Round trips including absent/defaulted fields, tool registry rehydration, no repeated completed effects on resume |
| Completion | Text-only response, tool continuation, iteration exhaustion, long-running and multi-agent completion |
| Testing | Network-free scripts, unexpected extra calls fail, live checks separate |

Use project-owned versioned envelopes retaining ordered messages/events, correlation and pending/completed operations. Reconstruct clients/tools instead of persisting trait objects. These are proposed choices, not upstream guarantees; existing external formats remain authoritative until an approved migration.

## Unverified before dependency selection

MSRV, companion runtime versions, dependency lock compatibility, backend crash consistency, cancellation, authorization and complete provider schema transformations need targeted investigation. No framework's full runtime has been proven behaviorally equivalent. License metadata and repository links are recorded below; their inclusion is not legal approval for adoption.

### Independently checked registry metadata

Exact-version crates.io API responses were inspected in addition to the sources above. All three versions were unyanked.

| Crate | Repository | Declared license | Archive SHA-256 | Declared Rust version |
|---|---|---|---|---|
| rig-core 0.42.0 | https://github.com/0xPlaygrounds/rig | MIT | `432d83e0facf16749f91fe729cbffca84437e8062d2f4e92f4f12e903693922d` | Not set |
| genai 0.6.5 | https://github.com/jeremychone/rust-genai | MIT OR Apache-2.0 | `1d12aba7e9dc2c4d54654566dc3dc8383b5cb52e0cfc5754989afe0480d933e3` | Not set |
| adk-core 2.2.0 | https://github.com/zavora-ai/adk-rust | Apache-2.0 | `040e96a250780592a2d9fb2e1813bcfa035d191494c383675b785ad36d889388` | 1.95 |

Checksums are registry-reported archive identities, not a claim of locally compiled source. All three declare edition 2024. Query: `https://crates.io/api/v1/crates/<crate>/<version>`. Full dependency license compatibility remains unverified.
