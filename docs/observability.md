# Observability

For the separate SDK category-store APIs and host-owned exporter constructors,
see [trace-store and telemetry ownership](trace-store.md). The native event
pipeline documented here retains schema 1; it is not silently relabeled schema 2.


## Parent wiring

The implementation is owned by `crates/adk/src/observability.rs` and is opt-in:

- `observability` feature: `runtime`, optional `adk-security`, `serde`, `serde_json`, `sha2` (0.10), `tokio`, and Unix `rustix` (1, `fs`).
- `otel` feature: `observability` plus optional `opentelemetry` (0.31, default features disabled, `trace`).
- Library: `#[cfg(feature = "observability")] pub mod observability;`.
- Tests: `tempfile` (3) and `opentelemetry_sdk` (0.31, default features disabled, `trace`, `testing`).

The OTel bridge uses the actual OpenTelemetry tracer API. Hosts supply a tracer and explicit parent context, own the SDK provider/exporter, and flush/shutdown that provider themselves. No global provider, environment lookup, implicit stdout exporter, or network connection is installed by ADK.

## Attach hooks and host events

Create one `Arc<Observability>` per run ID. Set it as `RunnerConfig::hooks` and put the same instance in `ObservedHost::observations`. `ObservedHost::host` is your existing host: approval decisions and original run events are delegated unchanged. Sensitive original events must still go only to a trusted host. Attaching only hooks omits host completion/failure events; attaching only the host omits raw pre-truncation outputs and hook-derived progress.

```rust,ignore
use adk::observability::{CapturePolicy, FilesystemTraceStore, Observability,
    ObservedHost, TraceLimits};
use std::sync::Arc;

// Unix: the root must be newly created or already owner-private (0700).
let store = Arc::new(FilesystemTraceStore::create(
    "private-trace-root", &context.run_id, TraceLimits::default(),
)?);
let observations = Arc::new(Observability::new(
    &context.run_id, CapturePolicy::default(), vec![store],
)?);
runner_config.hooks = Some(observations.clone());
let host = Arc::new(ObservedHost {
    host: trusted_host,
    observations: observations.clone(),
});
// Build and run your Runner with runner_config and host.
// Keep observations alive while a paused continuation can still resume.
// After final completion, error, cancellation, or abandoning a continuation:
observations.shutdown().await?;
```

Do not install the same pipeline simultaneously as both agent-level and run-level hooks; both registrations receive observations and would double-count them. Unique run IDs distinguish concurrent runs. For durable replay these hooks are explicitly observational: callbacks may be missing/repeated and do not provide exactly-once persistence or effect idempotency.

## Native event codec and ordering

`EventRecord` is a native version-1 JSONL envelope with `schema_version`, `run_id`, positive `sequence`, `timestamp_unix_ms`, `kind`, `data`, and `progress`. It is **not** Go's trace-schema-2 envelope or Go's content-event wire format. Millisecond timestamps are wall-clock observations; the sequence is the ordering authority.

- `EventRecord::{to_json_line,from_json_line}` round-trip native records; decoding rejects malformed JSON, unsupported schema versions, missing run identity and zero sequence.
- `LineDecoder` accepts fragmented UTF-8, several lines per write, CRLF/blank lines, and explicit EOF without a final newline. Each malformed line returns an error. Over-limit lines return one error, consume no further buffer space, and are discarded through the next newline; the following valid line is retained. The configured limit excludes the newline delimiter.
- Both hooks and host events pass through one awaited mutex. Concurrent producers acquire one total order, not a guessed timestamp order. Every sink is awaited in registration order under that lock; slow sinks exert backpressure. Sinks must not call back into the same pipeline, including `snapshot()` or `health()`.
- Sink failures are fail-closed and propagate to the runner. Later sinks do not see the failed record. Already delivered sinks are not rolled back; a failed/cancelled delivery can leave a sequence gap and must not be blindly retried as an exactly-once operation.
- `TraceHealth` exposes attempts, fully delivered events, errors, and a safe generic last-error message. Underlying error sources remain available to trusted callers, not in serialized health. `events_written` counts successful fanout, not necessarily disk writes when no filesystem sink is installed.

Hook names cover agent start/end, model attempts/acceptance, cumulative usage, retries/fallbacks, raw tool output, handoffs, committed items with provenance/approval markers, replaced history, approval decisions, compaction and validation. Host names include `run_start`, `delta`, `reasoning_delta`, `tool_arguments_delta`, `item`, `model_complete`, `host_tool_start`, `host_tool_end`, `approval_required`, `done`, and `error`. Hook and host channels retain separate names where they describe the same activity; they are not deduplicated content feeds.

`publish(context, kind, data)` is the host extension boundary for status/log/phase/child-tool/subagent events. Put user/tool content under `text`, `input`, `output`, or `payload`; never hide content in identity or metric fields. There is no automatic Go content-line recognition, terminal renderer, or inference of child lifecycle events that the runtime did not emit. A `done` event with `status: "paused"` keeps the pipeline open for the same continuation; final done/error closes publication.

## Progress

`snapshot()` returns a detached `ProgressSnapshot`: sequence, agent turns, model attempts, tool calls/results, retries, handoffs, compactions, cumulative usage/cost, and the last 20 event names. The runner's `Usage` hook is cumulative, so repeated usage snapshots replace totals instead of adding them. Cost uses the host estimator's units, not an assumed USD currency. Agent-start hooks fire each model turn, including retries, so `agent_turns` does not mean unique agents.

This does not synthesize Go's session numbers, inferred current step, text summaries, per-model cost attribution, child aggregation or pending Kubernetes events. Content is not retained in the progress ring.

### SDK-shaped progress state

`adk::progress::ProgressTracker` (also `RunProgress`) is a **separate**
`observability`-gated API. It does not change the native `ProgressSnapshot` above
or its cumulative-usage semantics. Its `snapshot()` uses SDK-shaped fields,
signed token/call counters, per-model token totals, and a 20-event ring.

```rust,ignore
use adk::progress::RunProgress;
use adk::codec::dto::Usage;

let progress = RunProgress::new();
progress.set_session(1, "starting");
progress.record_assistant_text("Inspecting the repository");
let child = progress.child("child-task-1");
child.record_llm_usage("model-name", 0.01, &Usage::default());
let snapshot = progress.snapshot();
assert_eq!(snapshot.session_number, 1);
```

Clones share state; snapshots own their data. Children copy the parent's session
number, result-size setting, processor and root span ID at creation, but have
independent activity, step inference, text, event rings and model maps. Each
child LLM usage increment and tool-use count forwards through every ancestor.
Child completion does **not** add those totals a second time. Session changes
reset session counters, not run totals. Pending session-start/completion events
are drained separately from the bounded event ring; no Kubernetes client or
automatic event publication is installed.

The tracker can emit structural session, subagent, retry and compaction spans
through a host-owned `TraceProcessor`. Subagent completion closes its matching
start span; completion without a start produces a point-in-time span. The tracker
does not flush processors, persist checkpoints or redact its text/metadata.
Snapshots, pending events and processors therefore belong to trusted hosts:
unlike the metadata-first native pipeline, this API retains assistant content
and caller-supplied lifecycle summaries. Do not feed cumulative native `Usage`
observations into its incremental `record_llm_usage` method.

Structural callbacks run outside the state lock. Concurrent non-reentrant
structural record calls wait for their callback delivery before returning, so a subsequent host flush
cannot overtake their spans. Same-thread callback reentry is a Rust extension:
nested notifications queue behind the active callback and are delivered when it
returns. A nested record return is therefore not a flush barrier. Callbacks must
not panic, wait on other threads recording into this tracker, or flush/end the
enclosing trace while reentrant notifications remain queued.

`adk::progress` also exposes the SDK's `extract_file_path`, `extract_path`,
`extract_grep_pattern`, `extract_glob_pattern`, `extract_bash_command`,
`extract_lsp_operation`, `extract_agent_meta`, and `extract_ask_user_question`
inspection helpers. They accept raw JSON bytes, preserve duplicate-field order
and case-insensitive field matching, and apply Go's null/type-error behavior.
In particular, LSP extraction retains successfully decoded fields despite a
different field's type error; the other simple extractors return an empty value
on decoding errors, and agent metadata returns the `unknown` description.
Unknown fields need not fit a finite floating-point JSON value. These functions
are display helpers, not argument validation or permission checks.

`truncate`, `truncate_bytes` and `truncate_middle` preserve their distinct SDK
limits (Unicode scalars, UTF-8 bytes, and head/tail scalars respectively). Their
Rust text inputs are valid UTF-8; raw JSON helpers replace malformed UTF-8 and
lone UTF-16 surrogates as Go does. No helper invokes a tool or writes a file.

### Runtime progress adapter

`progress::runtime::RuntimeProgressAdapter` connects this tracker through the
optional synchronous `RunnerConfig::progress` interface. It is neither a
`RunHooks` policy hook nor a generation observer. Do not also feed the same
tracker cumulative usage callbacks: per-call accounting already comes directly
from the runner, including provider/summary compaction and genuinely completed
steering-superseded calls. Retained failed-stream responses remain in native
budget accounting but are deliberately omitted from SDK progress, matching the
SDK's discarded response on Complete → Error. Cancellation alone fabricates no
usage. Accepted model keys use the configured primary label, while out-of-band
calls use the actual request label; a host may explicitly override labels.

Pricing defaults to `UsdPolicy::Unpriced`: tokens are tracked without assuming
that a host estimator's units are USD. `HostUnitsAreUsd` is an explicit unit
assertion and retains available uncertain subtotals; uncertainty is reported
through `drain_diagnostics()`. `Custom` accepts a host USD-pricing callback.
Invalid prices and unsupported signed-counter conversions are diagnostic rather
than execution-policy failures. Diagnostics and snapshots are trusted-host data.

Root sessions are explicit owned scopes, not inferred from agent-turn or handoff
events. Call `adapter.session(number, step)` and consume the scope with
`finish(result, stop_reason)` once the session completes. This records pending
completion status under either span-ownership mode; dropping it invents no
completion. Default `SpanOwnership::External` leaves session/compaction spans to
the tracing owner. With tracker ownership, wrap only that owner's tracing hook
with `adapter.tracing_hooks(...)`, and finish `RunTrace` without its separate
session wrapper. Never filter policy hooks to suppress duplicate tracing.
`ContentOwnership::External` similarly avoids recording accepted content twice
when another adapter already owns it.

Automatic child assembly shares a session-scoped `ChildProgressRegistry` across
executor generations. Child runners replace any inherited template observer
with their immediate-parent scoped binding. Terminal/drop cleanup records status
once and does not charge aggregate child usage again. These bindings are
observational state, not an exactly-once durable accounting ledger.
Each executor generation has its own root observer: refresh to a new observer
or `None` affects new top-level children, not previously accepted executions.
The task-binding table remains shared so later descendants inherit their actual
parent, including an explicitly unobserved parent, across refreshes.

On durable descendant recovery, the scheduler supplies validated root-to-parent
ancestry. The registry restores absent ancestor bindings before forking the
descendant, even if those ancestors are still reconciling. Restoration replays
neither usage nor lifecycle starts; later ancestor execution reuses the same
binding. Historical model labels are not invented. Custom progress observers
must opt into the `restore_child` contract to observe independently resumed
subtrees; its default `None` leaves execution unchanged without misattributing
the subtree to the root.

## Capture policy and full outputs

The default `CaptureMode::Metadata` replaces content with `{ "sha256": "…", "bytes": N }`, using UTF-8 bytes for strings and compact JSON bytes for structured values. Identifiers, status and recognized metrics survive; unknown fields are digested rather than presumed safe. Digests still reveal byte length and equality and are not encryption. Run IDs and event-kind labels are host-controlled identifiers, not places for secrets.

`CaptureMode::Full` is an explicit high-trust opt-in. It retains complete raw output from `Observation::RawToolOutput`, which the runtime emits before its visible-output truncation/trust wrapping. No 16 KiB preview cap is applied by this module. The trace store can still reject a record exceeding its separate event quota; that is an error, not silent truncation.

Full capture recursively redacts credential-named JSON fields (`authorization`, `password`, `secret`, `token`, `api_key`, access/refresh/ID tokens), including JSON objects/arrays embedded in strings. Embedded JSON is normalized when redacted. Strings use the shared `adk-security` credential detector; a detected string is replaced in its entirety, not partially unmasked. Operator `CapturePolicy::redactors` run on strings with another credential check after transformation. The same processed payload reaches disk and OTel sinks. Directly constructing an `EventRecord` and calling a sink bypasses this pipeline policy and is a trusted low-level API.

Redaction is best-effort, **not DLP**. Encoded secrets, unrecognized credential names/formats, short tokens and credentials split across streaming deltas may evade detection. Full capture must only go to explicitly trusted storage/exporters. The Rust detector does not claim byte-for-byte parity with Go's regex collection. Full output retention does not imply that downstream OTel SDK/exporter attribute limits retain arbitrarily large payloads.

## Private local persistence (Unix)

Layout is `ROOT/traces/RUN/events.jsonl`, then `events.jsonl.001` through `.004`. This single ordered native stream is intentionally different from Go's category files (`llm_calls`, `tool_calls`, etc.).

- New directories are 0700 and files 0600. Existing public root/traces directories are rejected rather than silently repurposed. Create a dedicated root; `.` and `/` are rejected.
- Every path component is walked with directory-relative descriptors and `NOFOLLOW`. Parent traversal and symlink components are rejected. Appends use already-open descriptors, so replacing a root path with a symlink does not redirect later writes.
- Run IDs are restricted to at most 128 ASCII letters/digits/`._-`, excluding `.`/`..`. Run directories and chunks are created exclusively; existing runs, files, hardlinks and rotation targets are never reopened or overwritten. There is no cross-process concurrent append or automatic resume of trace files.
- Defaults: 1 MiB per record including newline, 64 MiB per chunk, four additional chunks. Records rotate as whole lines; exhausted quotas are visible errors. Limits are independently configurable through `TraceLimits`.
- Each accepted record uses `write_all` followed by `sync_data`. A write/sync failure poisons further appends to avoid extending a potentially partial JSON line. Directory entries are not fsynced: this is diagnostic persistence, not a crash-atomic checkpoint protocol.
- Filesystem calls are synchronous bounded local I/O inside the awaited sink. A slow filesystem can block the executing thread; hosts needing isolation should implement a worker-backed `EventSink`. Permissions do not protect against root or another process with the same OS identity; the directory tree must be host-controlled.

There is no non-Unix filesystem fallback, arbitrary `WriteFile`/16 MiB artifact API, scoring/list/filter API, category rotation, retention cleanup, or trace-layout migration. Other sinks and the native codecs remain available without the Unix store.

## Actual OpenTelemetry integration (`otel`)

`otel::OtelBridge::new(tracer, parent_context)` accepts an actual `opentelemetry::trace::Tracer`, including `opentelemetry_sdk::trace::SdkTracer`. Tests export through the real SDK `InMemorySpanExporter`, not a mock transport. Add the bridge as an `Arc<dyn EventSink>` to the same pipeline as the trace store; explicit capture policy applies before either sink.

| Native observation | OpenTelemetry mapping |
| --- | --- |
| First record | `adk.run`, explicitly parented to the supplied host context |
| Agent start/end | `adk.agent`; repeated turns of the same active agent reuse its span |
| Model attempt/acceptance | `gen_ai.chat` client span; model/attempt and final input/output usage attributes |
| Retry/fallback | Ends the failed model attempt with error status; record remains a root event |
| Tool start/raw output | `adk.tool`; call ID, name, captured input, final output and tool-error attributes |
| Every record | Root span event with native sequence and processed JSON data |
| Final done/error | Final root status, sanitized error type/message; closes active spans |

Pauses do **not** end run/agent spans: resuming the same owned continuation preserves trace identity and sequence. Tool parent contexts remain available even after the parent span has ended, so a later `tool_start` carrying `parent_call_id` stays correctly parented. Runtime `Observation::ToolStarted` currently has no parent-call field: explicit nested relationships must be supplied by host `publish`; they are not invented.

`Observability::shutdown()` calls `EventSink::finish_run(run_id)`, which abandons only that run's remaining spans as incomplete; it does not terminate other runs sharing the bridge. Sink owners can call `EventSink::shutdown()` on the bridge after all producers stop to abandon all remaining runs. Neither operation flushes or shuts down the host's SDK provider. Finally call that provider's `force_flush`/`shutdown` yourself. Dropping a pipeline is not a substitute for awaited finalization.

Parity gaps: there are no Go-style endpoint/env/stdout constructors, OTLP transport/TLS setup, global tracer mutation, trace-ID-ready callback, full Go span-data union or complete GenAI semantic-attribute mapping. Provider latency/cache/cost/error-attempt metadata absent from native hooks is not fabricated. Phase/subagent/session/compaction records are root events, not separately synthesized lifecycle spans. `OtelBridge` is a working API/SDK bridge with host-owned exporters, not an ADK-managed OTLP service.

Handoff lifecycle observations bracket the existing transition: `handoff_start`
precedes the original source-agent/run `handoff` hooks; `handoff_complete` follows
the committed target checkpoint. Only `handoff` increments the progress handoff
counter. Start without completion is possible on cancellation/error; trace-owner
cleanup closes the outstanding span without inventing a successful transition.
The schema-2 TraceWriter transition feed still writes the original single handoff
record; the new observations are native operation-lifecycle events.

## Typed SDK event debug formatting

With `compat`, `adk::codec::events::compact_content_event` formats a typed
`adk::codec::dto::ContentEvent` using the pinned SDK's ordered debug fields.
It includes output only for error events, retains the first LF-delimited line of
reason/message/error output after outer trimming, and imposes no text-size cap.
It preserves raw agent/tool/status text, including whitespace. This function
neither logs nor redacts; apply host redaction before exposing its result.
It is distinct from native `EventRecord` serialization and does not claim the
`NewSessionEventStream` construction behavior.

Ten independently executed cases in `fixtures/content-event/observations.json`
cover optional fields/order, blank values, CR/LF, Unicode and long output.
The same oracle separately reflects all 83 SDK content-event fields and produces
284 canonical documents plus six nonfinite-float rejections. The codec test
compares exact Go JSON bytes after typed decode/re-encode. This caught precision
loss in the default JSON float parser; workspace `serde_json` enables
`float_roundtrip` alongside `arbitrary_precision` so typed costs retain their
original finite `f64` value. This is canonical typed-wire evidence, not proof of
Go's permissive case-insensitive/duplicate-field JSON decoder behavior. Timestamp
and slice fields remain separate ledger obligations despite their canonical cases
passing.

`adk::codec::events::parse_content_event_line(&str)` and
`compact_content_event_line(&str)` provide the separate SDK JSONL helpers.
An additional 1,429 independently executed line cases cover all fields, invalid
field types, duplicate/null/case-fold semantics, Unicode escapes, repeated slices,
Go's permissive timestamp parser and its 10,000-container limit. Successful
records compare exact serialized bytes; timestamps outside RFC3339 offset bounds
parse but fail serialization as in Go. Invalid/non-event lines use first-line
fallback. The public input is valid UTF-8, not arbitrary Go string bytes; no
redaction, output cap, stream construction or native event conversion is implied.
Run
`python3 scripts/content-event-reference/run.py --check` and
`cargo test --locked -p adk-codec --test content_events` to reproduce them.

## Verification and source references

Run `cargo test -p adk --features observability --test observability` for native tests and `cargo test -p adk --features otel --test observability` for actual SDK spans as well. Coverage includes fragmented/malformed/oversized JSONL, fanout order/backpressure, default/full/operator redaction, complete pre-cap output, a real runner, cumulative usage, real paused continuation, successful two-turn spans, shared-run isolation, ended-parent contexts, final error attributes, filesystem modes/confinement and quota/hardlink defenses.

Reference behavior was read from `repos/sdk/pkg/agentsdk/events/events.go`, `tracestore/trace_writer.go`, `tracestore/trace_store.go`, `tracestore/filesystem_linux.go`, `otel/processor.go`, `repos/sdk/internal/agent/stream.go`, and the native `adk-core` types/Host plus `adk-runtime::RunHooks` and runner lifecycle. Deliberate format and lifecycle differences are enumerated above rather than labeled full Go parity.

### Typed SDK subagent progress

With `compat`, `adk::codec::events::subagent_event` projects a SDK `ContentEvent`
into the owned `SubagentStreamEvent` DTO. It selects exactly `subagent_status`
with a nonempty task ID, preserves unknown status strings, and falls back from an
empty agent name to the subagent type. It copies all 21 fields, including owned
dependency/waiting lists (empty becomes Go nil), signed counters and exact costs.
The 294 pinned projection cases compare exact wire bytes and verify that mutating
the input does not change the projected lists. Original SDK subscription and
public line-writer subagent regressions also run in the reference checker.
This helper does not emit observations, subscribe to a runtime, translate native
`EventRecord`s or implement `LineWriter`.

### SDK JSONL host writer

`adk::codec::line_writer::LineWriter` (feature `compat`) is the separate owned
`std::io::Write` adapter. A synchronous `FnMut(LineEvent)` receives subagent
progress before child-tool classification, otherwise compact logs. Writes preserve
order and partial bytes; an empty write or explicit flush dispatches a pending
partial line. Drop does not flush. Shared use is host-owned via `Mutex`; a no-op
closure replaces a nil sink. The API documentation includes a runnable example.

The pinned scanner's 16-MiB boundary is a buffering/dispatch threshold, not an
output cap. Overflow changes dispatch to logs, including already scanned tokens,
then compacts the whole original buffer; this can repeat its first line. The
22-case oracle verifies this behavior, uncapped oversized error output, CRLF,
partial and empty writes, split UTF-8, no-op sinks and exact per-write ordering.
Large outputs compare complete hashes and byte lengths rather than prefixes.
Invalid raw Go string bytes are normalized to the same replacement-character
text as Go JSON rendering; native `String` cannot retain those raw bytes.
Redaction remains the host's responsibility. This adapter does not automatically
subscribe to the native runner or construct an SDK session event stream.

### Owned SDK content streams

With `observability`, `adk::content_stream::ContentStream` owns a `Write + Send`
writer and an in-process subscription bus. `child(task_id)` shares those resources
but copies the current session/phase/step. Setters affect that stream's context,
not previously created children. `emit` fills only missing/zero metadata, preserves
explicit values, and recalls subagent names from earlier status events. Generic
emission does not apply the SDK tool convenience methods' truncation policies.

`subscribe(0)` chooses 64 slots (the native replacement for Go's nonpositive
capacity); positive capacities are bounded. Slow consumers receive the exact
`events_dropped` marker before later events when space is available. Markers use
fresh timestamps and zero context, and go only to the affected subscription.
`close` is idempotent and leaves queued events readable. Dropping a subscription
unsubscribes; dropping the last stream releases the writer and closes remaining
senders. Subscriber events are independently owned, not shared Go slice storage.

Subscription delivery precedes best-effort JSON serialization and one writer call.
Marshal/write failures do not revoke already delivered events; emission does not
flush. Explicit `flush` reports the writer's flush result. Bus and writer locks are
separate, as in the SDK: concurrent producers have intact records and per-producer
order, not a global atomic ordering across both sinks. Use native `EventRecord`
observations when the native sequence contract is required.

**Limits:** automatic timestamps are UTC; the pinned comparator explicitly runs
in a UTC Go profile. Explicit offsets and zero instants in nonzero offsets are
preserved/recognized. Non-UTC host-local automatic timestamp parity, typed emit
conveniences and full `NewSessionEventStream` options remain separate obligations.
The API contains a runnable embedding example; source fixtures cover metadata,
annotations, subscriber loss and writer failure, with additional concurrent-record
and owned-lifetime regressions.

### Owned content-event logging

`adk::content_logger::ContentLogger` implements the SDK `AgentLogger.Event` log
**body** format under `observability`. The host explicitly supplies a trusted
`Write + Send` destination; no process-global logger or timestamp/prefix is
installed. `AgentLogger` below adds turn/tool body formatting over the same sink.
Although SDK comments call this stdout mirroring, Go `log.Printf`
uses stderr by default and permits host replacement of output, flags and prefix.
Hosts wanting such decoration wrap the supplied writer. This is not full
`AgentLogger` turn/tool logging or arbitrary Go printf compatibility.

Attach with `stream.set_logger(Some(Arc::new(logger)))`, detach with `None`.
Children snapshot the current logger along with metadata, but share its owned
writer. Parent replacement does not change existing children. After name
annotation, emission logs first, delivers to subscribers, then attempts JSONL
serialization/writing. Log failures do not stop delivery; marshal failures do not
undo logs or subscription events. These sinks are not one globally atomic
transaction across producers. Explicitly flush each destination; dropping the
last logger owner releases its writer without flushing.

Logs contain **unredacted** messages, identifiers and provider errors. They are
not the redacted native observation pipeline. Assistant previews cap at 200
**bytes**, attempt-error previews at 1,000, both followed by an ellipsis when
truncated. Other quoted event messages remain uncapped. Byte cuts inside UTF-8
use Go `\xNN` quoting, not replacement characters. The independent pinned
oracle covers 42 event cases, logger inheritance and a complete SHA-256/length
comparison over all 1,112,064 Unicode scalar values. It uses Go 1.26.8's Unicode
15.0.0 printable tables rather than the Rust compiler's changing Unicode tables.
See the runnable Rust API example and `scripts/content-logger-reference/run.py
--check`; Go module dependencies must already be cached for this offline check.

### Owned turn/tool logger

`adk::agent_logger::AgentLogger` composes the same owned sink with `LogLevel`.
`NORMAL` is zero, `DEBUG` is one, and every value >= one enables debug bodies.
Negative/unknown values are retained, not silently clamped. Use `event_logger()`
to share the sink with `ContentStream`; dropping `AgentLogger` does not close a
sink still owned by a stream. Multi-line calls attempt all lines even after a
write error, then return the first error. Each line is one write, without retrying
short writes. Call `flush` explicitly; drop does not flush.

| SDK operation | Native mapping |
|---|---|
| `NewAgentLogger`, `IsDebug` | `AgentLogger::new(level, writer)`, `is_debug` |
| `Turn`, `TurnEnd`, `Tools` | `turn`, `turn_end`, `tools` |
| `Instructions`, `InputItems` | `instructions`, `input_items` |
| `ToolExec`, `ToolResult` | `tool_exec`, `tool_result` |
| `ModelResponse` | `model_response(ModelResponseSummary)` |
| `ParallelToolCalls`, `AutoContinue` | `parallel_tool_calls`, `auto_continue` |
| `Debugf`, `Infof` | `debug`, `info` with Rust `Display` / `format_args!` |
| `Warn` / `Warnf`, `Error` / `Errorf` | `warn`, `error` with Rust `Display` |
| `Event`, `FormatToolNames` | `event`, `format_tool_names` |

Normal mode logs instruction/item/output byte lengths. Debug instructions use
2,000-byte quoted chunks; message previews use 500 bytes, item tool-call/output
previews 300 bytes, and tool-result previews 1,000 bytes. These are human-log
limits, not changes to uncapped trace capture. Unquoted tool previews preserve
raw truncated UTF-8 bytes, so a debug log is **not guaranteed valid UTF-8**.
Encrypted compaction content is represented only by byte length. Message roles
come from the supplied DTO's historical agent presence, not a current-agent guess.

The oracle compares complete raw log bytes for levels -2, 0, 1 and 2, including
exact limits, split UTF-8, all item discriminants, nullable message/compaction,
raw JSON interior whitespace and signed integer extremes. Rust-only checks
exercise disabled formatting, writer recovery, first-error retention and shared
sink ownership. The API includes a runnable embedding example.

**Limits:** this API does not reproduce Go's process-global destination or
prefix/timestamp flags, nor interpret Go printf strings. `input_items` accepts
UTF-8 DTO text and validated `RawJson`; arbitrary invalid Go raw messages and
outer JSON whitespace are not representable. Tool-call/output items must carry
their matching payload. Native body support does not close whole logger/session
parity: only the seven separately verified log-level/type/predicate ledger
records are added by this slice. All log sinks must remain trusted and private.

### Typed content-event helpers

`ContentStream::emit_text`, `emit_thinking`, `emit_thinking_delta`,
`emit_compaction`, and `emit_hook_decision` construct SDK content events through
`emit`, retaining its metadata, logging, subscription and JSONL ordering. Text
and thinking take one explicit agent name; pass `""` for no name. Thinking deltas
retain the attempt ID in `tool_use_id`. Compaction takes signed 64-bit counts
and narrows them to signed 32-bit event fields as on the supported SDK host.
These helpers do not truncate text, summaries or reasons, and do not redact
sensitive content. Use private sinks. UTF-8 text is required by Rust's API.

The pinned oracle compares 21 complete records, including empty fields, escaped
multiline Unicode, 68,000-byte content and overflowing signed counts, through
both JSONL output and subscriptions. Session construction and remaining typed
emitters are separate contracts, not implied by these helpers.

### Owned session-stream composition

`ContentStream::session(writer, SessionStreamOptions)` applies session/phase and
an optional owned logger, then emits an optional `SystemInit` before returning.
Subscriptions created afterward receive subsequent events, not an initialization
replay. `emit_system_init(SystemInit)` and `emit_session_end(SessionEnd)` use
named records rather than the SDK's positional argument lists. Turn counts are
signed 64-bit inputs narrowed to the SDK's signed 32-bit event fields; token and
duration metrics remain signed 64-bit. Tool/server list order and duplicates are
preserved. Empty lists are omitted from JSON just as in the SDK. Text is not
redacted or truncated by these session helpers.

| SDK session option | Native composition |
|---|---|
| `Session`, `Phase`, `SystemInit` | Corresponding `SessionStreamOptions` fields |
| `Logger` | `logger: Some(logger.event_logger())` |
| `EnableLogger: false` with no logger | Default `logger: None` |
| `EnableLogger: true`, `LogLevel` | Construct `AgentLogger::new(level, writer)` and attach its event logger |

This deliberately does not allocate a process-global logger or infer its output
destination. Choose a trusted host-owned writer (including stderr if desired).
Go global log prefixes/timestamps are not reproduced. Event bodies do not depend
on log level, while turn/tool methods on the retained `AgentLogger` do. Flush
stream and logger explicitly; neither session-end emission nor drop flushes or
closes other owners' resources. A runnable Rust embedding example is included
in `ContentStream::session` rustdoc.

The pinned oracle compares four session scenarios: absent/default/populated init,
normal/debug logging, signed narrowing, empty end markers, complete metrics,
and init-before-end ordering. Only the two typed boundary-emitter ledger entries
are closed here, not whole Go constructor/default-global-logger parity.

### Typed tool events and bounded UI payloads

`emit_tool_start(ToolStart)` and `emit_tool_end(ToolEnd)` use owned records for
names, call/parent IDs, agent identity, summary, input/output and duration.
`DEFAULT_MAX_TOOL_RESULT_BYTES` is the SDK's 32,768-byte event limit. Only raw
input and output are capped; tool-start summaries and other metadata are not.
Successful **and failed** output events use this cap. Whitespace is retained.
Truncation reserves three bytes for ASCII `...` and backs up to a UTF-8 boundary,
so the final payload never exceeds the cap or contains a partial character.
Exact-limit payloads are unchanged.

This is separate from debug logging's raw-byte preview rules and from trace
persistence. Generic `emit` is not newly capped; use the trace-store APIs to retain
uncapped tool output. These helpers do not redact sensitive content.

The pinned oracle compares **27 complete event records** through both JSONL and
subscriptions using full SHA-256 digests and byte lengths, not prefix samples.
Cases cover exact/overflow limits, all four alignments of a four-byte character at
the reserved cut, 80,000-byte Unicode inputs, empty metadata, signed duration
extremes and uncapped summaries. Original SDK `TestTruncateBytes` regressions
also run. A native end-to-end test composes these helpers with `LineWriter` and
verifies ordered typed child-tool progress with the capped input/error output.

### Typed subagent event snapshots

`emit_subagent_started(SubagentStarted)`,
`emit_subagent_completed(SubagentCompleted)` and
`emit_subagent_task_status(SubagentTaskStatus)` emit owned records through the same
annotation, logging, subscription and JSONL path. Completion includes optional
cache metrics as zero-default counters; no second positional-argument overload is
needed. Task-status input is an explicit projection, not an automatic conversion
from native scheduler state. Emission does not execute or transition a task.

Prompt/result previews trim the pinned Go Unicode whitespace set, replace LF with
space, and cap at 4,096 Unicode scalars including three ASCII dots. This is not
the tool-event byte cap. Descriptions, parent messages and dependency lists are
not capped or redacted. Dependency order/duplicates remain intact; empty lists
are omitted on the wire. Task durations are signed stored nanoseconds converted
to milliseconds toward zero, not recomputed from wall-clock time. Only exact
`completed`, `failed` and `cancelled` statuses expose task result/stop fields;
the task's separate error is not substituted for its result.

The pinned oracle compares 73 complete wire/subscriber records, including all Go
whitespace characters, interior controls, BOM/zero-width-space preservation,
Unicode truncation boundaries, signed metric extremes, status spellings and
inherited agent identity. Native `LineWriter` integration also checks ordered
owned progress and subscriber delivery before nonfinite-cost serialization fails.
These are emitter contracts, not scheduler or automatic runtime-wiring parity.

### Typed LLM-attempt events

`emit_llm_attempt` accepts either `LlmAttempt` (six owned fields) or a full
`ContentEvent`. The typed record sets both legacy and canonical aliases. Full
records always become `llm_attempt`, then fill only zero/empty legacy attempt,
scope, status and model fields from their canonical counterparts. Conflicts are
preserved, whitespace is not empty, and defaults do not run in reverse. Usage,
retry/fallback, timestamp and unrelated fields remain intact; this helper neither
infers metrics nor redacts/caps content. Normal stream annotation still applies.

Rust intentionally uses an `i32` attempt and typed records rather than Go's
runtime-checked variadic interface. Invalid argument counts/types and implicit
platform-`int` narrowing are not overloads. Callers must choose an explicit
representable attempt (or deliberate conversion) rather than relying on silent
Go argument rejection. The pinned oracle verifies 25 complete wire/subscriber
records, including every independent alias-default combination and rich metrics.
These helpers do not automatically attach to the runner/provider retry loop.

### Opt-in runner attempt/reasoning adapter

`ContentGenerationObserver` connects a host-owned `Arc<ContentStream>` to
`RunnerConfig::generation_observer`. Unlike manually calling emitters, this
adapter uses each real generation's ID to correlate start/end attempts, live
reasoning chunks and accepted final reasoning. Use `CompositeGenerationObserver`
to preserve an existing trace observer; replacing the config field outright
would replace that observer. Existing `RunHooks` remain separate and unchanged.

For a blocking run, explicitly set `RunnerConfig::blocking_reasoning = true` to
consume native streaming bindings. The default remains the completion path.
Complete-only models do not invent live chunks. In streamed runs, native model
events remain on the existing event channel and the adapter emits no duplicate
JSONL reasoning deltas. Hidden blocking text/tool events are not published.
Surfaced reasoning counts as visible output, preventing retry/replay after that
output. The execution-selection option participates in durable configuration
identity; observer callbacks remain observational, not exactly-once effects.

The first nonempty reasoning chunk emits immediately. Later chunks coalesce until
500 ms have elapsed or 4,096 UTF-8 bytes accumulate; model return flushes the tail
before the terminal attempt. There is no background timer. Accepted nonempty
reasoning items, not concatenated deltas or opaque continuation fields, supply
final thinking. A failed retained response is not accepted final thinking.

This is an explicit trusted-content sink: `ContentStream` does not apply the
separate `TraceWriter` capture/redaction policy. Sanitize before emission if the
host requires it, including before logger/subscriber delivery. The adapter owns
no store/exporter shutdown and does not implicitly flush the host's writer.

This bounded integration does not establish complete SDK attempt/turn numbering
under interruption, provider-specific completion-only reasoning callbacks, or
all automatic subagent wiring. Builder-created child configuration selection is
unchanged: opting the parent into blocking stream consumption does not silently
change child execution selection. Those broader parity obligations stay open.

### Public metadata and child-tool projection boundaries

Session option `session`, `phase`, and optional `system_init` fields, and init
model/mode/directory/turn metadata have individual pinned-fixture evidence.
Permission mode and directory here are reported metadata: constructing a stream
neither grants permissions nor changes the working directory. The signed init
turn value narrows to the source's 32-bit event field; it is not a runtime limit.
This does not imply automatic global-logger construction or whole options parity.

`adk::codec::child_event` projects only `tool_start`/`tool_end` events with a
nonempty parent ID. Start copies raw input; end copies output, error and duration;
inapplicable fields keep their defaults. Unlike the typed tool emitter, this
projection does not truncate raw payloads. Twelve directly pinned helper cases
cover rejection, defaults, signed extremes and large Unicode input/output.
