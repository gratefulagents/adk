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
literal `]` cases. Ampersands, tildes and pipes in classes remain Go literals,
including repeated forms that Rust otherwise interprets as class-set operations.
An inner opening bracket that does not begin a complete POSIX atom is a literal,
as in Go, not Rust nested-class syntax. Unknown complete POSIX names are rejected.
Hyphens follow Go's literal/range-endpoint grammar rather than Rust set subtraction;
descending ranges remain invalid.
Counted repetitions enforce Go's 1,000-copy bound, including nested
products; zero maximums shield inner products but do not permit independently
invalid inner counts. Malformed count forms, including leading-zero numbers,
remain literal text as in Go rather than acquiring Rust repetition semantics.
Perl class atoms (`\d`, `\s`, `\w` and their complements) preserve ASCII Go
semantics inside and outside classes, including under case-insensitive flags.
In-class word-boundary escapes remain invalid, as in Go. Go octal escapes are
translated to Unicode hex atoms; single nonzero digits remain invalid
backreferences. Braces belonging to hex escapes or Unicode properties are not
interpreted as repetition delimiters. Named captures validate Go's ASCII
alphanumeric/underscore grammar, including digit-leading and duplicate names.
Since rule guards expose membership only, validated named groups compile as
noncapturing groups; no capture-extraction API is implied.

Unicode property names are restricted to the independently observed pinned-Go
catalog, including its category aliases and ASCII space/hyphen/underscore name
normalization. Rust-only binary properties, property-value assignments and script
abbreviations are rejected. A leading caret inside a property toggles the outer
`p`/`P` sign. `Cs`/`Surrogate` denotes an empty set for valid UTF-8 inputs, including
inside complemented classes. This is name/negation compatibility, not a claim
that Rust and Go use identical Unicode range or case-fold tables. The checked-in
catalog deliberately follows the reference toolchain's actual acceptance, not
all names theoretically present in the Unicode standard.

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
