# Issue #11 verification ledger

## Current continuation evidence

### Owned model-settings merge

`snapshots::ModelSettings::merge` now matches the pinned helper's value-selection
rules across 36 independently executed cases. Explicit optional zero/false values
override; integer budgets override only when positive; nonempty strings and stop
sequences replace their base values. Rust retains independent ownership rather
than Go pointer/slice aliasing. The offline settings/routing example passes the
merged settings through the builder to an actual scripted provider request.

Fresh Rust 1.88 Linux workspace results: **1,252 passed, 0 failed, 30 ignored**.
Strict Clippy, rustdoc, formatting, the facade matrix, platform-free consumer,
20 offline scenarios, host-session example, independent reference reproduction,
18 ledger validators and source/license locks pass. The helper closes one exact
record: **241 verified / 8,650 unresolved / 251 excluded**. Full issue acceptance
remains open; unresolved records are not a count of missing features.

Separately, pushed head `dbc39c37b4b81829408b0df1c84ef1d5783f6629` has
46 successful GitHub checks, including Linux/macOS execution-backend jobs and
real PostgreSQL verification. These CI results do not establish whole-SDK
cross-platform parity or credentialed provider/remote-collector behavior.

### Snapshot field transport and execution boundaries

Independent pinned decoding/encoding now covers **54 response and 34 request
records**, including each field, typed malformed inputs, nulls, signed-width
counters, opaque raw JSON and nested settings/tool/schema values. Both new native
comparators initially failed on null array elements. The codec now preserves
Go zero-valued elements instead of rejecting or dropping them. The observations
also exposed snapshot labels defaulting to `message` and rejecting opaque strings:
`SnapshotType` now represents empty and future labels without inventing a message.
Its schema is an open string and it is cloneable rather than copyable.

This does not grant execution authority to opaque records. Native checkpoint
migration and platform transcript persistence reject unspecified/unknown/future
labels before constructing executable state; dedicated regressions verify that
boundary without model dispatch. Existing source fixture data is unchanged; the
new independent records are additive.

The ledger adds **29 field-only closures**: ten remaining response fields, the
item label, nine model-setting fields, six tool-snapshot fields and three
output-schema fields. Existing request claims now include the new record test.
No whole-decoder, settings-merge, provider-profile or executable-conversion claim
is inferred. The first evidence pass correctly failed because its selected test
command did not run the newly claimed schema regression; the command now actually
runs codec baseline and platform boundary tests rather than waiving the check.

Fresh final Rust 1.88 Linux workspace: **1,251 passed, 0 failed, 30 ignored**.
Strict Clippy, rustdoc, changed-file formatting, full facade matrix, platform-free
consumer, all 20 offline scenarios, host-session example, independent fixture
reproduction, 18 ledger validators and source/license locks pass. The overlay is
**240 verified / 8,651 unresolved / 251 excluded**. Credentialed services and
non-Linux execution remain unverified. Full issue acceptance is still open.

### Named specialist output and extraction

Successful named `AgentAsTool` calls now return final text instead of a task JSON
envelope. The shared runner adapter uses the SDK's textual result contract:
empty or parsed non-string final output becomes `(no output)` rather than an
implicit JSON serialization. `RunnerChildExecutor::with_output_extractor` receives
the full successful native `RunResult`; nonempty projections replace final text,
while empty projections preserve its default. Projection is explicitly scoped to
an executor registration and therefore applies to named and managed calls to that
registration. Child history, checkpoints and usage are not replaced.

Thirty-six independent pinned SDK calls cover empty/whitespace/raw JSON/Unicode
text and parsed objects, numbers, null and strings, each with absent, empty,
final-text and full-value extraction. Native tests compare exact outputs, callback
counts, full-result access and retained task results. A separate regression proves
failed children skip extraction and an extractor panic does not exhaust a single
scheduler slot. Existing timeout/steering/guardrail/durable child tests remain
applicable; native pending and failed-task envelopes are intentionally preserved.
The offline named-specialist example now demonstrates explicit extraction.

Fresh final Rust 1.88 Linux workspace: **1,248 passed, 0 failed, 30 ignored**.
Strict Clippy, rustdoc, changed-file formatting, the feature matrix, standalone
consumer, all 20 scenarios, host-session example, independent fixture reproduction,
18 evidence validators and source/license locks pass. The ledger now closes only
one additional exact obligation: registered default child-name selection, backed
by eleven pinned helper observations and actual native dispatch. Totals:
**211 verified / 8,680 unresolved / 251 excluded**. The output/extractor fixtures
are not a claim of complete child-engine or provider parity. Live credentials and
non-Linux execution remain unverified.

### Automatic owned child assembly

`Builder::subagent_host` now assembles child runners and an owned scheduler from
catalog roles, with optional generic fallback. Existing schedulers take precedence;
borrowed sessions cannot be mutated into scheduler owners. Role access ceilings,
mode concurrency/turn limits, filtered delegation guidance and parent-only output
schemas are preserved. Child configurations omit parent admission/session state
to avoid ownership cycles. Explicit close and drop cancel active children, including
when handles escape the bundle; construction failures release owned resources.

Twelve independently executed pinned SDK assembly observations compare tool-group
selection, generic fallback, roles, handoffs and child calls. Six native regressions
cover those observations plus security, ownership, rollback and cancellation.
The offline handoffs/subagents scenario now runs an automatically built child.
The SDK runtime discards the named tools returned by its specialist helper and
attaches managed task tools instead: automatic named/nested tool graphs are not
a missing runtime-builder requirement. Native explicit composition remains available.

An initial full run exposed a wall-clock test deadline expiring during planning,
before the summary model could be dispatched. The test now constructs its runner
and request before arming a one-second deadline; all original cancellation and
request-count assertions remain. Fresh final Rust 1.88 Linux verification:
**1,246 passed, 0 failed, 30 ignored**; strict Clippy, rustdoc, formatting, full
facade feature matrix, standalone consumer, all 20 offline scenarios and host-session
example pass. Independent fixture reproduction, hash-bound evidence, 18 ledger
validator tests and immutable source/license locks pass. No new ledger IDs are
claimed: **210 verified / 8,681 unresolved / 251 excluded**. Live credentials and
non-Linux execution remain unverified; this is not issue-wide completion.

### Builder structured-output composition

The facade builder now forwards the runtime's typed output schema, schema name,
strict flag and custom parser through `Config`. These apply only to the parent;
catalog and generic handoff targets retain their independent output contracts.
The existing `structured_output` offline example now uses the builder and closes
its owned bundle. The standalone consumer compiles the schema fields without
requiring a direct schema-library import. No new crate version was introduced;
`schemars` is an existing workspace dependency enabled on the facade's builder
feature only.

Fourteen pinned SDK builder/runner observations cover absent, object and boolean
schemas, blank names, strict/non-strict behavior, invalid JSON, custom parsing
and parser failure in normal and streamed runs. Native tests compare parent-only
placement, request fields, parser call counts and parsed final values. Invalid
schema construction also verifies owned-session cleanup without model dispatch;
tool-only builds ignore agent output schemas. Native schema-validation diagnostics
remain deliberately stronger than the SDK's JSON-syntax fallback; no diagnostic
or provider wire-format equivalence is asserted.

The first full run and isolated follow-up exposed an unchanged paused-clock
subagent timeout test racing wall-clock deadline construction against a 50ms
virtual child delay. Its child now waits on an explicit release signal until
after the pending-result assertions. No production timeout behavior changed.
The isolated regression and all 23 child integration tests then passed.

Fresh final Rust 1.88 Linux workspace: **1,240 passed, 0 failed, 30 ignored**.
Strict Clippy, documentation, formatting, the complete facade feature matrix,
platform-free standalone consumer, all twenty offline feature scenarios and
host-session example pass. Independent pinned fixtures, hash-bound evidence,
source locks and all 18 ledger-validator tests pass. The ledger remains
**210 verified / 8,681 unresolved / 251 excluded**. This is not full issue closure;
automatic specialist/scheduler assembly and other acceptance work remain open.

### Managed subagent default selection

The builder no longer routes omitted `agent_name` to the parent's identity. It
uses immutable scheduler registrations: exact `agent` first, otherwise lexical
first nonblank name, or an empty default for an empty eligible catalog. Eleven
independent executions of the pinned `defaultAsyncSubAgent` helper are recorded
in the disposable runtime reference fixture. Native managed-tool/scheduler tests
compare the selected names and completed task records, explicit overrides and
empty-catalog rejection; the single-worker case failed before the correction.
Borrowed session ownership is preserved. This is a bounded default-selection
claim, not automatic scheduler allocation or complete SDK task-tool parity.

Fresh Rust 1.88 Linux workspace: **1,238 passed, 0 failed, 30 ignored**. Strict
Clippy, all-feature documentation, formatting, independent fixture reproduction,
hash-bound evidence, all 18 ledger validators and source locks pass. No new
capability IDs are closed; issue #11 remains incomplete.

### Metadata lookup cancellation and cached reads

Twelve independent observations from the unchanged pinned SDK now cover cancelled
and expired contexts through cold lookup, failed-fetch cooldown, retry, warm hit,
warm miss and subsequent active lookup. The new native regression failed before
the fix because public lookup rejected a cancelled context before consulting the
cache. Public `MetadataCompactionResolver::lookup` now serves cached results
regardless of context state and treats cancelled/expired fetches as misses with
the existing 30-second cooldown. The runner-facing `thresholds` adapter retains
pre-dispatch checks, cancellable waiting, the 15-second bound and cancellation
errors; it does not adopt the public raw-lookup cancellation semantics.

All eight metadata tests pass, including the twelve fixture observations, cache
isolation, shared fetches, timeout and runner cancellation. Fresh Rust 1.88 Linux
workspace verification: **1,237 passed, 0 failed, 30 ignored**. Strict Clippy and
all-feature documentation builds pass. The platform-free standalone consumer,
all 20 offline feature scenarios, owned host-session example, pinned fixture
reproduction, source locks and all 18 ledger-validator tests pass. This bounded
correction does not establish whole resolver/builder parity or close additional
capability IDs. Credentialed
live providers and non-Linux execution remain unverified. Issue #11 is incomplete;
PR33 must not merge; the CLI/evaluation/Terminal-Bench exclusions are unchanged.

### Model-written compaction summaries

The runner now implements the SDK-default model-summary phase after successful
local planning, in both normal and forced-overflow paths. It uses the actual
active binding (including same-name fallback models), records returned usage and
cost even for rejected summaries, and preserves approval boundaries, historical
provenance, append-only outputs and transient-context isolation. Explicit
`use_llm_summary: false` preserves deterministic behavior without disabling
model-specific thresholds. Pure planners and `LocalCompactor` remain provider-free.

The independently executed pinned oracle records 52 byte-truncation, 44
transcript, 18 summary-call and nine plan observations; 12 unchanged upstream
tests also pass. Native fixture comparisons cover all truncation cases, 39
representable transcripts, 13 pure request/body cases and all nine plans.
Runtime regressions separately cover successful/empty/error/oversized summaries,
usage and cost, default/explicit timeout, cancellation, deadline, streaming,
fallback selection, disabled/no-op behavior and budget exhaustion. Five transcript
cases require arbitrary non-JSON tool-input bytes and remain source-only because
native tool arguments are JSON values. This is not whole-helper parity closure.

Only explicitly attributed `context-summary` messages get the longer retention
limit; a marker cannot upgrade unknown/unattributed authorship. Summary failures
produce redacted host diagnostics rather than raw provider-error logs. Summary
calls intentionally remain outside ordinary generation spans and turn counts,
matching the SDK's out-of-band behavior. The summary setting is unconditionally
fingerprinted; legacy deterministic-default native checkpoints are rejected
rather than silently acquiring additional model calls on resume.

Fresh Rust 1.88/Linux workspace verification: **1,236 passed, 0 failed, 30
ignored**. Strict Clippy, Rustfmt and all-feature documentation builds pass.
The feature matrix and platform-free standalone consumer pass; all 20 feature
scenarios and the owned host-session example pass offline. Source locks, refreshed
independent fixture/evidence checks and all 18 ledger-validator tests pass.
Independent read-only review found no blocking issue; its local check launcher
was environment-blocked, while the parent successfully ran checks with the direct
compiler/Clippy binaries. No additional acceptance-ID closure is claimed here:
the ledger remains **210 verified / 8,681 unresolved / 251 excluded**. Live
provider credentials and non-Linux targets remain unverified; issue #11 and draft
PR #33 are not complete or ready for release.

### Static compaction model defaults

The pinned public helper now supplies 45 independently executed observations,
including all 24 upstream regression inputs. These cover budget branches,
small-model precedence, provider-prefix splitting and Unicode casing. The new
native comparator initially failed for `gpt-5.6-MİNI`: Rust's expanding lowercase
did not select the SDK's conservative small-model budget. Static selection now
uses the same Unicode-15 simple casing as metadata lookup, and all cases pass.
The unchanged upstream `TestCompactionDefaultsForModel` also passes freshly.

Only the static helper's four identities are closed: `SDK-FB58C46A3648E533`,
`SDK-06D324BF5C65AF00`, `SDK-33021DA02BCEB247`, and `SDK-71470C3083345121`.
Rust exposes `LocalCompactionPolicy::for_model` and its threshold fields rather
than a mutable Go function alias; custom defaults use explicit host resolver
injection. No additional metadata lifecycle or complete compaction-policy
closure is asserted. The ledger is **210 verified / 8,681 unresolved / 251
excluded**; the immutable baseline inventories are unchanged.

Fresh full workspace verification: **1,220 passed, 0 failed, 30 ignored**;
the focused followup suite passes all **14 tests**. Strict Clippy, Rustfmt and
all-feature documentation checks pass on Rust 1.88 / Linux x86_64. Live provider
credentials and other targets remain unverified. The feature matrix and external
standalone consumer pass, including the platform-free dependency check. All 20
feature scenarios and the owned host-session example pass offline; source locks
and all 18 ledger-validator tests pass. Issue #11 remains incomplete.

### Explicit-session metadata compaction

The provider runtime adapter now lazily loads an authenticated catalog, shares
concurrent fetches, caches successes, and retries failures after a 30-second
cooldown. Builder injection is explicit; host resolvers take precedence and the
compaction feature gates metadata access. Lookup is bounded to 15 seconds with
static threshold fallback, while caller cancellation remains an error. Mutable
catalogs intentionally do not provide a durable replay key. Diagnostics are
structured/redacted for the embedding host rather than ambient stderr logging.

Nine independent pinned SDK lookup observations plus a failed-fetch/cooldown
sequence reproduce byte-for-byte. Native tests also cover concurrency, separate
session scopes, timeout/cancellation, zero construction I/O and four builder
selection combinations. The fixtures caught an initial test catalog using the
plain OpenAI shape instead of Codex metadata; the corrected oracle uses `models`
and `slug`. Standalone consumers now have public cached `lookup` returning raw metadata;
raw misses/fetch failures and runtime static fallback remain distinct. The
exhaustive scalar regression first failed under Rust's newer Unicode table, then
passed after preserving Unicode-15-unassigned characters. All **1,112,064** scalar
lookup keys match the independent pinned Go digest on Rust 1.88.

The catalog decoder now uses separate Codex and OpenAI/Copilot schemas. Eighteen
executed catalog fixtures exposed and now cover foreign-field leakage, wrong ID
keys, null records/labels, Unicode picker ordering, signed numeric metadata and
JSON integer `-0`. Raw signed limits are retained; threshold arithmetic follows
the pinned 64-bit SDK, including overflow. A separate 219-case direct-helper grid
includes the exact three upstream regression inputs and signed boundary branches.
Those upstream Go tests also execute freshly in the reference harness.

Five ledger IDs are closed only for `CompactionDefaultsFromModelMetadata` and its
three regressions: `SDK-138AC0F3C11197C5`, `SDK-254D8EF33463B127`,
`SDK-2C743BBA75D3D99D`, `SDK-7228DB6E1027BBFE`, and `SDK-7EE10C4D8D92C468`.
This is not a closure claim for the complete catalog parser or resolver lifecycle.

Fresh workspace verification: **1,220 passed, 0 failed, 30 ignored**. Strict
Clippy, documentation build, source locks and the 18 ledger-validator tests pass.
The facade feature matrix and standalone consumer pass; all 20 feature examples
and the owned host-session example pass offline. The example script's cargo-clippy
launcher cannot find `/proc/self/exe` here, so its equivalent checks were executed
with the direct Clippy driver, followed by the same offline build/run commands.
The ledger is **206 verified / 8,685 unresolved / 251 excluded**. No broad
provider/runtime acceptance entry is closed by this bounded continuation, and
credential-dependent live tests remain unverified.

### Host model-compaction resolver

The native async resolver API and builder gate now match 48 pinned request/call
observations for feature selection, misses, zero/valid trigger values, target
normalization, single/continuing runs and complete/streamed providers. Four
additional executed cases verify retry/fallback model names and callback counts.
The first fallback comparison failed because the Go fixture used the single-model
constructor (which does not switch models); it now uses an explicit recording
provider, matching the native named-model graph rather than changing expected
names by hand. Native regressions additionally cover disabled local compaction,
shared custom/local resolution, cancellation, host failure, and durable key
requirements/change rejection/terminal non-replay. The explicit-session provider
metadata continuation above adds fetch/cache composition without implicit route
credential discovery. No additional ledger closure is asserted.

### Explicit builder compaction-policy precedence

`Config.local_compaction` is an optional typed host override, distinct from
feature defaults and legacy `runner_config` customization. Sixteen additional
pinned normal/streamed requests cover feature on/off with absent, disabled,
enabled custom and explicitly default-valued policies. The latter use input
between fixed and model-specific thresholds to distinguish fixed from automatic
resolution. All 48 compacted/uncompacted request digest arrays match the SDK.
Both fixed and automatic choices are bound to durable resume identity. No whole
LLM-summary/provider-metadata resolver parity or new ledger closure is claimed.

### Default builder carry-forward callback

Full builds install deterministic configured-provider/mode carry-forward text
only when the host has not supplied a callback. The expanded fixture now checks
32 normal/streamed compacted requests, default versus explicit-blank callbacks,
default versus custom policies, four provider/model combinations, display-name
whitespace/Unicode and fallback-name labels. Every compacted message digest
matches independent pinned SDK execution. Static working-state fallback tests
explicitly provide a blank callback, matching the SDK's precedence contract;
neither priming nor static text silently displaces a nonblank callback.
The bounded static-field mapping `SDK-94B3809F696E07F6`
(`RunConfig.WorkingStateContext`) is verified from these executed requests and
native carry-forward regressions; whole compaction/config parity is not claimed.

### Builder local-compaction defaults

Default full-builder policies use ten recent items/five summary bullets while
resolving thresholds per active model; explicit nondefault host policies retain
their settings. Sixteen actual pinned normal/streamed requests across four model
families and default/custom policies compare every compacted message by SHA-256.
The initial sixteen-case harness used an explicitly blank carry callback to
isolate policy/static fallback behavior. Its first comparison revealed the SDK
builder's default provider/mode carry callback, subsequently implemented and
covered above. No new ledger IDs closed.

### Durable local-compaction identity

Durable resumes now bind all six normalized local-compaction policy fields.
The regression first failed because changing the enabled flag was accepted;
after the fix, a seven-policy cross-product rejects every changed policy before
model dispatch and accepts identical or normalization-equivalent policies.
Default-policy identities remain unchanged. Previously written nondefault-policy
checkpoints did not bind these settings and must not be treated as proof of the
original compaction configuration; no automatic migration is supplied. This is
a native recovery correction, not an additional SDK ledger parity claim.

### Builder working-state fallback

Full builds now select the SDK host-adapter working-state fallback only after
project-state priming, when no nonblank text remains. Sixteen pinned builder
observations include blank/explicit/Unicode state; native normal and streaming
tests verify carry-forward after local compaction and absence before compaction.
Direct runtime defaults and provider-free tool construction are unchanged. No
new ledger closure is claimed by this bounded composition fix.

### Connected MCP prompt metadata

`AgentConfig::mcp_servers` now feeds request context after additional/schema
sections. Full composition copies only connected server names after MCP assembly;
handoffs use target metadata and durable identities bind nonempty lists. Unicode
classification is pinned to the SDK's Unicode 15.0 rather than compiler tables.
Fifty-six independently executed SDK requests and a digest covering all
**1,112,064 valid Unicode scalars** match native output. This closes only
`SDK-9BC0FD01C9F477BB` for native UTF-8 names, not whole MCP behavior or a general
prompt-injection security guarantee. The dependency's Apache-2.0 license and
versioned source are recorded in the research and fixture notice.

Focused tests first caught metadata copied before connection assembly; that
ordering was corrected before the fresh passing full run. The new source helper
claim initially failed the ledger gate because its original SDK regression was
not in the reference report. Executing all three pinned MCP-prompt source tests
added real pass records (746 passing source test/subtest records, 5 skipped),
after which overlay generation and all eighteen validators passed. No baseline
inventory or source-lock files changed. Current local logs are under
`/workspace/scratch/issue11-mcp-prompt-*`.

### Run-wide instruction composition

The runtime now applies `additional_instructions` to each active agent request,
including handoffs, before structured-output context. The builder maps feature
summary, mode directives and final-check instructions without mutating agent
instructions. Forty actual normal/streamed pinned SDK request observations match
native tests, including all eight builder combinations. Durable regression
coverage rejects a changed run-wide instruction value while accepting the
unchanged configuration. No deprecated-field fallback or MCP prompt claim is
made, and no ledger entries are newly closed by this slice.

The full suite, direct strict Clippy, rustdoc and feature matrix passed. The
example script's cargo-clippy launcher failed because this worker lacks
`/proc/self/exe`; equivalent direct-driver lint/build plus all twenty scenarios
and host-session execution passed. The first evidence invocation omitted
`GOROOT` and failed in its Go fixture launcher; the corrected explicit Go
installation environment passed all fixtures and eighteen ledger validators.
Failure logs were retained, rather than counted as successful checks.

### Owned composition and workspace prompts

Full builds now own host-authorized MCP sessions and project-state composition;
`Builder::build_tools` provides a provider-free tool-runtime owner using the same
resource assembly. Four project-state switches independently select priming,
task tools, memory tools and the prime tool. Injected stores take precedence over
automatic filesystem construction with explicit host paths. Blocking operations
retain drain leases across cancelled callers; close/drop revokes escaped handles.
Independent pinned SDK execution covers sixteen full/tool-only project-state
selections and twelve MCP selections. Project-state priming populates compaction
carry-forward state, not initial instructions.

The missing parent workspace prompt block is now composed after policy-prepared
tools and delegation guidance. Sixty independently executed SDK workspace cases
and ten mode-label cases compare strict/legacy text, whitespace, Unicode paths,
access labels and tool ordering. Full parent instructions now include the SDK's
legacy `Active mode: chat` default and `Active mode:` label spelling; snapshot
display-name/name/active-mode precedence is independently checked.
Native normal/streaming request tests preserve the block, and policy regressions
exclude denied/mutating tools in read-only mode. Legacy fixed tool/path prose is
compatibility guidance, not actual execution authority; strict selection names
only prepared tools. Non-UTF8 native full-builder paths fail explicitly.

Latest local verification: **1,201 workspace tests passed, 0 failed, 30 ignored**;
strict all-target Clippy, rustdoc, formatting, source locks, facade matrix,
platform-free external consumer, twenty offline examples plus host session and
**18 evidence-validator tests** passed. The immutable inventory is untouched.
The ledger is **200 verified / 8,691 unresolved / 251 excluded**. Only the
exhaustive project-state needs-store predicate, bounded legacy workspace formatter
and UTF-8 MCP prompt formatter were newly closed; no whole-builder or whole-SDK
claim is made.

An earlier separate project-state evidence attempt hit the unchanged paused-time
`tool_policy_timeout_preserves_managed_pending_results` regression: a child
completed before its expected timeout observation. Its failed log was retained;
subsequent unchanged full and sequential evidence runs passed. It was not waived
or weakened. Current-head CI and credentialed live provider/OAuth/collector tests
are separate evidence; unavailable live credentials remain **unverified**.

Older checkpoint summaries below are historical, not current aggregate counts.

### Opt-in built-in tool guardrails

The public facade now assembles built-in input/output tool guards before custom
rules, with default-off legacy and explicit feature selection and stable durable
identities. Shared security primitives avoid duplicate shell policy and secret
signature tables. Standalone tokens are redacted; compound credential markers
block the output. Reasoning content is scanned before runtime conversion to text;
redaction requiring loss of non-text structure blocks instead.

The pinned public SDK guard functions generate **87 independent cases** twice,
byte-for-byte. Native comparisons cover **75 representable cases**, excluding
11 malformed raw JSON cases and one escaped-marker case where decoded native
scanning is intentionally stronger. Guard order, tripwires, replacement presence
and exact redacted content are compared; parser error prose and general shell
language equivalence are not claimed. Bounded native shell parsing rejects
unsupported control/indirect heads and is not an execution sandbox.

Security review found reasoning-content leakage, multipart scan/render boundary
mismatches, unsupported shell heads and `env` split-string bypasses. These were
corrected, with actual Runner regressions checking requests, history, reports,
events and observations, plus shell classification regressions. Multipart
credential detections block output; unsupported split-string forms fail closed.
Final local verification: **1,141 workspace/doctests passed, 0 failed, 30 ignored**;
**579 hash-bound tests** and **17 evidence validators** passed. Formatting,
strict all-target lint, rustdoc, source locks, facade matrix, standalone consumer,
and twenty offline examples plus host session passed. The guardrail example now
exercises public built-in redaction and destructive-input rejection without
executing the destructive command. All seven handoff-module fixtures regenerate
twice. Ledger remains **197 verified / 8,694 unresolved / 251 excluded**;
no aggregate parity claim is inferred from helper availability. Focused independent
re-review confirmed both follow-up bypasses resolved by code inspection; test
execution above is parent-run evidence. Live credentialed provider behavior
remains unverified. Previous head `b495040` finished with **25 successful and
21 cancelled CI jobs**, including one successful Linux and one successful macOS
enforcement lane; cancelled jobs are not passes. New-head CI is separate evidence.

### Immediate-input wake signals

Optional host-owned wake signals now replace pending attempts before native
model-event publication. Once an event has been published, the stream continues
without replay; queued input waits for polling/finalization. Signals require a
poller, follow the builder's explicit feature gate, and bind durable identity.
Superseded root attempts gain a replacement turn while physical metrics and
child/shared security budgets remain charged. Generation cleanup is exactly
once. Restore now accepts legitimate finalizer/signal-created turn extensions.

Eight independently executed SDK cases compare pending-attempt cancellation,
one-turn replacement, text/reasoning publication, tool-boundary and finalizer
admission. The source oracle uses channel handshakes and passed race-detector
testing for 100 repetitions. Rust compares exact projected input, callbacks,
histories/new items, output, deltas and accepted-response counts. Raw SDK event
formats, retry-advice call counts and late returned-response billing are not
claimed equivalent. Native completed-item/tool-argument/complete events also
commit an attempt because they are public to the host. Independent review found
no actionable regression; limitations remain explicit in the builder guide.

Verification: **1,131 workspace/doctests passed, 0 failed, 30 ignored**;
**569 hash-bound tests** and **16 evidence validators** passed. All six pinned
handoff-module fixtures regenerate twice. Formatting, strict all-target Clippy,
rustdoc, source locks, facade matrix, standalone consumer and twenty offline
examples plus host session passed. Ledger remains **197 verified / 8,694
unresolved / 251 excluded**; no aggregate entry was closed solely because a
feature flag now exists. Live credentialed provider tests remain unverified.

### Immediate input at boundaries

Native runtime polling and atomic finalization now accept provenance-aware
batches, preserve normal/streaming ordering, and support late-input budget
extension. Explicit builder selection gates both host callbacks; legacy mode
retains them. Poll errors are best-effort observations; finalizer errors abort.
The observability pipeline preserves error capture policy and keeps polling
failures nonterminal. Fourteen independent pinned SDK cases compare complete
projected requests, histories, new items, actual authorship, callback/tool counts
and outputs. All five handoff-module fixtures regenerate twice byte-for-byte.

Review found that child steering could remain closed after a late-input
continuation. Transactional reopening now rejects cancellation and non-running
children; a Notify-gated durable/non-durable regression verifies subsequent
parent steering, exactly-once application and final closure. Malformed batches
are validated before mutating history. Cancellation/deadline and failed
publication regressions preserve ownership and admitted input.

Durable callbacks require stable keys. A dispatched-admission checkpoint is
explicitly non-replayable without host reconciliation; no crash-atomic external
queue admission is claimed. Wake-before-visible-output `ImmediateInputSignal`
remains missing, not silently treated as equivalent to boundary polling.

Final verification after the review fix: **1,123 workspace/doctests passed,
0 failed, 30 ignored**; **561 hash-bound tests** and **15 evidence validators**
passed. Strict all-target Clippy, Rustfmt, rustdoc, source locks, facade matrix
and the standalone platform-free consumer passed. All twenty offline feature
examples, the host-session example, and Go test/vet/build passed. No additional ledger entries
were closed: **197 verified / 8,694 unresolved / 251 excluded**. Live
credentialed provider tests remain unverified.

Previous head `84b3200` finished with **28 successful and 18 cancelled CI jobs**
(not 46 passes). Both Linux and both macOS enforcement jobs passed, including
the PID-readiness correction; one Windows unsupported-target lane passed and
the other was cancelled. New-head CI is separate evidence.

### Final-summary selection and lifecycle readiness

The builder and direct runner now expose opt-in final-summary attempts. Exact
request instructions, tool names, tool-call counts and outputs match eight
independently executed pinned SDK normal/streaming cases. Native tests separately
cover retry attempts, pending-child joins, denial of unexpected tool/handoff
calls without approval, snapshot/provenance alignment and durable recovery.
These additional cases do not claim independent SDK equivalence. Retry budgets
remain native attempt budgets. The feature defaults off and preserves default
durable fingerprints.

Verification: **1,108 workspace/doctests passed, 0 failed, 30 ignored**;
**524 hash-bound evidence tests** and **14 evidence validators** passed. Go
test/vet/build, twice-identical oracle regeneration, source locks, strict Clippy,
Rustfmt, rustdoc, facade feature matrix, standalone consumer, twenty offline
examples and host-session example passed. No additional aggregate ledger entries
are closed: **197 verified / 8,694 unresolved / 251 excluded**.

Head `150ca251` finished CI with **45 passed / 1 failed**. One macOS lifecycle
lane failed parsing an empty PID file; the other macOS lane passed. Shell
redirection creates the file before `echo` writes the PID. The test readiness
helper now waits for a complete newline-terminated record, with a regression for
empty and partial writes. All **25 lifecycle tests** and scoped Clippy passed
locally after this test-only correction; the full-suite total above precedes
that one added regression. macOS validation of the correction requires new CI.
No enforcement or process-reaping assertion was removed. Live credentialed
provider tests remain unverified; full issue acceptance is not claimed.

### Independent managed-selection proof

The managed selection matrix now compares against actual pinned public SDK
`runtime.NewBuilder(...).Build(...)` output for all eight boolean combinations,
not handwritten expected tool names. The independent generator records original
SDK ordering and scheduler presence, closes owned resources, and rejects any
provider request through a loopback trap. Rust compares tool-name sets; automatic
SDK allocation and differing tool order remain outside the native ownership
claim. All three handoff-module fixtures regenerate twice byte-for-byte; Go
test/vet/build, scoped strict Clippy, **497 hash-bound Rust tests** and
**13 evidence validators** pass.

Only `AsyncSubAgentFeatures.any` (`SDK-6866B794E8FC3833`) is newly closed: its
three-boolean truth table is exhausted. Full runtime/features/builder entries
remain unresolved. The ledger is **197 verified / 8,694 unresolved / 251 excluded**.
The validator requires a selection-specific pinned execution marker. Corrected
implementation head `8feca21` has passed both Linux and macOS enforcing lanes and
Windows unsupported-target lanes; those are actual CI observations, distinct
from the worker's local fail-closed checks. Full issue acceptance is not claimed.

### Managed subagent selection and CI correction

`Features::subagents` now exposes independent Task, Status and Control groups via
`SubagentFeatures`; all eight combinations have native composition coverage,
including scheduler requirements, disabled unrelated extras, specialist exclusion,
shared-session ownership and host allow/deny restrictions. This is source-inspected
mapping to the pinned `asyncSubAgentToolNames` helper, not a new independent Go
oracle or an aggregate ledger closure. Migration is documented in the builder
API guide. That guide also corrects stale statements about the already-supported
mode constraints.

The prior checkpoint's final CI result was **42 passed / 4 failed**, with failures
in the Linux/macOS push and PR enforcement lanes. Inspected Linux and macOS logs
show the new role-view safe-command assertion failing: actual output was
`role-view`, but the test incorrectly required `Exit code: 0`. `shell_session::bash_result` appends exit status only for nonzero
exits. The assertion now checks exact stdout; mutation-side-effect and nonzero
failure assertions remain mandatory. Successful native enforcement must still be
confirmed by the corrected head's CI, not inferred from this worker's fail-closed
execution.

Fresh local verification: **1,102 workspace/doctests passed, 0 failed, 30 ignored**;
**497 hash-bound evidence tests** and **12 validators** pass. Strict all-target
Clippy, Rustfmt, rustdoc, facade matrix, standalone consumer, twenty offline
scenarios and host example pass. One earlier full run failed the unchanged
`tool_policy_timeout_preserves_managed_pending_results` and
`nested_tool_policy_timeout_resumes_without_cancelling_children` assertions;
all 21 integration tests passed in isolation and the subsequent full run passed
without competing builds. These intermittent failures are retained here, not
silently discarded. Source pins and ledger counts are unchanged. Full acceptance
and the previously stated live/review limitations remain unresolved.

### Catalog-handoff checkpoint

The builder now composes explicit catalog handoffs, independent of managed
subagents, with shared provider routes, per-role routing/access, owner-bound tool
views and explicit bundle/session lifetimes. Runtime history filtering preserves
audit records and provenance while clearing forwarded approval anchors. A
regression verifies that a failing history-replacement hook cannot return a
journal/history mismatch. See [composition boundaries](../handoff-composition.md)
and [builder documentation](../runtime-builder.md).

Fresh Rust 1.88.0/Linux x86_64 verification: **1,100 passed, 0 failed, 30 ignored**
across workspace tests and doctests; strict workspace Clippy, Rustfmt, rustdoc,
facade matrix, platform-free standalone consumer, **20 offline feature scenarios**
and the host-session example passed. The example script's Clippy launcher still
panics because this worker lacks `/proc/self/exe`; direct-driver warning-denied
workspace/all-target checks and separate example build/runs passed. This is not a
claim that the unmodified example script exited successfully.

The independent pinned Go oracle regenerates both fixtures twice byte-for-byte:
23 filter cases (15 native-compatible, eight source-only), and 30 catalog cases
(ten compared through the documented bounded native projection, not all 30).
Go tests, vet, build and formatting pass. Hash-bound evidence runs **495 Rust
tests, 0 failed**, and **12 evidence-validator tests** pass. Source locks and the
overlay validate without changes to the immutable inventory. The ledger remains
**196 verified / 8,695 unresolved / 251 excluded**; no aggregate handoff parity
claim is closed by these bounded comparisons. Successful native OS confinement,
non-Linux targets and credential-dependent live services remain unverified in
this worker. Independent re-review of the final integrated checkpoint has not
been obtained. PR33 remains draft; issue #11 is incomplete.

### Previous fileconfig checkpoint

The fileconfig claim audit closes five bounded ledger entries: the public and
configuration records for the pure directive formatter, plus the pinned builtin,
builtin-override and traversal-rejection regressions. Exact formatter bytes and
complete builtin objects are compared against independently executed Go output.
File parsing APIs remain unresolved where strict native policies differ; the
nine source-only oracle cases are not parity passes. A subsequent direct-helper
fixture closes three more entries: standalone builtin templates and the
fileconfig guardrail/history no-ops. Those calls are verified with malformed
files, nonexistent active modes and cancelled contexts. The differential now
passes **174/174 eligible queries across 43 cases**; nine cases / 19 queries remain
explicitly excluded. The standalone `load_role_catalog(context, directory)`
facade now reuses the strict parser without requiring a source root, with direct
Go observations for ordering, isolation, missing directories and directory
errors. Native cancellation and strict duplicate/unknown-field tests are
separate; full loader parity is not claimed. Fresh Rust 1.88 workspace tests
pass **1,072 tests, 0 failed, 30 ignored**. The previous pushed head's two CI
quality failures were formatting-only; the corrected workspace formatting
check passes locally. Strict workspace Clippy, rustdoc/doctests, feature matrix,
platform-free standalone consumer, twenty offline feature scenarios and the host
session example pass. The example script's `cargo clippy` launcher hit the
container's missing `/proc/self/exe`; equivalent warning-denied checks succeeded
through the direct Clippy driver before the example build and runs.
The ledger has **196 verified, 8,695 unresolved and 251 excluded** entries.
Fresh claim verification
ran **401 Rust tests, 0 failed**, regenerated the Go fixtures twice, and passed
**11 evidence-validator tests**. The validator now requires a fileconfig-specific
pinned-oracle success marker, so unrelated host fixture evidence cannot satisfy
that requirement. This is not an issue-wide completion claim.

The historical audit below is retained; its counts and implementation gaps are
not a current completion claim. PR33 remains draft and issue #11 is incomplete.

- Both attached source checkouts were restored after runtime migration. SDK HEAD
  and fetched `v0.0.115` resolve to `1dc92b73900fac74dc357a938e4b5eee6392b418`;
  source-lock and strict inventory checks pass without baseline edits.
- Independent pinned public SDK plus selected internal guardrail and routing tests: **743 passed test/subtest records, 5
  skipped, 35 packages**. Store, schema-2 writer and typed OTel exporter fixtures
  also match independently executed pinned Go output. A fourth fixture checks
  full stdout documents and exact tab-indented bytes against the SDK-pinned Go
  OpenTelemetry exporter.
- Typed guardrails now enforce input/output/tool boundaries, explicit output
  replacement, ordered callbacks, typed tripwires, cancellation and panic
  isolation. Durable recovery fingerprints policy keys and retains reports.
  Composite hooks retain all failures while continuing ordered fanout.
- Schema-2 TraceWriter produces category records, typed spans, snapshot digests,
  instruction artifacts and health. Typed OTel mapping covers all span kinds,
  final attributes, error status, trace-ID notification and ended-parent lookup.
  Automatic generation observers now publish retry/fallback decisions, resolved
  model metadata, usage and once-estimated cost to the writer or OTel processor.
  Drop cleanup and a returned response followed by host failure are covered.
  Ordered request documents preserve raw JSON bytes for exact metadata digests.
  Representable native response snapshots are now assembled automatically and
  verified against pinned SDK bytes in full and metadata capture modes. Exact
  automatic SDK snapshot parity is **not** complete: normalized provider raw
  data remains unresolved. Representable requests now assemble automatically
  with actual attempt bindings, declared tool timeouts, approval-marker order,
  pinned token estimates and explicit historical authorship. Non-finite float
  settings (including parsed `1e309`/`-1e309`) fail at snapshot construction.
  Runtime sidecars retain attribution across retries, handoffs, local/custom
  compaction, child context, conversation ownership and native durable v2.
  Legacy native v1 restores Unknown, never fabricated current-agent names.
  Bundle run/stream variants accept explicit sidecars; malformed input fails
  before dispatch. Unknown attribution is a snapshot error, not a run failure.
  Full/metadata request capture, health reporting and digest tests pass.
- Stdout now emits Go-compatible JSON via the maintained Rust SDK batcher,
  preserving full parent context and child counts. Actual-provider tests verify
  ordering, late children, envelope-key collisions and flush errors. SDK endpoint
  constructors install globally; host-supplied exporters remain scoped.
  Pinned duration cases also verify Go's signed-duration saturation. Runtime
  generation model identities now match the pinned SDK for routing prefixes,
  empty providers/models, whitespace, nested model names and simple Unicode
  lowercasing (including dotted-I and non-contextual sigma).
- Native model responses retain optional provider data separately from metadata,
  with absent/null JSON round trips and HTTP/streaming regressions for retained
  payloads. An independent ordered `snapshot_raw` document now persists through
  native JSON/Value checkpoints and takes precedence in compatibility snapshots.
  Nine public-provider Go fixtures distinguish complete calls from streamed calls;
  two Chat completion and seven public streaming raw documents match exact bytes.
  Full complete-method profiles and response item/EndTurn parity remain open;
  these tests do **not** close complete SDK provider snapshot parity. Scoped review
  found unfinished Anthropic blocks could be accepted and malformed arguments
  could fail only during snapshot assembly. Both have executable regressions and
  fixes; source re-review found the two findings addressed, without independent
  test execution or an issue-wide approval. Request counts are now retained
  through providers, aggregation, observability, checkpoint export and Go recovery
  rather than inferred from attempts. Zero counters preserve native schema-1 bytes.
  Snapshot conversion errors remain observable in persisted writer health.
- Shared trace scopes now compose the schema-2 writer and OTel processor with
  ordered fanout, explicit root ownership, child guards and runner generation
  observers. Seven new regressions cover shared roots, late children, concurrent
  spans, actual run-future cancellation, exporter ownership and overlapping OTel
  roots. Independent scoped review found cross-root parent eviction; the fix
  retains each live root's parent contexts and has an exporter regression.
  Explicit composed flush now reaches host-owned telemetry, attempts every sink
  and aggregates all failures. Span-only processors reject unsupported flush
  rather than silently claiming exporter delivery. Two more regressions verify
  these boundaries.
- The explicit per-run adapter now produces observed agent, function, generation,
  handoff and compaction spans. Nine new tests exercise actual runner tools,
  guardrail privacy before output caps, handoffs, compaction, late generations,
  reentrant callbacks and cancellation. An owned future wrapper guarantees that
  the run future is dropped before trace cleanup; incomplete functions are marked
  interrupted. Independent review found stale no-op compaction state; an actual
  runner regression reproduced it, and superseded attempts now close without
  borrowing a later attempt's counts or parent. Scoped re-review found it fixed.
  Configured agent instructions now flow through the start observation into
  agent spans, with actual runner and handoff assertions. Resolved generation
  instructions remain separate. Schema-1 agent-start events and compatibility
  callbacks still omit instruction content; both capture modes have a regression.
  `RunTrace::run_session` now emits root-parented session summaries from native
  authoritative `RunResult::metrics`, including retry turns, compaction cost,
  partial failures and cumulative continuation accounting. Completed durable
  restore retains the saved metrics. Dropped futures fabricate no summary;
  missing legacy metrics and unrepresentable counters report processor health
  errors without changing execution. The offline trace-store example verifies
  persisted session fields. This does not establish complete ProgressTracker
  parity or exactly-once replay/billing semantics.
- Fresh Rust **1.88.0 / Linux x86_64** full workspace/all-features/all-targets:
  **1,004 passed, 0 failed, 30 ignored**. Strict Clippy and rustdoc pass. Facade
  matrix, independent all-feature consumer, twenty offline scenarios and the
  workspace doctest pass. The consumer remains transitively platform-free.
- The field audit exposed non-finite span costs becoming JSON null. Nine
  independently executed Go cases now verify matching rejection/health for
  NaN/+Inf/-Inf across session, generation and subagent records. The OTel bridge
  preserves the original Float64 attributes (including signed zero) without
  routing through JSON. Five-value actual exporter tests cover this distinction.
- Routing helpers now preserve normalized effort/verbosity labels and exact SDK
  budgets. Seventy-two independent Go combinations and five builder cases cover
  Unicode/whitespace, invalid labels, medium defaults and none/max overrides.
  Anthropic ignores string verbosity rather than rejecting a default-built agent.
  Mode/role composition and the offline routing example assert effective budgets.
- Complete and public-stream provider calls now have separate normalized snapshot
  profiles. Eight complete and seven stream cases compare full snapshot bytes,
  including items/aggregates, usage, end-turn and raw JSON. Explicit projection
  tags preserve native data and survive serialization and completed checkpoint
  recovery; untagged custom documents remain opaque. These are fixture-backed
  cases, not blanket provider parity.
- Independent record oracles verify **182 durable and 47 project-state cases**.
  Fixed-offset timestamps, fractional spelling, wide integers, unknown enum strings
  and omitted versus explicit-null metadata are preserved. The baseline generator
  no longer corrupts literal `task_ids` keys. The 80 durable and 46 project-state
  field claims do not close recovery helpers, input options or backend semantics.
  Review caught an execution-boundary bypass introduced by open enums; native and
  stored resume now reject unknown effect state/classification and stored run status
  before execution. Re-review found the safety finding fixed. Live Go checks cover
  record decoding plus 24 checkpoint restores/refusals with no unintended execution.
- The overlay now has **177 explicitly verified**, **8,714 unresolved** and
  **251 excluded** IDs. Six routing helper/API alias claims are backed by these
  executed fixtures. Eleven session claims close the nine SessionSpanData
  fields, its typed record and public alias mapping to `adk::tracewriter::Session`.
  They do not close ProgressTracker aggregation or its session producers. Three claims cover distinguishable store quota errors;
  ten cover specific OTel mapping/normalization regressions; eleven cover the
  request snapshot representation/fields; three cover span lifecycle APIs; one
  covers response EndTurn's absent/false/true representation and transport. Four
  cover guardrail diagnostic/tripwire/replacement fields and two cover exact
  input/tool-input runner regression obligations, with individually executed
  upstream counterparts and strengthened model-input/typed-cause assertions.
  Request representation claims do not cover automatic assembly; the EndTurn claim
  does not cover the whole response builder or provider raw normalization. Exact tests,
  compiler, independently executed fixture verification and input hashes are
  retained in `issue-11-rust-evidence.json`. Ten evidence-validator tests pass,
  including rejection of unbound implementation files and unexecuted fixtures.
  No blanket closure is inferred.

Remaining work includes provider cases outside the executed snapshot fixtures, full higher-level
host-session/runtime/helper composition, builder-managed scheduler configuration and the remaining acceptance-ID audit. Live providers/OAuth, collector delivery,
external Postgres/pgvector and non-Linux targets remain unverified. Fresh
independent review is still required before leaving draft status.

## Historical pre-implementation audit

## Status

This is the completed audit result, not final implementation validation. It
describes the repository at Rust revision
`489b1886aa760b6a2d4680d42bd3dce0a1397407` before concurrent builder,
observability, and feature-example changes are accepted. A passing module-level
test or a façade re-export does **not** close all baseline source obligations.

The authoritative disposition data is
[`docs/migration/ledger/issue-11-overlay.json`](../migration/ledger/issue-11-overlay.json).
It contains all **9,142** v0.0.115 acceptance IDs:

| Disposition | Count | Meaning |
|---|---:|---|
| `excluded` | 251 | Deliberately outside issue #11 scope; not implemented or verified. |
| `unresolved` | 8,891 | Requires acceptance-ID-specific implementation and semantic verification. |
| auto-verified | 0 | No module, API, test, command, or feature association is treated as automatic verification. |

Excluded records are `sdk_cli` (198), `sdk_evals`/`sdk_evals::*` (50), and the
three `.github/workflows/terminal-bench.yml` records misrouted to `sdk::ci`.
The latter IDs are `SDK-EE79D2040B1F61A2`, `SDK-A4A8B27E9715FD93`, and
`SDK-30D5117381CDA2A4`. Exclusion does not authorize implementation of the
prohibited CLI, evaluation, or Terminal-Bench contracts.

## Baseline and source-pin evidence

| Item | Result |
|---|---|
| Ledger baseline | SDK v0.0.115, `1dc92b73900fac74dc357a938e4b5eee6392b418` |
| Generated baseline files | `inventory.json` and `sources.json` manifest digests matched; all 460 archived source-file hashes matched their corresponding local checkout files during the audit (this does not make the checkout HEAD match the pin). |
| Local checkout at audit | SDK v0.0.116, `63afe2ed8cc5f13ca7469054f2c1cb812fcac801` |
| Drift | v0.0.116 adds computer-use metadata files after v0.0.115. The overlay is keyed only to the archived v0.0.115 ledger. |
| Standard validator | `python3 scripts/inventory/validate.py` stops at its checkout-pin assertion because `repos/sdk` is v0.0.116. This failure is expected evidence of drift, not a failing source-hash comparison. |

The generated v0.0.115 directory is immutable baseline evidence. The overlay
does not edit its historical `not_implemented` / `not_run` statuses, because
the baseline schema fixes those values and regeneration would overwrite direct
completion claims.

## Overlay contract and deterministic check

Every acceptance entry has the source record identity, category, proposed Rust
module, scope/disposition, empty Rust implementation and test evidence lists,
an upstream-regression reference-group key, `verification_status: not_run`,
semantic-closure state, and an approved-divergence placeholder. The root
resolves reference groups to pinned upstream test records and also records the
audit Rust revision, SDK drift, counts, and a module-level API/test association
index.

The association index is intentionally separate from semantic closure. For
example, a test in `crates/adk-tools/tests/registry.rs` can be a useful module
reference, but it cannot establish every tool's schema/default/approval/result
semantics. Only an entry with explicit evidence may change from `unresolved`.

Regenerate or check only the overlay from the repository root:

```sh
python3 scripts/issue11-ledger.py generate
python3 scripts/issue11-ledger.py check
```

The script reads the generated inventory and manifest, writes only
`docs/migration/ledger/issue-11-overlay.json`, and refuses an output path inside
`docs/migration/ledger/sdk-v0.0.115/`. `check` verifies deterministic bytes and
the audited totals: 9,142 total, 251 excluded, and 8,891 unresolved.

## Module references are not closure

The overlay's `module_reference_map` points to the current façade and test
locations for core, runtime, tools, providers, durable state, MCP, execution,
and observability. It is a navigation index, not a crosswalk from every Go
symbol to Rust behavior. The companion [facade research](../research/facade.md)
lists the associations and their feature gates.

No audit command ran live providers, OAuth, remote MCP services, or the
operating-system runtime matrix. Those remain unverified even where an offline
test file already exists.

## Implementation delivered on the issue #11 branch

This change adds working **native** APIs, not full source parity:

- `adk::builder::{Builder, Config, Features, ConfigSource, FileConfigSource,
  SessionState, SessionHandle, Bundle}`: provider/tool assembly, strict versus
  legacy selection, YAML/Markdown mode/role overrides, read-only narrowing,
  typed resource inputs and owned/shared lifecycle. See [builder mapping](../runtime-builder.md).
- `adk::observability`: ordered awaited hooks/host events, bounded incremental
  JSONL decoding, cumulative progress, metadata-first capture, explicit full raw
  capture, private Unix trace storage and a host-owned real OTel tracer bridge.
  See [observability mapping](../observability.md).
- Twenty named, runnable [feature scenarios](../feature-examples.md), including
  a custom auditing namespace-memory backend. Directory-level coverage is not
  an assertion that every Go example entrypoint is equivalent.
- An independent consumer workspace, individual Cargo-feature CI matrix,
  original native JSONL fixture, research/licensing notes and explicit CLI exclusions.

### Local validation

Validation used **Rust 1.88.0, x86_64 Linux**, with dependencies cached before
using `--offline`. The installed rustup/cargo-clippy launchers could not resolve
`/proc/self/exe` in this worker. A directly installed pinned compiler plus an
explicit library path worked; Clippy ran using `RUSTC_WORKSPACE_WRAPPER` and
`CLIPPY_ARGS=-Dwarnings`. This is not validation on a newer compiler.

| Check | Observed result |
|---|---|
| `cargo test --offline --locked --workspace --all-features --all-targets` | **788 passed, 0 failed, 30 ignored** |
| `cargo test --offline --locked --workspace --all-features --doc` | **1 passed** |
| `builder` integration tests | **18 passed**; strict/legacy selection, unavailable tool failures, routing/overrides, policy narrowing, lifecycle/rebuilds, typed resources and isolated HOME rejection |
| `observability` tests with `otel` | **22 passed**; real in-memory OTel SDK exporter, paused continuation, ended-parent IDs, shared-run isolation, two-turn success spans, ordering/backpressure, raw outputs, redaction, private file/quota regressions |
| `event_fixtures` | **1 passed**; exact native-schema bytes and fragmented order; not a Go trace-schema fixture |
| Feature executable | **20/20 PASS**, including custom store operation assertions; no live credentials or provider calls |
| Clippy direct driver, workspace/all-features/all-targets, `-Dwarnings` | Passed |
| `cargo doc --offline --locked --workspace --all-features --no-deps`, `RUSTDOCFLAGS=-Dwarnings` | Passed |
| Rustfmt direct check over workspace and consumer sources; `git diff --check` | Passed |
| `cargo deny --locked check` | `advisories ok, bans ok, licenses ok, sources ok`; narrow hashbrown duplicate exception documents YAML/SQLite dependency generations |
| Existing offline codec harness/replay | **7 fixture cases passed**; **17 Python replay tests passed** |
| `sh scripts/check-facade-matrix.sh` with offline cache | Passed: minimal, each of 12 individual features, runtime/tools/providers composition, all features, independent consumer and transitive purity check |
| Independent consumer workspace | Compiled and ran with every reusable facade feature; all-feature dependency closure is platform-free |
| Purity negative tests | **4 passed**, including independent provider/tool roots |
| `python3 scripts/issue11-ledger.py check` | **9,142 IDs**, **251 excluded**, **8,891 unresolved**; no invented semantic closure |

Independent source review found four concrete bugs and all received fixes and
regressions: paused runs terminalizing observations, HOME fallback reading an
untrusted checkout, per-run shutdown ending other shared OTel runs, and normal
multi-turn runs producing error agent spans.

The thirty ignored tests are 28 pinned-Go MCP interop cases, one pinned-Go
configuration/history replay case, and one sandbox-invoked network helper.
They are **not** counted as passing. Provider/OAuth credentials, real Postgres/
pgvector, remote MCP/OTLP collectors, required OS-confinement jobs and non-Linux
execution were not configured or verified here. The native examples use an
explicit local subprocess backend, not a claim of sandbox enforcement.

### Blockers to closing #11

1. The complete retained ledger still needs acceptance-ID-specific semantic
   evidence; the immutable audit overlay deliberately remains unresolved.
2. Native observation schema 1 and event-store layout do not implement Go trace
   schema 2, category files, metadata/score/artifact/list APIs, reopening or every
   span attribute. Existing Go event adapters remain separate, not a claim of
   full trace/event format parity.
3. The builder does not automatically construct specialist/handoff graphs,
   prime project state, discover MCP stores/connections, install arbitrary
   guardrail callbacks or implement all public ChatLoop/AutoLoop/phase helpers.
   Mode subagent/runtime limits and default reasoning/verbosity mappings have
   explicit unsupported/different behavior in the builder document.
4. Go OTLP endpoint/environment/stdout constructor defaults and complete child/
   session progress inference are not provided by the injected-tracer bridge.
5. Live-provider, external-service and cross-OS evidence is unverified. The
   checkout-pin discrepancy was resolved in the continuation below.

These are blockers, **not accepted divergences or capabilities hidden behind
flags**. The PR must remain draft and must not auto-close #11. No worker CLI,
evaluation/Terminal-Bench adapter, release, merge or production default switch
is part of this delivery.

## PR33 continuation: authoritative reference and store/exporter APIs

The source checkout is now clean at the authoritative
`1dc92b73900fac74dc357a938e4b5eee6392b418`. Both
`python3 scripts/inventory/validate.py` and
`python3 scripts/check-source-lock.py` pass. The latter also required restoring
the clean attached platform reference to its already-accepted source-lock
revision `08e65c970830f05042c251bcbb46ec6a9e3719b9`; no platform source was
changed. No accepted baseline, source lock, archived hash or historical audit
snapshot was rewritten.

Fresh independent Go execution is recorded in
[`issue-11-pinned-reference.json`](issue-11-pinned-reference.json):
`go test -count=1 -json ./pkg/agentsdk/...`, Go 1.26.8, **729 passed test/subtest
records, 5 skipped**, across 34 packages. Package results include packages
without tests; they are not counted as passing tests. The overlay now attaches
exact source-test execution evidence to **644 acceptance IDs**, separately from
Rust semantic verification. These source results do not close the 8,891 retained
Rust obligations.

New [category-store and exporter APIs](../trace-store.md):

- Linux `tracestore::TraceStore` / `FilesystemTraceStore`, metadata and scores,
  category rotation, artifact writes, reopen, list/filter, private atomic
  metadata replacement and explicit close.
- An independently generated pinned-Go store fixture plus Rust filesystem and
  quota regressions. The original schema-1 event writer is unchanged.
- `telemetry::Telemetry` constructors with explicit/environment/stdout endpoint
  selection, gRPC/TLS, five-second batching, service resource/instrumentation
  defaults, flush/shutdown and explicit global installation.

### Fresh continuation verification

Rust **1.88.0**, Linux x86_64:

| Check | Result |
| --- | --- |
| Full locked workspace/all-features/all-targets | **797 passed, 0 failed, 30 ignored** |
| Locked workspace doctests | **1 passed** |
| Strict workspace Clippy via direct driver | Passed |
| Strict all-feature rustdoc and workspace Rustfmt | Passed |
| Facade feature matrix and independent consumer | Passed |
| Offline feature scenarios | **20/20 passed** |
| Store and telemetry targeted regressions | **5 + 4 passed** |
| Cargo deny 0.20.2 | advisories, bans, licenses, sources passed |
| Python purity/replay tests | **4 + 17 passed** |
| Pinned Go store fixture, source lock, inventory and overlay | Passed |

The dependency review added exact duplicate-version exceptions for `rand` and
`rand_core` 0.9.5: OpenTelemetry SDK 0.31 requires them while Postgres requires
0.10. No advisory or license failure was waived. The older locally installed
cargo-deny 0.18.4 could not parse a CVSS 4 advisory; the repository's already-pinned
0.20.2 checker was installed and used instead. Direct compiler/Clippy binaries
avoid the worker's missing `/proc/self/exe`; no new compiler was substituted for
project verification.

The 30 ignored Rust tests remain unverified, not passing. The five skipped Go
records are the MCP subprocess helper, Linux automatic confinement/daemonized
child checks, and two Darwin Seatbelt checks (exact names are in the reference
report). No live collector, provider credentials, external Postgres/pgvector or
non-Linux target was verified in this continuation. Fresh independent Rust code
re-review has not been obtained.

The remaining trace work is **producer parity**, including the complete Go
schema-2 hook/span writer. Constructor defaults now have offline coverage, but
Go stdout JSON format, complete span attributes and trace-ID callbacks are not
implemented. Higher-level runtime/session/guardrail helper parity and complete
acceptance-ID Rust closure remain unfinished implementation, not external
blockers. This continuation must not be represented as completion of #11.

## Public user-input helper continuation

Added `adk::codec::userinput` with quick-action encoding, question/choice/plan
extraction, ordered pause detection and the auto-turn-cap prompt. The independent
pinned SDK oracle executes **51 raw-input cases / 306 pause observations**, plus
nil/empty action encoding and seven signed turn-cap cases. Byte comparisons cover
Go field order/escaping, malformed inputs, repeated-key slice backing-element
reuse, Unicode field folding and malformed Unicode replacement. See
[`docs/user-input.md`](../user-input.md) for native representation limits and the
explicitly non-executing host API. No worker CLI or evaluation adapter was added.

Fresh Rust 1.88.0 Linux checks: **1,254 passed, 0 failed, 30 ignored** for the locked
workspace/all-features/all-targets; direct Clippy driver with `-D warnings`, strict
rustdoc, changed-file Rustfmt and whitespace checks passed. The facade feature
matrix and platform-free external consumer passed; a separate compat-only
consumer ran the documentation's user-input example. All 20 offline feature
scenarios and the host-session example passed. The new oracle, all existing
pinned evidence checks, 18 ledger-validator tests and both source locks passed.
Python files compile and the Go harness is gofmt-clean.

The first evidence invocation lacked `GOROOT`; rerunning with the established Go
environment fixed it. Invalid-UTF-8 oracle cases then exposed undecodable Go log
output in the Python capture, not a Rust mismatch. The harness now prints those
bytes with backslash escapes (without discarding logs); fresh evidence passes.
The examples script's `cargo clippy` entrypoint cannot resolve `/proc/self/exe`
on this worker; equivalent direct-driver checks, builds and runs passed instead.

Only the encoding helper and turn-cap prompt ledger functions are newly closed:
**243 verified / 8,648 unresolved / 251 excluded**. This is not full issue #11
completion or a claim that unresolved records each represent a missing feature.
No new credentialed live-service checks were run; ignored checks remain unverified.

## Completion-confirmation continuation

Added the default-off `RunnerConfig::require_completion_confirmation`, scalar
Go configuration mapping, exact SDK confirmation prompt, tool/nonfinal/steering
resets, stop-gate ordering and ordinary-turn extension. Eighteen independently
executed SDK normal/streamed cases compare results, feedback, tool availability
and admission/model counters. Native tests also cover denied tools, exhausted
token budgets, checkpoint recovery across every candidate boundary, configuration
binding and Go migration's required pending state/effective turn budget.

An initially failing regression showed the new confirmation prompt was delivered
after the next immediate-input callback. It now publishes before admission; the
normal/streamed ordering regression passes. The existing stop gate already skips
forced no-tool summaries through `effective_tool_policy`; no stop-gate correction
was needed. This does not implement `FinalAnswerVerifier` or its critic helper.

Fresh locked Linux Rust 1.88.0 workspace/all-features/all-targets:
**1,260 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy with
`-D warnings`, strict rustdoc, changed-file Rustfmt, full facade matrix and
platform-free consumer, all 20 offline scenarios and the host-session example
passed. All pinned oracles, 18 ledger-validator tests, both source locks, Python
compilation, Go formatting and whitespace checks passed. The two actual SDK
completion-confirmation source tests passed independently and were added to the
reference evidence report. Only the exact configuration-field obligation is
newly closed: **244 verified / 8,647 unresolved / 251 excluded**. Immutable
inventories and source locks remain untouched. No new credentialed live checks
were run. Issue #11 is still incomplete; this is not a release/default switch.

## Independent final-answer verifier continuation

Added the owned async `FinalAnswerVerifier`, default-off runner registration,
once-per-run state after stop-gate/confirmation checks, exact SDK feedback and
confirmation reset/turn extension. Twenty-eight independently executed pinned
normal/streamed cases cover acceptance, whitespace, rejection, callback failure,
tool/nonfinal progress, forced/no-tool turns, gate ordering and structured output.
Native adaptations are explicit: borrowed JSON instead of callback-text encoding,
host-owned capture-policy-safe diagnostics instead of global stderr, and parent
cancellation/deadlines remain authoritative. These are not authorization checks.

Durable execution requires an explicitly identified pure/replay-safe verifier.
Committed invocation state and extended budget survive recovery; configuration
changes and missing Go recovery evidence fail closed. Uncommitted pure reviews
may replay; this is not an exactly-once adapter for live critic model calls. The
separate SDK `NewCriticVerifier` constructor remains unimplemented at this checkpoint.

Fresh locked Rust 1.88.0 Linux workspace/all-features/all-targets:
**1,265 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, changed-file Rustfmt, full facade matrix/platform-free consumer, all 20
offline scenarios and the host-session example passed. Pinned oracle/evidence
checks, 18 ledger-validator tests, source locks, Python compilation, Go formatting
and whitespace checks passed. The actual SDK single-refutation source test also
passed freshly. Only the specific verifier configuration-field obligation is
newly closed: **245 verified / 8,646 unresolved / 251 excluded**. No new
credentialed live-service checks were run. The broader issue remains incomplete;
no CLI/evaluation/Terminal-Bench, release or production-default switch was added.

### Owned read-only critic helper

The native `CriticVerifier` owns its runner, critic agent, host and original task.
Twenty-nine independently executed pinned SDK cases compare exact prompts and
instructions, verdict parsing, structured output, model errors, read-only tool
advertisement and execution, forged mutation denial, child limits and the forced
twelfth summary turn. Native integration verifies a single parent revision and
cancellation before dispatch. Live critics deliberately remain unsupported in
durable callback registration; no replay-safe identity is fabricated.

Fresh locked Linux Rust 1.88.0 workspace/all-features/all-targets:
**1,267 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, changed-file Rustfmt, facade feature matrix, platform-free standalone
consumer, 20 offline scenarios and host-session example passed. The ordinary
Clippy launcher remains unusable without `/proc/self/exe`; the equivalent
strict direct-driver checks passed. Source oracle reproduction, 18 ledger tests,
source locks, Python compilation, Go formatting and whitespace checks passed.
Only the critic constructor and default-instruction obligations are newly closed:
**247 verified / 8,644 unresolved / 251 excluded**. Credentialed live-provider
checks remain unverified. No excluded CLI/evaluation/Terminal-Bench contracts,
production default changes or release work were added. Issue #11 is incomplete.

### Dynamic agent instructions

`AgentConfig::instruction_provider` resolves host-owned instructions before each
model attempt, including retries and destination-agent handoffs. Sixteen pinned
SDK normal/streamed observations cover fallback/dynamic/blank precedence,
additional/MCP composition, cumulative usage, retries and forced-summary retries.
Native tests cover errors before dispatch, cancellation/drop of pending futures,
and pure identity/fingerprint checks across durable recovery. The ordinary Go
static/dynamic source regressions also passed and are recorded in pinned evidence.

Fresh locked Linux Rust 1.88.0 workspace/all-features/all-targets:
**1,270 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, Rustfmt, facade matrix, platform-free consumer, all 20 offline scenarios
and host-session example passed. Source oracles, 18 ledger validators, immutable
source locks, Python compilation, Go formatting and whitespace checks passed.
The first ledger generation correctly rejected a missing reference-test entry;
the source tests were executed as JSON and recorded, then all gates rerun.
Only the dynamic-instruction field capability is newly closed:
**248 verified / 8,643 unresolved / 251 excluded**. Full issue #11 remains open.
No new credentialed live checks, excluded CLI/evaluation contracts, release work
or production default changes were performed.
