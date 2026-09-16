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

## Canonical comparison boundary

* Exact model name, instructions, ordered model-input history, advertised tool
  names, executed tool name/arguments/output, final/new history, response count,
  last agent, input/output token totals, final output and outcome category.
* Streamed event projection compares text deltas and committed items in order.
  Go emits committed run items; Rust model-complete items and tool-finished
  payloads describe the corresponding semantic events. Runtime-specific
  lifecycle/telemetry envelopes are not wire-compatible and are not compared.
* Text-only message items omit role because Go `RunItem.Message` has no role.
  Rust inputs use User and scripted responses use Assistant. Call IDs are not
  renamed, arrays are not sorted, Unicode/text is unchanged, null final output
  on partial failure is retained, and JSON arguments are compared structurally.
* Go uses StreamResponse even inside Run. Rust's regular path uses complete and
  its streaming path uses stream; the mocks assert these dispatch choices.
* Both sides explicitly disable untrusted-output wrapping for the trusted
  in-process echo tool. No normalization strips delimiters after execution.
* Go's approval `RunItem`s are projected into an ordered `pending` side channel,
  matching Rust `pending_approvals`. They are omitted from the model-facing
  history and committed-item event projection. All non-approval entries retain
  order and content. Approval event timing/wire marker identity is NOT compared.
* This corpus does not claim parity for policy denial, approval resume, cancellation,
  general retry/advice policy, handoffs, arbitrary custom parser implementations, other provider
  failures, multimedia, timing/backpressure,
  concurrent tool scheduling, raw telemetry or partial stream errors. Separate
  engine tests cover selected Rust behavior; this is not a parity claim for
  those excluded cases. Unresolved external differences are enumerated in
  `docs/runner-design.md` and require resolution before #4 is scope complete. Scripted streaming is not a provider test.

## Licensing and provenance

The Go runner is from gratefulagents/sdk commit
`1dc92b73900fac74dc357a938e4b5eee6392b418`, GPL-3.0. See
`fixtures/licenses/SDK-GPL-3.0.txt` and `fixtures/NOTICE.md`. The new harness and
fixture derivatives are GPL-3.0-only, consistent with this workspace. Scenario
inputs are synthetic and contain no user data, generated IDs or timestamps.
