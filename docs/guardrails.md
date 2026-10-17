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

With the `builder` feature, `guardrails::match_tool_pattern` exposes the SDK
public helper's edge-star syntax. A trailing `*` selects a literal prefix before
a leading `*` can select a literal suffix. Internal stars are literal: `*read*`
does not mean a substring search. `*` alone matches every name, including empty.
This is deliberately separate from host rule compilation's multi-star glob.
With `host`, `guardrails::make_tool_input_guardrail` and
`make_tool_output_guardrail` take an owned `host::GuardrailRule` and caller-compiled
`regex::Regex`. They use that regex rather than the configuration's regex string,
preserve message whitespace, and block for every action except exact `warn` and
`log`. Names use `crd:` rather than ChatLoop's `config:` prefix. Default diagnostic
names use SDK quoting. Input checks inspect original argument text; output checks
match newline-joined text parts, ignoring non-text parts. A guard ignores the
opposite phase. They do not supply durable identities or claim Go RE2 syntax;
callers explicitly own Rust regex compilation.

`guardrails::tool_guardrails_from_rules` (with the `host` feature) compiles the shared Go/Rust
regex subset and returns `(CompiledGuardrails, Vec<Error>)`. It retains valid
rules in source order within each phase while collecting rejected-rule errors in
source order. Callers must handle errors before using a partial rule set. Regex
validation precedes action and type validation. Actions are Unicode-lowercased
and trimmed; empty means `block`. This public compiler does not change the host
compiler's fail-fast contract or its different glob/message semantics. Both
compilers accept Go's Unicode action case normalization (for example, the Kelvin
sign in `blocK` lowercases to `block`). RE2-only
syntax outside the supported subset remains explicitly unsupported, and native
regex-parser prose is not a Go byte-parity claim. The shared subset accepts the
14 Go POSIX ASCII class names and their negations, including unions and leading
literal `]` cases. This does not enable arbitrary nested classes or Rust class-set
operations. Counted repetitions enforce Go's 1,000-copy bound, including nested
products; zero maximums shield inner products but do not permit independently
invalid inner counts. Malformed count forms, including leading-zero numbers,
remain literal text as in Go rather than acquiring Rust repetition semantics.
In-class Perl escapes remain outside the supported subset.

Builtin destructive-command checks inspect `ToolCall::argument_text()`, so an
original malformed input cannot be hidden by a null parsed projection. Shell
keys are case-insensitive; duplicate string values replace earlier values, while
null leaves them unchanged and any wrong-typed field fails closed. Value-only
calls retain their canonical fallback. Unknown fields are ignored subject to the
native JSON depth limit; deeper input fails closed rather than claiming Go's
entire parsing domain. The destructive guard's durable key is version 2, rejecting
checkpoints bound to the earlier argument-projection behavior. The builtin secret
input guard also scans original argument text before its additional decoded-value
inspection; malformed or opaque text cannot hide credentials behind a null
projection. Its durable key is version 2 as well. Diagnostic reports do not carry
the matched credential.

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
