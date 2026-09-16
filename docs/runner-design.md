# Standalone runner design (issue #4)

The optional `adk-runtime` engine implements existing `adk-core` model, tool and
host contracts; it does not import the platform. Provider adapters, durable
checkpoints and managed subagents must enter through these boundaries rather
than fork the loop. This document describes implementation choices and bounded
verification, not production/provider parity.

## Rust research

Inspected versioned sources, not unversioned example snippets:

* **Rig v0.42.0**, [release, 2026-08-17](https://github.com/0xPlaygrounds/rig/releases/tag/v0.42.0),
  [runner](https://github.com/0xPlaygrounds/rig/blob/v0.42.0/crates/rig-agent/src/agent/runner.rs),
  [completion preparation](https://github.com/0xPlaygrounds/rig/blob/v0.42.0/crates/rig-agent/src/agent/completion.rs).
  Its sans-I/O `AgentRun` and effectful `AgentRunner` share normal/streamed
  execution. Adopt a single explicit state machine and advertisement/dispatch
  consistency, not the Go goroutine topology. Reject blindly copying its turn
  accounting (zero means no calls; retries consume turns), concurrent hook
  interleaving, or synthetic structured-output tool defaults. Those change
  observable contracts. The tagged runner is in `rig-agent`, not the historical
  `rig-core/src/agent` location.
* **Swiftide v0.32.1**, [release, 2025-11-15](https://github.com/bosun-ai/swiftide/releases/tag/v0.32.1),
  [agent implementation](https://github.com/bosun-ai/swiftide/blob/v0.32.1/swiftide-agents/src/agent.rs).
  [Repository metadata](https://api.github.com/repos/bosun-ai/swiftide) checked
  during implementation reported not archived and a 2026-09-16 push. The older
  release is a pinned reference, not a claim to describe all current development.
  Adopt separated lifecycle hooks/context interfaces and explicit stream deltas;
  reject its default stop tool/system prompt, duplicate-tool replacement, and
  per-name/arguments retry accounting as implicit compatibility changes.

Neither framework is a dependency. A runtime-specific driver around the existing
object-safe, borrowing core traits is smaller than adapting another ADK's message,
policy and provider abstractions. JSON Schema validation is delegated to
`jsonschema` 0.33 with network/file resolution features disabled, rather than
implementing an incomplete validator. Its transitive `borrow-or-share` dependency
uses MIT-0 (the permissive MIT no-attribution variant), explicitly accepted in
`deny.toml`; network/file resolver features remain disabled. Tool and host implementations are trusted
code; access policy is not an OS sandbox.

## Ownership and behavior

One engine drives both complete and genuinely streaming model capabilities.
Tool effects are sequential and ordered. Approval suspension owns the exact
unresolved call and remaining queue; resumption consumes the continuation, not a
reconstructed `input + new_items` transcript. This prevents completed effects
being repeated by the continuation API, **not exactly-once execution after a
process crash**. Durable recovery/idempotency protocols remain separate issues.

Awaited event delivery provides backpressure. Cancellation and total deadlines
must race pending I/O; checking a flag only before dispatch is insufficient.
Provider idle timeouts reset per received event rather than acting as total
stream deadlines. Limits stop at reported usage boundaries, not mid-generation;
provider compaction reports usage and cost too, and is charged before another
model call. A cost-limited run requires a cost estimator. Retries re-enter the
primary model each turn rather than maintaining Go's sticky fallback/reprobe
counter. No tool effect is automatically retried, and cancellation cannot undo an
already-completed external effect. Raw tool hooks observe
output before model-facing wrapping/capping; a trusted observer must handle that
sensitive data accordingly. Owned spill files outlive the model's use of them and
are removed when their owner drops. Read-only policy never creates spills.
Output protection transforms `ToolOutput`, not `ToolDefinition`: executable
wrappers cannot acquire approval or read-only privileges from a changed output
wrapper. The runner rechecks the original registered definition and preserves the
host's `ToolPolicy` in `ToolContext`, including allowlisted mutating tools.

Instructions/tool definitions form a stable prefix. `prompt_cache_key` is hashed
with `prompt_cache_namespace` (or the run ID if absent); set a stable explicit
namespace to reuse cache keys across separate invocations of one conversation.
This isolates logical keys without putting user history into the cache key.
Transient context belongs to the current request, not persisted history. Effective history may be replaced by
explicit compaction while observable new items stay append-only. Hosts must
resume from effective history, not append old input to new items. Compaction
hooks must expose transformation boundaries to persistence/observability adapters.

## Go source and regression obligations

SDK baseline: [`1dc92b73900fac74dc357a938e4b5eee6392b418`](https://github.com/gratefulagents/sdk/tree/1dc92b73900fac74dc357a938e4b5eee6392b418).
Relevant references: [runner](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/runner.go),
[configuration](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/run_config.go),
[runner tests](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/runner_test.go),
[ChatLoop](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/chatloop.go).

| Behavior family | Observable regression obligation |
|---|---|
| Text/tool loop | Dispatch order and IDs, paired call/results, usage, final output and effective history |
| Approval | No repeated completed effect; exact-call decisions; denial cannot bypass access policy |
| Limits/errors | Incomplete partial results preserve committed history and usage; turn limit has no final output |
| Retry/fallback | Only safe model calls retry; visible stream output must not replay; delay/cancellation bounded |
| Idle/deadline | Slow active stream survives idle limit; stalled stream and total deadline terminate distinctly |
| Cap/trust/spill | Default 16 KiB, UTF-8 boundaries, private files only in writable mode, drop cleanup, raw hook fidelity |
| Compaction/cache | Stable prefix/key; transient context absent from history; observable effective-history replacement |
| Schema | Invalid schema/config and invalid structured output classified explicitly; response retained |
| Streaming | Ordered deltas/terminal events, bounded producer advance, dropped consumer releases owned work |
| Handoff | Target dispatch and last agent, tool queue ordering, turn accounting |

Go defaults nonpositive max turns to 100; native core instead requires an explicit
nonzero turn budget. Do not invent a Go-style sentinel in the Rust policy. Go's
ChatLoop initial error branch discards the runner partial result; retaining usable
partial results is an intentional Rust improvement. The Go no-gate denial branch
also has history/interruption-state ambiguities; owned continuations avoid copying
that control flow. Go provider retry advice, default compaction heuristics and
fallback reprobe timing are not automatically supplied by neutral core traits.
Rust rejects schema-invalid final output with a typed error and retained response,
rather than Go's warning-and-raw-string fallback. Tool-provided trust delimiters
are escaped instead of being accepted as evidence that output is already wrapped.
These are deliberate fail-closed behavior changes. Native tool execution is
sequential; unlike Go's concurrent batch dispatch, effect completion and hook
ordering are deterministic. This is not a timing/concurrency parity claim.

Cross-language replay must use the real Go runner and the Rust engine, normalize
only declared representation differences, retain event order/call correlations,
and separately report Rust-only regression tests. It is not enough to replay the
older codec fixtures or have a fake model emit the expected transcript. Live
providers, platform persistence and crash recovery are outside this change.

## Verification map

The public example is `cargo run -p adk --features runtime --example runner`.
It uses only a scripted local model and a console host, and asserts equal normal
and streamed final results.

| Acceptance family | Executable evidence |
|---|---|
| Real Go/Rust replay | `tests/replay.rs`: eight real-engine scenarios plus argument/ID/delta mutation checks; [normalization and provenance](../scripts/replay/runner_README.md) |
| Approval/stop/pause | `tests/runner.rs`: `approval_resume_keeps_cursor_and_completed_effects`, `tool_pause_resumes_next_turn_and_stop_executes_batch` |
| Stream backpressure/drop | `stream_is_lazy_bounded_and_drop_drops_provider`, `cancelled_next_future_is_safe_and_owner_drop_cleans_pending_stream`, `invalid_stream_protocol_is_not_success` |
| Retry/limits/timeouts | `retries_then_fallback_without_spending_extra_turns`, `turn_token_and_cost_limits_keep_partial_usage`, `cancellation_deadline_idle_and_tool_timeout_interrupt_pending_work` |
| Compaction/cache/context | `compaction_replaces_history_and_hints_cache_prefix_are_request_only` |
| Policy/schema/handoff | `authorization_and_argument_validation_precede_effects`, `duplicate_names_and_invalid_schema_are_rejected`, `structured_output_is_schema_validated_not_just_json_parsed`, `handoff_preempts_siblings_and_pairs_all_calls` |
| Hooks/spill lifetimes | `raw_hooks_precede_processing_and_spills_live_across_pause_and_error`, `durable_failure_before_effect_fails_closed_after_effect_preserves_result`, 12 `output.rs` unit tests |
| Independent review regressions | `tests/review_regressions.rs`: failed resume adoption; trailing partial text after reasoning/completed messages; no duplicated committed deltas; last nonempty final answer validated in both execution modes |

Tests are under `crates/adk-runtime/`. The replay corpus deliberately covers a
bounded common observable subset; Rust-only lifecycle/policy regressions are not
presented as independently verified Go parity. No rollout/canary or live-provider
work is part of this change.
