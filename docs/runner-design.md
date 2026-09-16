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
  accounting (zero means no calls; retries consume turns) or synthetic
  structured-output tool defaults. Tool scheduling follows the Go read/write
  exclusion contract, rather than adopting another framework’s defaults. Those change
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
Ordinary preauthorized tool batches run as owned concurrent futures. Read-only
tools share a Tokio read lock; mutations take its exclusive write lock. Results
fold in call order, while starts and raw-output hooks may interleave. Approval suspension owns the exact
unresolved calls and remaining queue; resumption consumes the continuation, not a
reconstructed `input + new_items` transcript. This prevents completed effects
being repeated by the continuation API, **not exactly-once execution after a
process crash**. Durable recovery/idempotency protocols remain separate issues.

Awaited event delivery provides backpressure. Cancellation and total deadlines
must race pending I/O; checking a flag only before dispatch is insufficient.
Provider idle timeouts reset per received event rather than acting as total
stream deadlines. Limits stop at reported usage boundaries, not mid-generation;
provider compaction reports usage and cost too, and is charged before another
model call. A cost-limited run requires a cost estimator. Fallback selection and success counts are per agent identity, live inside the
continuation, and reset to primary after three successful fallback calls. A new
fallback resets its count; a failed primary reprobe can select fallback again.
Configured fallback is selected before policy retries, matching Go's precedence. No tool effect is automatically retried, and cancellation cannot undo an
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

### Delivery-scope audit (PR18 correction)

Documentation does not approve external differences. **Issue #4 is not scope
complete.** The following is an enumerated audit, not a parity waiver.

1. **Sticky fallback/reprobe restored.** Pinned `runner.go:517–522, 715–717,
   1114–1115, 1224–1230` stores fallback per agent, counts successful calls and
   clears it at three. Rust retains the same state through approval continuation,
   selects subsequent fallback on failure, resets the count on switching, and
   reprobes primary. Replay includes recovery, failed reprobe and chain switching,
   in regular and streaming modes. Rust tests also cover continuation retention,
   distinct agents with the same display name, and fallback-before-retry priority.
2. **Default structured-output outcomes restored.** `runner.go:2137–2146` warns
   and keeps raw text when `OutputType.Validate` fails. Crucially,
   `output_schema.go:Validate` defaults to JSON parsing, not JSON Schema validation:
   schema-invalid but parseable JSON is returned parsed. Rust now matches both
   outcomes, including schema prompt instructions, and exposes validation problems
   as `OutputValidationFailed` observations instead of aborting by default.
   A host may deliberately fail its hook (an explicit host policy, not default).
   Replay compares non-JSON, schema-invalid JSON, and valid JSON in both modes.
   Custom `OutputParser`, schema name and strict-mode configuration are now
   exposed explicitly. Parser failures preserve raw text, parser successes may
   return transformed values, matching Go ParseFn. Four extra Go/Rust cases
   verify custom rejection/transformation and named non-strict schema prompts.
   Name/strict settings also reach the provider-neutral ModelRequest.
3. **Ordinary tool concurrency and deferred-approval sibling scheduling restored.**
   `runner.go:2288–2365` uses concurrent read-safe tools/exclusive mutations and
   ordered result slots. Rust uses `join_all` plus a Tokio `RwLock`, no spawned
   producer/tasks. Barrier tests prove read fan-out (sequential execution would
   deadlock), exclusive mutations, ordered result folding, and consumer-drop
   cleanup in both execution modes. No exact concurrent start/hook order is
   promised by either baseline; results retain input order. Go's managed-subagent
   parallel-safe marker/semaphore remains in its dedicated integration issue.

   Go partitions approvals before executing eligible sibling tools and returns a
   batch (`runner.go:2229–2285`). Rust now does this too: all deferred calls are
   returned in `pending_approvals`, and eligible siblings execute before pause.
   `Continuation::{resume_batch,stream_batch}` accepts exact call-ID decisions;
   missing/duplicate/unknown IDs fail before effects. The single-decision methods
   remain for one pending call or a tool pause. Completed effects are removed from
   the queue; re-deferral and resume cannot replay them. Approved calls resolve in
   call order. Two additional actual Go/Rust scenarios verify eligible dispatch,
   model-facing transcript, usage, and ordered pending IDs in both modes. Rust
   tests cover repeated pause/resume, reversed decision input, and invalid IDs.
   Two further actual Go ChatLoop/Rust continuation replays approve one or two
   pending calls after an eligible sibling has completed. They compare exact
   dispatch, hooks, requests, history and final/usage outcomes through resume;
   repeating that sibling would fail the comparison. These resume cases use the
   normal execution path; denial/re-deferral and streamed resume remain Rust-only
   evidence.

   The existing #2 representation deliberately puts approvals in a side channel
   (`pending_approvals` / `ApprovalRequired`) rather than a `RunItem` variant.
   Replay explicitly extracts Go approval markers into the same side channel;
   it does not claim raw approval-history/event-envelope identity or identical
   approval-event timing. Core documents native Serde as distinct from Go wire
   compatibility (`adk-core/src/lib.rs`); a full adapter must preserve the marker
   when reconstructing the Go wire representation, not silently discard it.
4. **Output formatting/history restored; partial/default mappings audited.**
   Rust now matches Go's delimiter idempotence, including preserving text that
   contains the opening marker. This text-format limitation does not authorize
   execution: registered definitions and ToolPolicy checks remain unchanged.
   NewItems/tool-result events retain capped raw text, while effective model
   history is wrapped, matching runner.go:1579–1588. Hooks retain raw text before
   capping. Ten real-engine replay cases cover ordinary/marker-bearing/already
   wrapped text, Unicode truncation and caps smaller than the wrapper, in both
   modes. Like Go, tiny caps preserve intact delimiters and empty the body.

   Four real-engine cases map Go's zero/negative turn sentinels to the public
   Rust RunPolicy default of 100, comparing execution rather than Serde shapes.
   This verifies the mapping, not acceptance of Go sentinel integers by native
   NonZeroU32 deserialization. A Go-compatible configuration adapter must perform
   that mapping; no such adapter is claimed here.

   The earlier assertion that ChatLoop discards all partials was too broad:
   its nil result slot on MaxTurnsExceeded is accompanied by a typed error with
   PartialResult. Two actual ChatLoop/Conversation cases verify normal completion
   and turn-limit partials, extracting that error's existing partial, never
   reconstructing history. Other infrastructure-error partial enrichment remains
   a native capability required by #4's usable typed partial errors, not evidence
   of equivalence for every Go ChatLoop error path.
5. **Provider retry advice and precedence implemented.** `Model::retry_advice`
   carries should-retry, retry-after and reason. Eligible overload/quota/rate-limit
   advice selects fallback before the host ModelErrorHandler; then policy retries
   are considered unless advice explicitly rejects them; advised retries are
   bounded at ten failures per turn. Delays honor retry-after and are capped at
   five minutes, with nonzero backoff for advice without a delay. Visible stream
   output, cancellation and deadlines still prohibit replay. Virtual-time tests
   cover precedence, explicit rejection, missing reason, ten-retry bounds and the
   five-minute cap without sleeping in real time. Fallback/schema cross-language
   replay remains separate from these Rust-only advice regressions. All 44 replay
   scenarios now also compare lifecycle callback order/content, including
   per-attempt agent/model start, model-end items/usage, raw tool callbacks and
   final agent output. This is a declared native-to-Go projection, not raw
   hook/trace payload equivalence. Default compaction-heuristic equivalence is
   still not claimed.

Concrete delivery boundary: fallback/reprobe and default schema outcomes are
corrected and verified; tool scheduling including deferred-approval siblings is corrected with owned
Rust futures. Output text/history is now baseline-compatible in the enumerated
cases. Remaining delivery decisions concern native approval side channels versus
Go wire markers/timing, native configuration versus Go sentinel ingestion, and
infrastructure-error partial enrichment beyond the proven turn-limit case. Raw
trace and default compaction-heuristic equivalence are not established. Do not
close #4 on the basis of this bounded correction. If full Go-facing ingestion and
wire identity are required in #4, those adapters need implementation and replay;
otherwise their explicit ownership/acceptance belongs in the maintainer's scope
decision under #1, not an implicit waiver in this note.

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
| Real Go/Rust replay | `tests/replay.rs`: 44 real-engine scenarios plus argument/ID/delta mutation checks; [normalization and provenance](../scripts/replay/runner_README.md) |
| Approval/stop/pause | `tests/runner.rs`: `approval_resume_keeps_cursor_and_completed_effects`, `batch_approvals_run_eligible_siblings_and_resume_only_unresolved_call_ids`, `batch_approval_decisions_reject_missing_duplicate_and_unknown_ids_before_effects`, `tool_pause_resumes_next_turn_and_stop_executes_batch` |
| Concurrent tool batches | `tool_batches_fan_out_reads_exclude_mutations_and_fold_in_call_order`, `dropping_stream_drops_all_inflight_batch_tools` (Rust regressions, not Go scheduler replay) |
| Stream backpressure/drop | `stream_is_lazy_bounded_and_drop_drops_provider`, `cancelled_next_future_is_safe_and_owner_drop_cleans_pending_stream`, `invalid_stream_protocol_is_not_success` |
| Provider advice | `provider_advice_controls_policy_retries_and_caps_delay_at_five_minutes`, `provider_advised_retries_are_bounded_without_spending_model_turns`, `fallback_precedes_error_handler_which_precedes_advice_retry` |
| Retry/limits/timeouts | `fallback_precedes_policy_retries_without_spending_extra_turns`, `sticky_fallback_survives_approval_resume_and_reprobes_after_three_successes`, `fallback_state_is_per_agent_identity_not_display_name`, `turn_token_and_cost_limits_keep_partial_usage`, `cancellation_deadline_idle_and_tool_timeout_interrupt_pending_work` |
| Compaction/cache/context | `compaction_replaces_history_and_hints_cache_prefix_are_request_only` |
| Policy/schema/handoff | `authorization_and_argument_validation_precede_effects`, `duplicate_names_and_invalid_schema_are_rejected`, `structured_output_validation_preserves_baseline_result`, `handoff_preempts_siblings_and_pairs_all_calls` |
| Hooks/spill lifetimes | `raw_hooks_precede_processing_and_spills_live_across_pause_and_error`, `durable_failure_before_effect_fails_closed_after_effect_preserves_result`, 12 `output.rs` unit tests |
| Independent review regressions | `tests/review_regressions.rs`: failed resume adoption; trailing partial text after reasoning/completed messages; no duplicated committed deltas; last nonempty final answer validated in both execution modes |

Tests are under `crates/adk-runtime/`. The replay corpus deliberately covers a
bounded common observable subset; Rust-only lifecycle/policy regressions are not
presented as independently verified Go parity. No rollout/canary or live-provider
work is part of this change.
