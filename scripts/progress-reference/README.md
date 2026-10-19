# Pinned progress oracle

`python3 scripts/progress-reference/run.py` archives SDK commit
`1dc92b73900fac74dc357a938e4b5eee6392b418` from `repos/sdk` into a temporary
folder under `/workspace/scratch` (override `ADK_REFERENCE_TMPDIR`), installs
three Go test harnesses there, and executes real SDK methods.
It never edits the reference checkout. `--check` regenerates all files in memory
and compares their complete content, including source/harness hashes and Go
version, before printing `Pinned progress observations reproduced exactly`.

## Tracker fixture

`fixtures/progress/observations.json` contains 16 named scenarios and 88 operation
observations. `cases.json`
contains stimuli, not expected behavior. Rust replays each operation through
`ProgressTracker` and compares the corresponding observation, including every
existing parent/child tracker. Actual Go/Rust snapshot serializers are used.
The extra `model_usage_wire` string checks ordered model serialization explicitly.

Coverage: initial/null-map snapshot; session and deduplicated explicit steps;
Unicode text truncation and thinking isolation; tool-derived steps and result
errors; lifecycle/result summaries; no-op hook; additive model/cache usage;
nested forwarding with parent activity isolation; child option/root inheritance;
session resets and non-double-counting completion; ring eviction versus pending
queue/drain; retries, compaction, subagent progress, terminal metadata removal,
nonterminal retention, span reuse and orphan completion.

Clock timestamps become `<time>`; measured subagent elapsed durations become
`<elapsed>`. Explicit session durations and retry delays remain exact. Generated
span UUIDs are assigned encounter-order aliases, retaining start/end identity.
Root IDs are fixed test input and never normalized. Span callbacks are captured
at callback time rather than retaining mutable Go span pointers. Trace start/end
and flush callbacks are also observed (and absent for these tracker operations).

Go span structs and Rust tracewriter structs are explicitly projected to a
shared field vocabulary. Empty Go file slices project to lists, matching Rust's
nonnullable vectors; this is a span projection, not a comparison of raw span
DTO wire shapes. Snapshot null versus empty arrays/maps remains exact. Float
projections use the Go-compatible float serializer; no tolerance is applied.

## Helper fixture schema

`fixtures/progress/helpers.json` has `provenance` and `cases`. Each case has:

- `name`: unique diagnostic/test name.
- `function`: exact Go function name (`ExtractFilePath`, `ExtractPath`,
  `ExtractGrepPattern`, `ExtractGlobPattern`, `ExtractBashCommand`,
  `ExtractLSPOperation`, `ExtractAgentMeta`, `ExtractAskUserQuestion`,
  `Truncate`, `TruncateBytes`, `TruncateMiddle`).
- `input`: raw JSON text for extraction, plain text for truncation; omitted if
  empty. **Do not parse and reserialize extraction inputs**: duplicates, raw
  numbers, nesting, and escape sequences are deliberate.
- `input_bytes`: JSON numeric byte array instead of `input` for malformed UTF-8.
- `limit`: signed integer; only truncation functions use it.
- `expected`: observed string, actual PascalCase AgentMeta object, or LSP object
  `{"operation": "...", "file_path": "..."}`.

236 helper cases exercise duplicate/null/case aliases, invalid JSON/type errors,
partial LSP decoding, unknown huge numbers/deep nesting, lone/paired surrogates,
malformed UTF-8, AgentMeta fallback/limits, structured/simple question forms,
and Unicode/rune/byte/middle truncation boundaries. No output normalization is
applied to helpers.

## Runner accounting fixture

`fixtures/progress/runtime.json` records 12 actual SDK runner executions:
`Run`/`RunStreamed`, clean completion/completion followed by stream error, and
tracker depth zero/one/two. All four token counters, USD subtotal, configured
model key and last accepted text are observed at every ancestor. A complete
response followed by an error contributes no SDK tracker accounting or accepted
text; clean completion contributes once. The model uses the public streaming
interface even for `Run`, and the fixture asserts the expected error outcome.
This distinguishes SDK tracker semantics from native runtime retention of a
failed stream's authoritative response; native budget accounting is not changed
to match the SDK's discarded-response behavior.

## Scope limits

Tracker/helper cases and the bounded runner accounting fixture do not establish
full runner/host parity. Processor replacement during an open span, snapshot mutation
independence, and concurrent/reentrant callback stress are not covered here.
The Go baseline tests selected by `run.py` additionally cover existing SDK
tracker and forwarding behavior. Rust runtime bridge coverage is separately owned.
