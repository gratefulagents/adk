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

## Verification and source references

Run `cargo test -p adk --features observability --test observability` for native tests and `cargo test -p adk --features otel --test observability` for actual SDK spans as well. Coverage includes fragmented/malformed/oversized JSONL, fanout order/backpressure, default/full/operator redaction, complete pre-cap output, a real runner, cumulative usage, real paused continuation, successful two-turn spans, shared-run isolation, ended-parent contexts, final error attributes, filesystem modes/confinement and quota/hardlink defenses.

Reference behavior was read from `repos/sdk/pkg/agentsdk/events/events.go`, `tracestore/trace_writer.go`, `tracestore/trace_store.go`, `tracestore/filesystem_linux.go`, `otel/processor.go`, `repos/sdk/internal/agent/stream.go`, and the native `adk-core` types/Host plus `adk-runtime::RunHooks` and runner lifecycle. Deliberate format and lifecycle differences are enumerated above rather than labeled full Go parity.
