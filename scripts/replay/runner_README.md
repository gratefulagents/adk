# Deterministic Go → Rust runner replay

Unlike snapshot-only codec vectors, `runner_export.go` calls the real pinned Go
`Runner.Run` / `Runner.RunStreamed`. Its model implements the public model
interface with scripted responses; its echo function is executed by the runner.
Rust `crates/adk-runtime/tests/replay.rs` feeds the same scripts to the actual
Rust engine, recording model requests, tool dispatch and host events. Neither
side derives observations from the expected output.

```sh
# Refresh: defaults to offline, requires a warm Go module cache.
python3 scripts/replay/runner_export.py
# Re-execute Go and require byte-identical fixtures and provenance.
python3 scripts/replay/runner_export.py --check
cargo test -p adk-runtime --test replay
```

For a cold cache, use `--allow-downloads` once. No provider, credentials, external
commands, network tools or persistence are exercised. The exporter uses an
allowlisted environment and fresh empty HOME. Go module resolution is the only
optional network operation. The SDK checkout must match the pin and have no
tracked edits. `runner_manifest.json` pins source URLs/hashes, harness/input
hashes and the generated fixture hash. The complete SDK is pinned by commit;
the listed files identify the principal execution contracts, not the entire
transitive dependency graph. Dependencies are locked by the pinned go.sum.

## Cases

Normal final text, two successive tool turns with distinct call IDs and Unicode
arguments, max-turn partial after a completed tool result, and explicit
`end_turn=false` continuation. All four run in regular and streaming modes.
Streaming scripts deliver separate text deltas and a complete response; the
harness drains actual runner events rather than reconstructing deltas from the
final text. Each max-turn run retains its history, outputs, responses and usage.

Added 12 fallback/schema cases: sticky fallback with successful primary reprobe;
failed primary reprobe; fallback-chain switch resetting the three-success count;
and non-JSON/schema-invalid JSON/valid JSON structured output. Each runs normally
and streamed. The Go provider returns explicit overloaded/retryable advice; Rust
uses its Provider-error retry predicate. Both execute real selection logic.
Schema input is serialized to compact, sorted-key JSON before configuring either
runner so the schema prompt bytes are identical (no output normalization).

Two further approval scenarios interleave two deferred approval calls with an
eligible sibling. Both real runners must execute that sibling, pause, and return
both pending call IDs in order. Resume safety/decision validation is separately
covered by Rust tests; these two Go cases only test the initial suspension.

Four custom-parser scenarios bring the total to 26: parser rejection preserves
raw JSON; parser success transforms the value. Both modes use a named, non-strict
schema and compare the resulting model instructions as well as final output.

Ten output scenarios cover trust wrapping, embedded opening markers, already
wrapped text, UTF-8 caps and caps too small for the wrapper. Raw items/events and
wrapped model history are compared without delimiter normalization. Four default
budget cases map Go zero/negative sentinels to the public Rust policy default.
Two actual ChatLoop/Conversation cases cover success and turn-limit partials:
Go's nil result slot is recovered from MaxTurnsExceeded.PartialResult itself.
Two additional normal-path cases compare actual Go ChatLoop approval handling
with Rust continuation for one and two approved calls. An eligible sibling must
execute exactly once. Shared dispatch recording includes approved tools, and all
hooks, requests, history and final usage are compared across the entire resume.
Two denied-resume cases also compare Go ChatLoop and Rust: one or two calls
receive error tool results without executing, then the model continues. The Go
approval tool panics if accidentally invoked under denial. These cases preserve
exact denial text and confirm the eligible sibling is not repeated.
Two ordinary tool-failure cases verify paired error outputs and continued execution.
Two low-budget fallback cases prove that the initial failed attempt consumes a turn.
Four three-strike/stop-gate cases compare error escalation, consecutive block caps,
and turn-boundary extension. Two handoff cases compare source/target dispatch,
original-order skipped siblings, paired handoff outputs and source provenance.
The corpus now contains **56 cases**.

## Canonical comparison boundary

* Exact model name, instructions, ordered model-input history, advertised tool
  names, executed tool name/arguments/output, final/new history, response count,
  last agent, input/output token totals, final output and outcome category.
* Lifecycle hook projection compares agent/model start, model completion with
  items/usage, tool start/raw output, and successful agent end with final output.
  The pinned Go implementation fires OnAgentStart on every model attempt, despite
  its interface comment saying once per run. Rust AgentStarted and ModelAttempt map to
  that callback followed by OnLLMStart; Rust Started is a separate run-start event.
  ModelAccepted / ToolStarted / RawToolOutput / AgentEnded map to the
  corresponding callbacks. Order is preserved, including failed model attempts;
  no sorting or expected-golden reconstruction is used. This proves the projected
  lifecycle for these scripts, not raw OTel payloads or concurrent callback order.
* Streamed event projection compares text deltas and committed items in order.
  Go emits committed run items; Rust model-complete items and tool-finished
  payloads describe the corresponding semantic events. Runtime-specific
  lifecycle/telemetry envelopes are not wire-compatible and are not compared.
* Empty tool-result content and false error flags are omitted to match Go's
  omitempty JSON representation; nonempty text is compared verbatim.
* Text-only message items omit role because Go `RunItem.Message` has no role.
  Rust inputs use User and scripted responses use Assistant. Call IDs are not
  renamed, arrays are not sorted, Unicode/text is unchanged, null final output
  on partial failure is retained, and JSON arguments are compared structurally.
* Go uses StreamResponse even inside Run. Rust's regular path uses complete and
  its streaming path uses stream; the mocks assert these dispatch choices.
* Both engines use the script's trust and cap settings. Original trusted echo
  cases disable wrapping; output cases enable it. No normalization strips
  delimiters or truncation markers after execution.
* Approval cases compare **full Go snapshot history and new items**, not only
  pending side channels. Pending marker provenance and resolved marker absence of
  agent identity are retained; denied outputs have no invented agent identity.
* All streamed cases compare **raw snapshot event order** via the awaited
  `GoEventAdapter`, including approval markers. This replaces the earlier
  marker-stripping committed-item projection. Sink backpressure and consumer-drop
  are independently tested. Native event enum envelopes remain Rust-native.
* This corpus does not claim parity for policy denial, re-deferred/streamed approval resume, cancellation,
  general retry/advice policy, arbitrary custom parser implementations, other provider
  failures, multimedia, timing/backpressure,
  concurrent tool scheduling, raw telemetry or partial stream errors. Separate
  engine tests cover selected Rust behavior; this is not a parity claim for
  those excluded cases. Representation boundaries and the acceptance evidence map are enumerated in
  `docs/runner-design.md`; they are not waivers. Scripted streaming is not a provider test.

## Licensing and provenance

The Go runner is from gratefulagents/sdk commit
`1dc92b73900fac74dc357a938e4b5eee6392b418`, GPL-3.0. See
`fixtures/licenses/SDK-GPL-3.0.txt` and `fixtures/NOTICE.md`. The new harness and
fixture derivatives are GPL-3.0-only, consistent with this workspace. Scenario
inputs are synthetic and contain no user data, generated IDs or timestamps.
