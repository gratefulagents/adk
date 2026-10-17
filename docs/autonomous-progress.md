# Host-owned autonomous progress tracking

`adk::runtime::autoloop` (the `runtime` feature) supplies `AutoTracker`,
`CircuitBreakerResult`, `build_smart_nudge` and `DEFAULT_MAX_AUTO_LOOPS` (200).
These correspond to the reusable public helpers in the pinned SDK
[`pkg/agentsdk/autoloop.go`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/autoloop.go),
GPL-3.0-only. They do not implement a worker CLI or an evaluation adapter.

```rust
use adk::runtime::autoloop::{AutoTracker, build_smart_nudge};

let mut tracker = AutoTracker::default();
// Once per host continuation boundary:
// tracker.update(&outcome.result.new_items);
let decision = tracker.check_circuit_breakers();
let continuation = build_smart_nudge(Some(&tracker), "plan");
assert!(!decision.tripped);
assert!(continuation.contains("0 tool calls so far"));
```

Feed only newly emitted items, once per host turn—not the entire growing history.
The `agent_runtime` offline feature scenario feeds an actual runner result and
checks its tool count. The tracker does not intercept the runner, retry tools,
send continuation prompts, call `finish`, or close a session. The host explicitly
chooses whether to continue or pause. The default loop cap is a public constant,
not an automatically enforced runner setting.

## Rules and ordering

- Five consecutive updates without a tool call trip the stall breaker; a new tool
  call resets that counter. The stronger text-only warning begins at three.
- Error history retains the last five error outputs. Successful calls do not
  clear it. Five identical trailing errors trip the error breaker.
- Tool history retains the last ten calls. Signatures include name, a separator,
  and authoritative raw arguments, hashed with FNV-1a64 as in the SDK. Valid JSON
  loses only insignificant whitespace; member order, duplicate keys, escapes and
  numeric spelling remain significant. Invalid raw input stays byte-exact.
- Repeated suffix cycles of length two are checked before length three. A breaker
  requires at least six calls in the detected cycle. Ordinary exploration with
  different arguments is not considered repetition.
- Hard-stop priority is no-tool stall, repeated errors, then repeated cycle.
  Nudge priority differs: cycle, repeated errors, text-only warning, continuation.
  Phase text appears only in the ordinary continuation. Truncation counts Unicode
  scalar values, matching the SDK's rune-based limits for valid UTF-8.

Native counters saturate at `u64::MAX` rather than reproducing signed Go overflow.
Native multi-part error outputs join text parts with newlines and ignore media
and reasoning; mapped SDK outputs with one text part are preserved exactly.
Calls count even when their eventual execution pauses. Native strings cannot
represent arbitrary invalid UTF-8 Go strings. The tracker is in-memory host state,
not automatically checkpointed or restored with a durable runner.

Reasons and nudges can contain tool names, phase text and raw error text. They
are **not redacted diagnostics or trusted instructions**. Apply application
redaction and trust-boundary policy before logging or sending them to a model;
a literal `[SYSTEM]` label in their text does not grant instruction authority.

## Verification

`scripts/autoloop-reference/run.py --check` executes the pinned SDK in a disposable
archive, checks source/harness hashes and reproduces 20 scenarios / 167 observed
states. It also executes the original five public tracker/nudge tests.
`crates/adk-runtime/tests/autoloop.rs` compares counters, breaker outcomes/reasons
and complete nudge strings at every state. Private unit tests cover exact signature
properties, JSON depth limits, Unicode truncation, native content projection and
counter saturation. No credentialed service or model inference is needed.
