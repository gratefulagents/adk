# Go baseline SDK codecs

This GPL crate is independent of `adk-core`, the runtime, and platform integrations.
It exposes `replay(operation: &str, input: &serde_json::Value) -> Result<Value,
CodecError>`, typed public DTOs in `dto`, `schema::<T>()` for their JSON Schemas,
and `GoTimestamp` for nanosecond RFC3339 content-event timestamps.

Supported operations:

- `snapshot_items`: numeric Go `RunItem.Type` to string snapshot types, selecting
  the corresponding payload, adding reasoning/thinking aliases, and applying the
  distinct Go wire and snapshot omission rules.
- `response_snapshot`: aggregate item text, reasoning and tool calls; preserve an
  explicit false `EndTurn`, usage and raw provider JSON.
- `child_event`: select parent-linked tool start/end events and construct the Go
  `ChildToolEvent` wire shape.
- `state_ready`: bounded fixture event replay of project initialization, task
  creation, claiming and closing, with sequence/dependency checks and ready-list
  ordering. This is not the filesystem state engine or a complete event reducer.

The separately licensed platform `codec` module implements `persist_transcript`
and delegates SDK operations here. There is no platform dependency in this crate.
The executable fixture runner is owned by the workspace harness, not this crate.

## Wire contract and bounds

DTOs follow the pinned SDK sources referenced by `fixtures/sdk.json`:
`internal/agent/{items,usage,model,llm_snapshot,event_stream}.go` and
`pkg/agentsdk/session_event_stream.go`. Existing checked-in Go-generated inputs
and expected outputs are tested without altering the corpus. The platform module
is derived from the separately licensed transcript codec and stays in
`adk-platform`; see repository fixture provenance notices.

- Raw provider/tool/schema JSON uses arbitrary-precision `serde_json::Value`;
  typed Go integer fields remain checked `i64`/`i32`. No float conversion is used
  for JSON payloads or timestamps. Actual Go float64 cost fields use float64 and
  Go-style JSON formatting (including `0`, not `0.0`). The Value API
  normalizes integer negative zero to `0` as serde_json does; signed-zero lexical
  fidelity is not supported.
- `RawJson` preserves missing raw bytes versus explicit JSON `null`. Optional
  Go pointers collapse absent/null as Go unmarshalling does; explicit false is
  retained. Non-pointer null scalar fields deserialize to their Go zero value.
- SDK numeric item types intentionally allow unknown integers: actual Go snapshot
  code emits `"unknown"`. This differs from the narrower Python reference, which
  rejects them. Persisted platform item types reject unknown values.
- `AgentRef` is only the name projection used by snapshots, not the executable
  Go Agent type. Arbitrary unknown Go object fields and case-insensitive aliases
  are not promised to roundtrip. Known ContentEvent fields are typed.
- Timestamps accept valid RFC3339 dates with up to nine fractional digits and
  numeric offsets, trim trailing fractional zeroes, and encode zero-offset UTC
  with `Z`. Precision beyond nanoseconds and Go parser's permissive non-RFC3339
  forms are rejected by DTO deserialization. The separate JSONL parser below
  preserves Go's permissive parsing. State fixture ordering requires UTC timestamps.
- The platform operation consumes lossy SDK snapshots, not live Go RunItems;
  it cannot reconstruct omitted empty agent-pointer identity. It is a baseline
  JSON transform, not gzip storage, restart restoration or database integration.

Tests cover all six SDK fixtures and the platform fixture, typed wire roundtrips,
all eight item types, absent/null/false/empty, exact large numeric payloads,
Unicode, Go float formatting, timestamp precision/calendar validation, schema
wire names and enum types, and rejection of malformed state events.

## SDK JSONL lines

`events::parse_content_event_line(&str)` returns a typed event or `None` for
malformed records and empty/missing event types. `events::compact_content_event_line`
formats an event or returns the first trimmed raw line. Both accept UTF-8 Rust
strings; they neither redact nor log, and impose no line/output size cap.

The parser preserves Go case folding (including long-s and Kelvin-sign aliases),
ordered duplicate fields, null scalar no-ops, repeated slice backing values,
unpaired-surrogate replacement, checked integer/float decoding and the 10,000
JSON-container nesting limit. Unknown fields are ignored. Timestamp parsing also
accepts Go's single-digit hours, comma fractions, excess fractional digits
(truncated to nanoseconds), and offset-minute rollover. As in Go, accepted offsets
of 24 hours or more cannot subsequently serialize as RFC3339. Timestamp JSON
escapes are rejected, matching Go's timestamp unmarshaler. This permissive path
does not change the stricter DTO deserializer or create a session event stream.
