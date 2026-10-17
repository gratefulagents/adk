# Guardrails and ordered hooks

A host can supply typed asynchronous `runtime::Guardrail` implementations.
`Builder::input_guardrails` and `output_guardrails` append agent checks;
`RunnerConfig::tool_input_guardrails` and `tool_output_guardrails` apply to tools.
Callbacks run in order, with cancellation/deadline boundaries and panic isolation.
`None` means an empty successful result. A tripwire stops subsequent checks;
input/output tripwires fail the invocation, while tool tripwires become
model-visible tool errors. Callback failures are fatal, not tripwires.

`GuardrailResult::output` is diagnostic data, never an implicit replacement.
`replacement_content: Some(text)` is the Rust representation of the pinned SDK's
`ContentReplaced`/`ReplacementContent`, including an explicit empty replacement.
Only a non-tripped tool-output check applies it, preserving tool flags and feeding
the replacement to later checks before raw-output hooks, traces or model caps.
Reports remain in successful and partial `RunResult::guardrails` snapshots.

Tripwire messages identify the input/output phase and, for tool checks, the tool
name. Diagnostic `output` remains in the report and underlying typed cause; it is
not appended to serialized `ErrorInfo` or model-visible error text, because it
may contain secrets. This differs from the Go error formatter's diagnostic suffix
and is not a claim of unrestricted error-text equivalence. Host configuration
compilation failures retain their error category and cause with explicit
`compile guardrail rules` context, before any model dispatch.

Builtin destructive-command checks inspect `ToolCall::argument_text()`, so an
original malformed input cannot be hidden by a null parsed projection. Shell
keys are case-insensitive; duplicate string values replace earlier values, while
null leaves them unchanged and any wrong-typed field fails closed. Value-only
calls retain their canonical fallback. Unknown fields are ignored subject to the
native JSON depth limit; deeper input fails closed rather than claiming Go's
entire parsing domain. The destructive guard's durable key is version 2, rejecting
checkpoints bound to the earlier argument-projection behavior.

Durable runs require a nonempty `Guardrail::durable_key` for deterministic checks.
Ordered names/keys participate in the checkpoint policy fingerprint; changing
one rejects recovery. Completed recovery retains reports without re-running
callbacks. Guardrails must not detach work or perform replay-unsafe effects.

`runtime::CompositeHooks` awaits each observer in order even after a previous
observer returns an error. `HookErrors` retains every cause. This is distinct
from `compat::GoCallbackAdapter`'s infallible lifecycle observers. A composite is
replay-safe only when every constituent observer explicitly opts in.

Reference: SDK v0.0.115, revision
`1dc92b73900fac74dc357a938e4b5eee6392b418`, `internal/agent/guardrail.go`,
`internal/agent/runner.go`, and `pkg/agentsdk/composite_hooks.go`; GPL-3.0-only.
Implicit string-output replacement is deliberately not adopted. Tests distinguish
diagnostics, explicit empty replacement, and tripwire precedence using the
authoritative pinned contract.
