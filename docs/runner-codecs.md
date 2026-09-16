# Runner baseline codecs

Baseline: Go SDK `1dc92b73900fac74dc357a938e4b5eee6392b418`, especially
`internal/agent/run_config.go`, `internal/agent/runner.go`, and
`pkg/agentsdk/chatloop.go`. Native `adk-core` types remain unchanged.

## Configuration integration

`adk_codec::config::RunConfigSentinels` is a **bounded scalar projection**, not
an entire Go RunConfig. Unknown JSON fields fail rather than disappear. Select
these fields explicitly when adapting a larger config. Go field names and signed
64-bit integers are used; JSON null scalars behave like Go zero values.

- `resolve() -> Result<EffectiveRunConfig, ConfigError>` returns checked native
  `NonZeroU32`, `usize`, `Option<Duration>`, and booleans.
- Retain the original DTO to serialize exact zero/negative/default field values.
  Resolution does not mutate it. This preserves values, not source JSON spelling
  or omitted-vs-explicit-zero fields.
- `from_effective(&EffectiveRunConfig)` produces canonical explicit wire values;
  resolving those produces the same effective configuration. Literal native zero
  caps/timeouts cannot be represented by Go's default sentinel and are rejected.

| Field | Zero | Negative | Positive |
|---|---|---|---|
| MaxTurns | 100 | 100 | explicit (checked u32) |
| SubAgentMaxTurns | 50 | 50 | explicit (checked u32) |
| MaxConcurrentSubAgents | unlimited | unlimited | explicit |
| ConsecutiveToolErrorLimit | 3 | disabled | explicit |
| StopGateMaxBlocks | 8 | 8 | explicit |
| MaxToolOutputBytes | 16384 | disabled | explicit bytes |
| ModelCallTimeout | 300 seconds | disabled | explicit **nanoseconds** |
| ToolPolicy.DefaultTimeout | no override | no override | explicit **seconds** |

`UntrustedToolOutputs: null` defaults to true; explicit false remains false.
`ToolPolicy: null` remains distinct from a present policy with zero timeout.

Runner mapping: use `effective.max_turns` for native `RunPolicy.max_turns`,
`effective.model_idle_timeout` for runtime `RunConfig.model_idle_timeout`, and
`effective.max_tool_output_bytes` / `effective.untrusted_tool_outputs` for
`OutputPolicy.max_bytes` / `OutputPolicy.untrusted`. None disables the latter cap
and model timeout. Stream activity resets the idle timeout; non-streaming
operations use it as a hard deadline. The codec does not install timers.

**Do not map Go ApprovalRequired to native ApprovalPolicy::All.** Go applies it
only to mutating, non-control-flow tools. Likewise, DefaultTimeout is an override
of eligible tools' own timeouts, not a blanket replacement of every native
ToolPolicy. The runner adapter now enforces mutation-only approval through registered tool identity, preserving tool-owned approval and all access/name denials. Host timeout overrides the tool's own optional timeout. Other callbacks,
compaction/handoff settings, and authorization fields are outside this projection.

## Approval and history integration

Module: `adk_codec::approval`.

```rust,ignore
let marker = ApprovalMarker::from_call(&call, ApprovalPhase::Pending, agent);
let boundary = ApprovalMarkerBoundary {
    before_item: native_history.len(),
    marker,
};
let wire = encode_history(&native_history, &agents, &recorded_boundaries)?;
let restored = decode_history(&wire, &recorded_phases)?;
```

Exact signatures:

- `encode_history(items: &[adk_core::RunItem], agents: &[Option<dto::AgentRef>],
  markers: &[ApprovalMarkerBoundary]) -> Result<Vec<dto::RunItem>, BridgeError>`
- `decode_history(wire: &[dto::RunItem], phases: &[ApprovalPhase])
  -> Result<NativeHistory, BridgeError>`
- `NativeHistory { items, agents, markers }` retains the native items and explicit
  provenance/marker sidecars.
- `encode_item(item, agent)` / `decode_item(wire)` bridge the supported item subset.
- `ApprovalMarker::from_call(call, phase, agent)`, `to_wire()`, and
  `to_request(reason)` bridge native calls/requests. Reason is explicitly external:
  Go ToolApprovalData cannot carry it. `approval_call(data)` returns a native call.

`before_item` counts **native items, excluding all approval markers**. Record it
at the actual observation boundary, not from final pending state. Boundaries must
be nondecreasing and at most items.len(); equal boundaries retain supplied order.
If a runtime emits an approval observation before appending its native ToolCall,
translate the boundary only when the actual history insertion order is known.
Compaction must rebase retained boundaries, or export the pre-compaction history.

A pending marker has Approved=false, as does a denied marker. No wire bit can
reconstruct the distinction. Decoding therefore requires exactly one explicit
phase per wire marker; contradictory Approved bits fail. These sidecars are not
approval grants and are not a checkpoint/resume authorization mechanism.

The baseline runner appends pending markers among tool results; the chat loop
later appends a resolved marker followed by its tool output. Do not move pending
markers next to calls or group all resolved markers before all outputs. Parallel
calls and repeated pending/denied false bits make those transformations incorrect.
The codec neither sorts nor deduplicates calls/markers/results and does not invent
denial outputs. The runner supplies the actual outputs (including host reasons).

## Explicit boundaries of support

History conversion supports text-only user/assistant messages, tool calls with
present JSON arguments (including explicit null), and text-only tool results.
Exactly one text block is required, preserving native segmentation. Agent sidecars
are required for every item; user messages require None, assistant messages Some.
Unknown types, handoffs, reasoning, compaction, images/media, message phases,
extraneous payloads, native system/developer roles, and should_pause=true tool
results fail explicitly. Keep unsupported values native or add a deliberate,
tested richer bridge; never flatten them silently. `decode_item` leaves provenance
in its input; use `decode_history` to retain it automatically.

Markers retain their complete ToolApprovalData, including missing RawJson input;
converting missing input to a native ToolCall fails because it is not JSON null.
Native approval request reasons must remain separately recorded if needed.
Tool timeout seconds that would overflow Go's signed nanosecond duration are
rejected rather than reproducing wrapped, potentially disabled deadlines.

## Verification

`cargo test -p adk-codec` runs the bidirectional codec tests, including mixed
parallel pending/approved/denied ordering, duplicate/end/future boundaries,
provenance, JSON null, signed/default sentinel preservation, and fail-closed
unsupported cases.

`cargo test -p adk-codec --test go_runner_differential -- --ignored` additionally
requires Go and the pinned `repos/sdk` checkout. It verifies the checkout commit,
extracts unmodified effective-config and denial-history functions, executes them
in a temporary standalone Go program, and compares 21 config cases plus ordered
denied marker/output pairs against Rust. This intentionally does not claim to
exercise the full Go runtime or provider dependencies. The temporary program is
removed after execution. Minimal containers may need `GOROOT` set explicitly.

## Runtime event and gate adapters

`GoEventAdapter` translates awaited `TextDelta`/`CommittedItems` observations to
Go stream snapshots without a second engine or detached producer. Settled tool
batches preserve pending markers interleaved with eligible outputs in call order.
This is a stream projection, not a resumable checkpoint: `should_pause` remains
on the native outcome/continuation while the wire output carries its text/error
payload. Native handoffs project to `Handing off to <target>` tool outputs, with
source-agent provenance; skipped sibling outputs remain in original call order.

`apply_go_config` also installs consecutive-tool-error and stop-gate block limits.
Managed-subagent limits are returned explicitly for the dedicated host integration.
`StopGate` is bounded by parent cancellation/deadline. Its consecutive block cap
resets after tool execution or `end_turn=false`, and a blocked final answer at the
turn boundary gets the same one-turn extension as the Go runner.

Model-specific threshold lookup and 104 local compaction cases are executed
against the pinned SDK. Full approval histories/new items and streamed snapshot
order are now compared in real-engine replay; see the replay README.
