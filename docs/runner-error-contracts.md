# Runner error and callback contracts

This is an enumerated behavior contract, not a claim that native Rust snapshots
are Go wire values or replay checkpoints. Regression coverage lives in
[`error_contracts.rs`](../crates/adk-runtime/tests/error_contracts.rs); existing
runner/lifecycle tests cover additional approval, batching and spill lifetimes.

## Reference boundary

Go references below are pinned to SDK commit
`1dc92b73900fac74dc357a938e4b5eee6392b418`:

- **G** = `repos/sdk/internal/agent/runner.go`.
- **C** = `repos/sdk/pkg/agentsdk/chatloop.go`.

Line ranges refer to that revision, not the current native implementation.
A direct Go runner `(result, error)`, the public Go chat-loop return, and native
`RunError.partial` are three distinct interfaces. In particular, C191–194 returns
`nil, err` when `Runner.Run` fails, discarding its result slot. A typed max-turns
error can still carry its own partial result; that does not mean C preserves the
ordinary result slot for provider errors. By contrast C240–268 explicitly combines
previous `NewItems` with approval-resolution items before returning gate errors.

## Enumerated contracts

| ID | Boundary | Required behavior / comparison | Focused regression |
| --- | --- | --- | --- |
| E01 | Configuration and input validation | Invalid runner configuration fails construction. Invalid initial history fails before model/tool dispatch or `Started`. Native input diagnostics may still be attached to the latter; there is no accepted response. | `invalid_configuration_and_input_never_dispatch` |
| E02 | Provider error before any output | Preserve the original provider error. Native partial contains initial input, no new items/responses/usage. This is diagnostic enrichment: G594–621 normally leaves the result nil when no items accumulated and the parent context is still active. Cancellation is a distinct exception in that Go deferred-return path. | `provider_failure_keeps_input_only_and_failed_sink_cannot_replace_error` |
| E03 | Delta then provider error | Native diagnostics retain observed assistant text but do not synthesize an accepted response or usage callback. No retry/fallback after a visible event. G1950–1993 forwards deltas and returns a stream error; these raw deltas are not a completed Go turn. | `delta_or_complete_then_error_is_diagnostic_not_model_acceptance` |
| E04 | `Complete` then provider error | Raw `ModelEvent::Complete` is still delivered. Native diagnostic history, response and reported usage survive, but `ModelAccepted` and the successful usage callback do not fire. G1950–1993 retains the candidate response only for successful stream completion; a subsequent stream error is still failure. Native retained data is not Go partial-result parity. | Same as E03 |
| E05 | Successful model acceptance | Streaming acceptance requires successful EOF after `Complete`; complete-only calls require successful return. `Observation::ModelAccepted { agent, response }` occurs before recording the accepted response, before `Observation::Usage`, and before tool execution. Go `OnLLMEnd` similarly precedes usage accumulation (G1276–1280). Raw `Complete` is not a substitute for `OnLLMEnd`. | `accepted_response_precedes_usage_and_tool_start_only_after_successful_eof` |
| E06 | Observation versus security hooks | Go void observation callbacks are panic-recovered (G2839–2851); this is not a license to swallow a fallible security-hook error. Native `RunHooks` are awaited and fail-closed. A failed post-tool hook retains a paired, withheld error result, aborts the run, and emits no successful `ToolFinished`. Go's explicit `ToolEndErrorHook` is likewise fatal (G2476–2481), separately from void `OnToolEnd`. Native fallible hooks are not the Go void-callback ABI. | `post_tool_security_hook_is_fatal_and_suppresses_successful_finish` |
| E07 | Failure reporting sink | `Failed` delivery is best effort. If its host sink fails, `RunError.error` remains the original cause rather than the reporting error. Cancellation/deadline may prevent delivery entirely. | Same as E02 |
| E08 | Ordinary tool error and local tool timeout | Produce a paired error tool result and continue the model loop. Do not automatically retry the tool. A parent deadline/cancellation remains run-fatal, rather than a recoverable local tool failure. | `ordinary_failure_timeout_unknown_and_inaccessible_tools_pair_and_continue`; E11 |
| E09 | Unknown/inaccessible tool | Treat unavailable names as unknown-tool error results and continue. No execution and no `ToolStarted`; Go resolves unknown tools synchronously before async execution (G2226–2243). Explicit security/approval-hook failures remain separate fatal boundaries. | Same as E08 |
| E10 | Retry, fallback and turn budget | Every provider attempt consumes a turn, including retry, fallback and error-handler `Retry`/`Continue` attempts (Go loop G624 onward). With max turns = 1, a failed first attempt cannot dispatch a second provider call; the terminal category is `MaxTurns` when another attempt is requested. | `retry_fallback_and_error_handler_attempts_all_consume_turns` |
| E11 | Timeout origin and ownership | A pre-output model-local idle timeout is eligible for configured retry/fallback. Parent cancellation/deadline is not, and neither is an idle timeout after visible output. Dropping a pull stream drops pending provider/tool work. Native tools/providers must not detach work; cancellation cannot undo effects already performed. | `local_idle_before_output_can_retry_but_visible_output_and_parent_deadline_cannot`; `parent_cancellation_and_owner_drop_close_pending_model_and_tool_resources` |
| E12 | Native token/cost limits | Zero blocks dispatch. Equality exhausts the budget, including after accepted usage and before tools. Usage is retained even when it trips the limit. Cost limiting requires an estimator. These optional, broader run-wide counters have no equivalent Go run-wide counter in this reference: they are explicitly configured native guardrails, **not default behavior departures**. Provider generation caps remain a different boundary. | `zero_and_equal_token_or_cost_limits_stop_before_tool_effects` |

## Partial results are not replay authority

Native failure snapshots intentionally preserve more diagnostics than the Go
result-return contract: input-only failures, visible-but-unaccepted streamed
items, and a `Complete` response followed by an error can all appear. They have
`Incomplete` status and no final output. An accepted response hitting a budget
before tools can leave tool calls without results. Other security failures may
replace sensitive outputs with paired withheld results. Neither case proves
that replaying history is safe or that external side effects did not occur.

G594–621 instead describes its deferred partial as completed, folded,
replay-safe history; C191–194 can then discard that ordinary return slot. Do not
serialize native `RunError.partial` and label it a Go partial, assume a raw model
completion means `OnLLMEnd` occurred, or reconstruct an approval continuation
from it. Native continuations own live cursor/budget state; Go chat-loop resumes
are fresh runner invocations. Wire conversion, chat-loop result adaptation and
durable replay must establish their own contracts explicitly.
