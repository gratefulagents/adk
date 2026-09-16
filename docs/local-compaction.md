# Deterministic LOCAL compaction

## Scope and provenance

`crates/adk-runtime/src/compaction.rs` is a Rust-native port of the deterministic
local compaction path in SDK commit
`1dc92b73900fac74dc357a938e4b5eee6392b418` (GPL-3.0-only). It does not execute Go,
call a provider, or add dependencies at runtime. Source hashes, immutable source
URLs, exporter hashes, and fixture hashes are in
`fixtures/compaction_manifest.json`; the existing license copy is
`fixtures/licenses/SDK-GPL-3.0.txt`.

**Important baseline distinction:** Go `DefaultCompactionConfig()` sets
`UseLLMSummary=true`. The Go runner first tries provider compaction, then selects
a deterministic local plan, then optionally asks the model to replace that
plan's summary. This module implements the deterministic local algorithm, not
that additional model call. Its usage counters and cost are zero. Provider-native
compaction, opaque provider item transport, and LLM summary generation remain
outside this LOCAL deliverable (#5). The fixtures execute the actual Go planner
and default post-compaction finalizer, not a second handwritten algorithm.

## Exact policy and source evidence

All source paths below are under `repos/sdk/internal/agent` at the pinned commit.

| Behavior | Evidence | Implemented behavior |
|---|---|---|
| Default policy | `run_config.go:175–184` | enabled; trigger 180,000; target 100,000; recent 12; initial user messages 2; bullets 4 |
| Normalization | `run_config.go:257–279` | zero trigger → 180,000; zero or target ≥ trigger → max(1, trigger/2); zero preservation/bullet counts → defaults |
| Trigger | `history_compaction.go:61–71,126–154` | compact only when full estimated request **>** trigger; equality is a no-op |
| Disabled/empty | same planner | unchanged history, before=after=0, reason `disabled` |
| Request adjustment | `history_compaction.go:126–154` | negative overhead → 0; subtract overhead from both budgets, clamp each to 1, then normalize again in history planner |
| Token estimates | `history_compaction.go:184–266` | trimmed Unicode scalar count / 4 + 1, empty=0; message +8, call name+JSON +16, result +12, approval tool name+JSON input +8 |
| Request overhead | `history_compaction.go:235–256` | instructions +8; each tool name+description+schema +32; output reserve + safety buffer |
| Reserve | same estimator | positive `max_tokens`, else 16,384; raise to `thinking_budget` if larger; safety buffer 8,192 |
| Calibration | `runner.go:760–764,1281–1308` | ratio actual normalized prompt tokens / estimated sent prompt; 50% old + 50% ratio, clamp [0.5,2.5]; divide local trigger/target by calibration and truncate to integer |
| Calibration denominator | same runner block | excludes output reserve and safety; includes sent input and instruction/tool overhead; actual usage must already be provider-normalized, not input plus cache counters again |
| Selection | `history_compaction.go:74–118,860–930` | preserve initial user framing; try recent counts descending from min(configured,len) to 1; preserve both partners; first plan under target wins, else smallest shrinking plan |
| Initial exclusions | `history_compaction.go:269–291` | empty, `[SYSTEM]`, `[PHASE TRANSITION`, and `[COMPACTION CARRY-FORWARD]` messages consume no initial-user slot |
| Summary degradation | `history_compaction.go:889–927` | full summary; terse if full is not smaller than removed history; terse again if target exceeded; minimal marker if still over target; reject if after ≥ before |
| Pair protection | `history_compaction.go:1017–1062` | last matching nonempty call/result ID indexed; add partner of each initially protected item |
| Summary order | `history_compaction.go:937–982` | summary inserted at first removed item; defer behind first preserved user if otherwise assistant summary would be first |
| Re-compaction | `history_compaction.go:308–411,438–549` | merge latest prior summary highlights, omit its old timeline, retain new timeline; terse includes up to 600 prior-summary scalars; minimal fallback may still erase previous highlights (intentional pinned behavior) |
| Finalization | `runner.go:3012–3053,3229–3339` | remove stale carry-forward items; repair/reorder/deduplicate pairs; preserve trailing in-flight calls and surviving calls with approval IDs in current/previous history; default has no fresh carry-forward payload |
| Forced context overflow | `runner.go:1079–1106` | force trigger 1 and target min(configured target,max(1,history estimate/2)); invoke history planner without overhead; normalization then makes target 1 because target ≥ trigger |

A summary is an assistant message beginning `[COMPACTED HISTORY SUMMARY]`.
Full summaries include scope, unique recent requests, pending-work keywords,
case-insensitively deduplicated/sorted tool names, up to 8 paths, current work, and
a timeline bounded to max(bullets×10,20) source items. Text limits are scalar-based
(160 for bullets/timeline, 180 current work), with newline-to-space replacement.
The path matcher deliberately follows the Go regex's alternative order: e.g.
`.tsx` can be matched as `.ts`; correcting that would break pinned parity.

The target is a best-effort goal, not an invariant: preserved framing, paired
recent context, and request overhead can keep the smallest result above target.
No-removable and ineffective-summary results preserve original history and do
not justify retrying a failed model call.

### Cache and history invariants

Go `runner.go:426–437` selects the logical key (`PromptCacheKey`, task ID, or
`run`) and namespace (configured namespace or trace ID) once. `runner.go:2524–2527`
hashes `namespace + NUL + logical` using SHA-256. Both compaction requests and
subsequent normal requests reuse this key (`runner.go:777,874,1048`). There is no
compaction generation suffix or cache-key rotation. Instructions and tool schemas
are request overhead, not summary content. Keep the Rust cache prefix,
instructions, namespace, wire key and settings stable across compaction.

Only working history is replaced. The Go runner's accumulated emitted items
(`allItems`) and usage are not reset by local compaction. The Rust integration
must leave `RunResult.new_items` append-only, and preserve cumulative usage/cost.
Do not append synthetic summary messages to `new_items` or account them as model
usage. Transient request-only context must not become persisted summary content.

## Integration API

Expose `pub mod compaction;` in `lib.rs` (the coordinating change owns wiring).

- `LocalCompactionPolicy::default()` — enabled local defaults. `normalized()`
  implements zero normalization. Rust fields are unsigned, so negative Go
  config values are not representable; config decoding should normalize/reject
  these at that boundary.
- `compact_for_request(history, policy, overhead: i64) -> LocalCompactionOutcome`
  — pure planner, with `history`, `before_tokens`, `after_tokens`, `changed`, and
  exact Go `reason`. Estimates include overhead. No input mutation.
- `finalize_local_history(compacted, previous) -> Vec<RunItem>` — call only after
  `changed=true`; strips stale carry-forward and repairs native tool pairs using
  previous history. Recount after finalization. Does not append fresh dynamic
  runtime state or run its guardrails; those are host/runner responsibilities.
- `estimate_history_tokens`, `estimate_string_tokens`,
  `estimate_request_overhead_tokens`, `output_reserve_tokens` — estimated budgets.
- `EstimateCalibration::default()` — retain one per run. After model response,
  call `observe(actual_prompt_tokens, sent_history, sent_request)`. Before next
  planning call, use `apply(policy)`. Do not apply calibrated values to a future
  provider-native server-side threshold.
- `LocalCompactor { policy }` implements existing `runner::Compactor` and performs
  planning + finalization. `policy.config()` constructs an existing
  `CompactionConfig`. **For this adapter, `CompactionRequest.context_tokens` is
  the estimated full request, not reported provider usage**; it infers overhead
  by subtracting the local history estimate. `target_tokens` overrides its target.
  No change is reported by returning original history, not an error.

### Approval-aware runner integration

Use these public APIs from `adk_runtime::compaction` when the runner maintains
an approval journal alongside native history:

```rust,ignore
compact_with_approvals(items, markers, policy, overhead) -> LocalCompactionOutcome
estimate_history_tokens_with_approvals(items, markers) -> u64
finalize_local_history_with_approvals(compacted, markers, previous, previous_markers)
    -> (Vec<RunItem>, Vec<ApprovalMarkerBoundary>)
```

`LocalCompactionOutcome` contains `history`, `markers`, `before_tokens`,
`after_tokens`, `changed`, and `reason`. Markers use
`adk_codec::approval::ApprovalMarkerBoundary`: `before_item` counts only native
items, `items.len()` means append, and markers at an equal boundary retain slice
order. Boundaries must be within history. Marker phase (pending/approved/denied),
JSON input, and optional `AgentRef` are retained verbatim for surviving markers.
No phase is inferred from `Approved=false`.

The shared internal Native/Approval history makes markers participate in token
estimates, recent-item counts, summary scope, and the `tool_approval <name>`
timeline. As in Go, markers are **not** automatically protected: an old marker
may be summarized away, and a recent marker alone does not protect its call.
Call/result partner protection operates in the mixed-history index space.
Finalization preserves surviving calls without outputs when an approval with
that ID exists in either previous or compacted history, regardless of phase,
and rebases marker boundaries after stale carry-forward removal and pair repair.

On `changed`, finalize using **both previous slices**, replace native working
history and the marker journal with the returned slices, then recount using the
marker-aware estimator. Do not append compacted history or summaries to native
`new_items`: that journal remains append-only execution output. Calibration
should use `observe_estimate` with the full mixed-history prompt estimate
(excluding reserve and safety). Ordinary `compact_for_request`, estimator, and
finalizer delegate with empty marker slices; `LocalCompactor` implements the
existing native-only adapter contract and cannot receive a marker journal.

For default automatic LOCAL behavior, use the pure planner independently of an
explicit custom/provider compactor: build instructions/tools/settings first,
estimate the current request (even before the first model call or when provider
usage is missing), then compact persisted history. On a successful plan, finalize,
replace working history, rebuild request input with transient context, and emit
only genuine changed-history observations. Recoverable custom-provider compactor
errors can fall through to LOCAL; parent cancellation/deadline remains fatal.
Do not erase an existing explicit compactor's usage/accounting or alter its legacy
reported-context gate as a side effect of enabling the default local policy.

Forced overflow recovery needs no separate API: construct the forced policy
above and call `compact_for_request(history, forced, 0)`. Retry only if changed,
with an explicit runner retry bound. The
`forced_trigger_one_renormalizes_target` fixture exercises the surprising target
normalization directly against Go.

Go model-specific thresholds (`run_config.go:199–254`) are an optional resolver,
not inherent in `DefaultCompactionConfig`: small/fast variants (spark/nano/mini/
lite/flash) 110,000/60,000; GPT-6 244,800/136,000; GPT-5.6 334,800/186,000;
GPT-5.5/5.4/5.3-codex/5.2-codex/5.2/5.1 360,000/200,000;
Fable 900,000/500,000; other models 180,000/100,000. The runner should apply
host-resolved thresholds for the active model before calibration when it has
such a resolver. This module does not add provider metadata resolution.

## Representation boundaries

Differential parity covers text messages with user/assistant roles, JSON-value
tool calls, text tool results, and explicit approval markers. Core `RunItem` has no Go agent name, opaque
provider compaction item, native approval item, or standalone reasoning/handoff-call/
handoff-output variant. Therefore:

- Prior summaries are recognized by assistant role + marker rather than Go's
  extra `context-summary` agent name. Ordinary assistant names normalize to
  `assistant`; arbitrary named-agent timeline text is not represented.
- JSON arguments are compared as canonical JSON values, not original raw JSON
  spacing. Go exporter canonicalizes tool input before calling the real planner.
- Rust-only system/developer messages, any non-text content (including signed
  reasoning/media), and native handoffs are protected verbatim rather than
  silently dropped. This safety extension is tested separately, not advertised
  as Go text-path differential parity. Media token costs are unknown.
- Approval markers cross the codec sidecar boundary rather than changing native
  `RunItem`; optional marker agent provenance and explicit phases survive. The Go
  exporter carries phases separately because pending and denied share wire false.
- The Go encrypted-item protection and capped estimate (20,000+8), provider blob
  growth deferral, LLM summaries, provider
  compaction, and dynamic carry-forward hooks are not implemented here.

## Reproduction and verification

```sh
python3 scripts/replay/compaction_export.py          # refresh, offline by default
python3 scripts/replay/compaction_export.py --check  # execute Go and compare all bytes
cargo test -p adk-runtime --test local_compaction
```

The Python driver verifies the complete SDK pin and a clean tracked checkout,
uses a temporary Go test overlay to access the actual private planner/finalizer,
never edits pinned sources, and runs a single explicitly selected offline test.
It allowlists environment variables, uses a temporary credential-free HOME,
and disables module downloads unless `--allow-downloads` is supplied. A seeded
case generator produces `compaction_inputs.json`; expected output comes only
from Go. Both fixture files and the provenance manifest are checked on replay.

The differential cases compare whole ordered histories, summary bytes, estimates,
normalization, reasons, and post-planner final histories—not just item counts.
They include full/terse/minimal summaries, Unicode, initial-message exclusions,
pair integrity/repair, no-op boundaries, repeated summaries, target misses,
request overhead, default-policy pressure, seeded mixed transcripts, and 24
approval cases (pending/approved/denied, marker-only history, old-call removal,
recent pair retention, same-boundary order, and post-finalization rebasing).
Additional Rust tests cover provider-free adapter usage/cost, prompt-only
calibration, native opaque-content protection, and the Go cache hash vector.
The cache test establishes the hash and local nonmutation contract; actual
runner event/usage/cache sequencing is integration behavior, not a claim made
by a planner-only test.
