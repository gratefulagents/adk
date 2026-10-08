# LLM summary observations: schema version 1

`observations.json` is a single JSON object. Names identify cases, not ordering
requirements. All arrays preserve SDK item order. Integers are exact; timeouts
are nanoseconds. No floating-point timing values occur.

## Root

| Field | Shape |
| --- | --- |
| `schemaVersion` | integer `1` |
| `provenance` | `{repository, commit, license, goVersion, sourceSHA256, harnessSHA256}`; hash objects map paths to lowercase SHA-256 hex |
| `upstreamTestsPassed` | array of exact unchanged upstream Go test names verified on every run |
| `constants` | `{maxOutputTokens, transcriptByteBudget, defaultTimeoutNanos, instructions}` |
| `truncate` | array of `{input: string, maxBytes: integer, output: StringObservation}` |
| `flatten` | array of `{name, items: ItemSpec[] or null, maxBytes, output: StringObservation}` |
| `summaries` | `SummaryCase[]`, below |
| `plans` | `PlanCase[]`, below |

## TextRecipe and ItemSpec (inputs)

A `TextRecipe` is `{prefix?: string, repeat?: string, count?: integer,
suffix?: string}`. Expand as `prefix + repeat.repeat(count) + suffix`.
Omitted strings are empty; omitted count is zero. This keeps long inputs small
without outsourcing oracle behavior to the consumer.

`ItemSpec` fields:

- `kind`: `message`, `reasoning`, `toolCall`, `toolOutput`, `handoffCall`,
  `handoffOutput`, `approval`, `compaction`, or `unknown` (SDK enum value 99).
- `agent?`: agent name. Omission means a nil agent, hence user-role messages.
- `missing?`: when true, all payload pointers are nil, retaining type and agent.
- `text`: TextRecipe for message/reasoning text, tool output content, or encrypted
  provider-compaction content. Present as `{}` when unused.
- `input`: TextRecipe for tool-call input **bytes**. Do not parse or reserialize
  as JSON: the flatten helper accepts and renders the raw bytes, and limit
  vectors intentionally include non-JSON text. Present as `{}` when unused.
- `name?`, `id?`, `isError?`: tool name, call ID, error flag. Omitted values are
  empty/false. For tool outputs, `id` maps to `CallID`.
- Handoff payloads use `fromAgent="a"`, `toAgent="b"`; approvals use only
  `ToolName=name`. Other payload fields use SDK defaults. Unknown has no payload.

A null input array corresponds to a nil Go slice; an empty array is non-nil empty.
Both flatten identically.

## StringObservation and ItemObservation (outputs)

`StringObservation` is `{bytes, runes, validUTF8, sha256, text? , head?, tail?}`.
The hash and byte count refer to raw UTF-8 bytes; `runes` counts Unicode scalars.
For strings up to 18,000 bytes, `text` contains the complete output. Larger
strings omit `text` and include `head` and `tail`, each 128 Unicode scalars.
Compare complete outputs via byte count and SHA-256 for those large cases;
head/tail are diagnostics, **not** replacements for full hash comparison.

`ItemObservation` is an SDK item projection:

- `type`: numeric SDK enum (`message=0`, `toolCall=1`, `toolOutput=2`,
  `handoffCall=3`, `handoffOutput=4`, `reasoning=5`, `approval=6`, `compaction=7`).
- `agent`: name or null.
- Payload when present: `message: StringObservation`,
  `reasoning: StringObservation`,
  `toolCall: {id, name, input: StringObservation}`, or
  `toolOutput: {callID, isError, content: StringObservation}`.

All observed request/rebuilt/deterministic items fall within these projected
payload types. Null output item arrays represent nil Go slices, notably rejected
plan application.

## CapturedRequest and Usage

`CapturedRequest` records `model`, `promptCacheKey`, `instructions`,
`input: ItemObservation[]`, `toolsNil`, `toolCount`, `settings`, `outputSchema`,
and `compactionThreshold`. `settings` uses SDK JSON names and omits zero/nil
settings; the probe also asserts the complete struct equals exactly
`ModelSettings{MaxTokens: 2048, ReasoningEffort: "low"}`.

Additional fields: `deadlinePresent`, `deadlineMatchesPolicy`,
`inheritsParentDeadline`, `configuredOrDefaultTimeoutNanos`. The last is the
chosen helper timeout before any shorter parent deadline. Actual timestamps are
not recorded. Parent `deadline` means five seconds; `expired` means an already
expired deadline; `cancelled` means an already cancelled context. The model stub
returns the context error for the latter two rather than waiting.

`Usage` uses SDK names: `requests`, `input_tokens`, `output_tokens`, and optional
`cache_read_tokens`, `cache_create_tokens`. Omitted cache counters are zero.

## SummaryCase

Inputs: `name`, `removed: ItemSpec[] or null`, `responseItems: ItemSpec[] or null`,
`responseNil: boolean`, `responseUsage: Usage`, `modelError: string`,
`nilModel: boolean`, `parentContext: string`, `timeoutNanos: integer`.
Empty model error means no provider error. `responseNil` is distinct from a
response object with nil items. For cancelled/expired parents, context error
behavior takes precedence over the scripted response.

Observed outputs: `requests: CapturedRequest[]`, `body: string`, `usage: Usage`,
`error: string` (empty on success). The exact error strings are recorded.
Zero requests establishes a pre-call failure (nil model or empty transcript).

## PlanCase

Inputs/state:

- `name`, `usesPlanner`, `config` (native SDK `CompactionConfig` JSON shape),
  `source: ItemSpec[]`, `removedIndices: integer[]`,
  `protectedIndices: integer[]` (sorted, zero based).
- `usesPlanner=true`: call the local SDK-equivalent planner using `config` and
  compare its split. Otherwise construct a plan directly from the recorded
  source and indices; `config` is not used for those manual plans.
- `deterministicItems: ItemObservation[]`, `planAfter`, `sourceTokens` capture
  the original plan. Manual plans use `RebuildWithSummary("fallback")` for their
  deterministic items. The probe verifies removed indices reconstruct the
  actual SDK `Removed` slice exactly.
- `responseBody: TextRecipe`, `responseUsage: Usage`, `modelError: boolean`,
  `nilModel: boolean`. True model error uses `"oracle provider failure"`.
  The response is one message. Application uses `oracle/model` and timeout zero.

Observed outputs:

- `accepted`, `items: ItemObservation[] or null`, `usage: Usage`,
  `requests: CapturedRequest[]`, `planUnchanged`.
- `candidateBody: StringObservation`, `candidateSummaryError: string`,
  `candidateTokens`, `candidateItems: ItemObservation[]`. A separate invocation
  of the **upstream summarizer** obtains candidate body/error, then the upstream
  `RebuildWithSummary` and estimator produce these diagnostics. This does not add
  a request to the captured application call. If summarization fails, candidate
  items are a diagnostic rebuild with the returned empty body, not an actual
  attempted replacement. The error and `accepted=false` disambiguate this.

Consumers should compare outputs, not infer acceptance from `planAfter` or the
candidate diagnostics alone. No expected summary algorithm is implemented in
the harness: all observations come from the pinned Go helpers.
