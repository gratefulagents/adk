# Issue #11 verification ledger

## Current continuation evidence

### Additional original type regressions and no-op hooks

Mapped six original regressions: static/dynamic instruction precedence, explicit
tool-before-handoff ordering, ordinary-ASCII tripwire messages, unknown provider
rejection, and no-op hooks. Existing independent request/route/guardrail coverage
supports the first five. A new test invokes empty `CompositeHooks` through
`Arc<dyn RunHooks>` for all seven original lifecycle categories; its production
implementation is now included in the evidence input hashes. Native owned
observations replace Go nullable callback arguments. These are test-level claims,
not whole agent, provider, hook or arbitrary Go formatting closure.

Fresh native verification: **1,758 workspace +10 doctests passed; 0 failed,
30 ignored**; strict Clippy/typecheck/format passed. Final independent references,
**56 validators**, separate artifact proof, overlay/audit and source locks passed.
The first Go run failed `TestServerModeUsesPolicyBoundaryAndImmutableRequest`;
ten targeted repeats passed, followed by a full **932 pass /5 skip /38 package**
run. The cause was not established and no SDK source was changed. The reference
runner now preserves failing command output, with a unit test proving diagnostics
and nonzero status survive; this does not suppress failures or retry internally.

Ledger: **1,080 verified /7,811 unresolved /251 excluded**. All46 remote checks
passed at preceding `57eab4e`; full acceptance remains incomplete.

### Original parent/child turn default regressions

Independently executed the exact `TestRunConfigEffectiveMaxTurns` and
`TestRunConfigEffectiveSubAgentMaxTurns` originals. Native counterparts cover
parent100 and explicit50; child50 remains independent of parent400; child200
is accepted explicitly and reaches scheduler dispatch. Six records now map only
the fixed default values, their public aliases and those original regressions.
Rust exposes values through typed `Default` policies and positive fields, not
Go zero-sentinel getters or duplicate alias names. Whole configuration types,
other scheduler bounds and all exhaustion behavior remain outside these claims.

Fresh Rust 1.88 Linux: **1,757 workspace +10 doctests passed; 0 failed, 30 ignored**.
Strict Clippy/typecheck/format, independent bound references, all55 validators,
artifact/overlay/audit and immutable source locks passed. Original Go report:
**928 pass /5 skip /38 packages**. Implementation is unchanged from the preceding
full matrix/docs/consumer and20-example checks. Ledger: **1,074 verified /7,817
unresolved /251 excluded**. Full retained acceptance remains incomplete.

### Managed child turn default

Reproduced the native scheduler dispatching a 100-turn default where the pinned
SDK registry resolves to 50. The scheduler now defaults to 50; parent policy
remains 100. Regression coverage inspects actual child invocation policies for
default 50, explicit 40, a lower submission limit of 12, and explicit 120 with a
150-turn submission allowance. The independent Go harness executes the exported
resolvers with zero, negative, and positive inputs; its test file is included in
the existing evidence input hashes and runs through `check.py`.

The initial full suite and concurrent evidence run exposed the old child guidance
expectation of a 100-turn ceiling. That expectation is now 50, retaining the full
static/dynamic instruction and non-mutation assertions. Final Rust 1.88 Linux:
**1,756 workspace +10 doctests passed; 0 failed, 30 ignored**. Strict
Clippy/typecheck/rustdoc/format, facade matrix, standalone consumer/purity,
all 20 offline feature scenarios and the host-session example passed. Examples
used the established direct-driver Clippy workaround. Go harness test/vet/build,
all bound references, 55 validators, artifact/overlay/audit and source locks passed.
Ledger remains **1,068 verified /7,823 unresolved /251 excluded**; these fixes do
not close the remaining SDK acceptance obligations.

### Child default cache namespace

Reproduced a child request hashing its task-specific execution ID rather than the
shared owner identity. Pinned `subagent_registry.go` supplies the parent trace to
child RunConfig; the native child runner now uses the scheduler's root run ID
when its configured namespace is absent or blank. Explicit overrides remain
intact. No run-ID string parsing or public invocation-field change is involved.

The expanded regression checks five namespace forms, two inherited key forms and
two tasks (20 requests). A separate integration test compares the default child
request directly with the independently executed Go root-trace fixture. Fresh
Rust 1.88 Linux: **1,755 workspace +10 doctests passed; 0 failed, 30 ignored**;
36 subagent integration tests, strict Clippy/typecheck/rustdoc/format, facade
matrix and standalone consumer/purity passed. All bound references, 55 validators,
artifact/overlay/audit and source locks passed. No additional ledger records are
closed: **1,068 verified /7,823 unresolved /251 excluded**; full acceptance remains
incomplete.

### Exported prompt-cache configuration inputs

Mapped the two logical-key/namespace fields to native `RunnerConfig`, with an
additional regression comparing explicit empty options and absent options against
the independently executed Go request oracle. These field-level mappings retain
the caller-owned identity boundary described below; they do not close the entire
configuration type or imply hidden Go trace/context emulation.

Fresh Rust 1.88 Linux: **1,754 workspace +10 doctests passed; 0 failed, 30 ignored**.
Formatting and strict all-target/all-feature Clippy/typecheck passed. Independent
bound-reference reproduction, all 55 validators, artifact/overlay/audit checks and
immutable source locks passed. Ledger: **1,068 verified /7,823 unresolved /251
excluded**. Full retained acceptance remains incomplete.

### Parent cache defaults and normalization

Direct runners now trim explicit logical keys/namespaces, default absent or blank
keys to `run`, and use caller-owned `Context.run_id` when the namespace is blank.
Every request receives the namespace/NUL/logical-key SHA-256 digest. This maps a
supplied Go Trace.ID to explicit native identity; it does not synthesize hidden
Go trace objects or reproduce arbitrary Go context-value storage.

The new independently executed oracle records **216 cases /432 model requests**,
normal and streamed, with repeated runs sharing identity, different namespaces,
Unicode whitespace, embedded NULs and the original long Unicode logical label.
All three original cache regressions are now independently executed and mapped.
Reviewed the complete 55-line test file; the inventory's extra subtest record is
actually its Runner.Run call and is mapped explicitly rather than excluded.
Ledger: **1,066 verified /7,825 unresolved /251 excluded**. Full acceptance remains
incomplete; no whole runner/cache-configuration closure is inferred.

The initial full run exposed a builder test equating agent ModelSettings with
request settings. Go carries its cache key separately; the native request map
now has request-only cache metadata. The test preserves every original builder
setting assertion and checks the exact independently calculated cache digest.

Fresh Rust 1.88 Linux: **1,753 workspace +10 doctests passed; 0 failed, 30 ignored**.
Strict all-feature Clippy/typecheck/rustdoc/format, full facade matrix and external
consumer/purity passed. Go harness test/vet/build, twice-reproduced cache oracle
and all other bound references, **926 original Go passes /5 skips /38 packages**,
all 55 validators, artifact/overlay/audit and immutable source locks passed.

### Stable child prompt-cache identity

Reproduced a missing child cache key when the registered runner had none. Child
execution now always replaces the inherited logical key with its stable task ID,
matching the original Go child helper. End-to-end tests inspect actual model
requests for the original shared-run/task-stable-123/inherited-parent-key inputs,
an absent parent key, and a distinct second task. Explicit namespaces remain
shared; parent default keys and implicit namespace/trace identity remain separate
unresolved contracts.

The original Go regression was independently executed and added to the bound
report: **924 pass /5 skip across 38 packages**. Its one exact test record is now
verified; this is not whole cache-helper/test-file closure. Ledger: **1,062 verified
/7,829 unresolved /251 excluded**. Full acceptance remains incomplete.

Fresh Rust 1.88 Linux: **1,752 workspace +10 doctests passed; 0 failed, 30 ignored**.
Strict Clippy/typecheck/rustdoc/format, complete facade feature matrix, standalone
consumer/purity, independently reproduced bound references, all 55 validators,
artifact/overlay/audit and source locks passed. All 46 remote checks passed at
preceding Unicode-fix head `3110d520dfe42a25bb3aaa207885486ddc9f80fd`; that is not a
current-child-change CI claim.

### Unicode property grammar in guardrail rules

Reproduced the Go-valid `\P{^adlam}` rejection before changing the shared
host/public compiler. Property caret negation now toggles the outer sign, and
names are normalized/validated against 200 names accepted by the actual pinned
Go compiler. Rust-only binary properties, property assignments and script
abbreviations no longer bypass Go validation. A second reproduced mismatch,
`Cs`, is translated to an empty UTF-8 class (or its complement); Rust has no
surrogate category table.

The independently executed SDK corpus adds **568 compilation cases (412 accepted,
156 rejected)** and **84 patterns /142 inputs**, or **11,928 membership decisions
per compiler**. The checked-in name catalog is compared exactly to the Go output.
Candidates come from categories/scripts/aliases but acceptance is not inferred
from map membership: this toolchain rejects some normalized script keys. The
scope is name grammar/negation and bounded membership, not Unicode range-table,
case-fold version, or full RE2 equivalence. Ledger unchanged: **1,061 verified
/7,830 unresolved /251 excluded**; full issue acceptance remains incomplete.

Fresh Rust 1.88 Linux: **1,751 workspace +10 doctests passed; 0 failed, 30 ignored**.
Strict workspace Clippy/typecheck/rustdoc, formatting, full facade matrix and
standalone consumer/purity passed. Go test/vet/build, independently reproduced
bound references, all 55 validators, refreshed artifact evidence and source locks
passed. The ordinary example script failed in the Clippy launcher because this
worker lacks `/proc/self/exe`; equivalent direct-driver strict Clippy, build and
execution passed, including the host-session example and **all 20 offline feature
scenarios**. No failed application check was suppressed.

### Legacy final-summary selection

Added the missing legacy `Config::force_final_summary_turn` input, defaulting off,
matching the pinned Go `Config.ForceFinalSummary` projection in `legacyFeatures`.
Explicit `Features` remain authoritative. The existing end-to-end builder test
now checks legacy off/on and explicit opposite-value overrides, including actual
model request tools and the final-turn directive, against a conflicting direct
runner configuration.

Fresh Rust 1.88 Linux: **1,749 workspace +10 doctests passed; 0 failed, 30 ignored**.
Strict all-feature Clippy/typecheck/rustdoc, formatting, full facade feature matrix,
standalone consumer/purity, independently reproduced bound references, all 55
validators, artifacts/overlay/audit and source locks passed. The first formatting
check requested standard line wrapping; formatting was applied before the passing
checks. No new whole-feature or ledger claims: **1,061 verified /7,830 unresolved
/251 excluded**. Full acceptance remains incomplete.

### Complete MCP prompt helper review

Reviewed the complete 63-line MCP prompt helper and mapped its original test
file and three original regressions to explicit native tests: server-name
flattening, empty contexts, and control stripping/64-rune clamping. Existing
independent Unicode scalar and normal/stream request fixtures remain bound to
these claims. This closes the pure formatter, not MCP transport or tool assembly.

Fresh Rust 1.88 Linux verification: **1,749 workspace tests +10 doctests passed;
0 failed, 30 ignored**. Strict all-feature lint/typecheck/rustdoc and formatting
passed. Independently executed Go regressions reported **923 pass /5 skip across
38 packages**; bound references, all 55 validators, artifacts/overlay/audit and
immutable source locks passed. Ledger: **1,061 verified /7,830 unresolved /251
excluded**. Full issue acceptance remains incomplete.

### Complete pure routing-file review

Reviewed all 235 lines of `internal/agent/mode_routing.go`: five data types,
ten constants, fourteen fields, five exported helpers and three private helpers.
Its 34 API records and twelve branch-conditioned defaults were already verified;
the complete file is now mapped to the data codecs, pure resolvers and native
settings helpers. Exact role keys, fallback order/duplicates, empty-list behavior,
normalization and explicit-none budget clearing are independently covered.

The native label boundary is explicit strings, not Go `any`/`fmt.Sprint` or
arbitrary formatting callbacks. Hosts render domain labels before invoking the
helper; this preserves all label values without reproducing Go reflection.
Provider construction/clamping, runtime enforcement and the containing agent
package are not closed by this file review.

Fresh independent reference reproduction, all claimed Rust regressions,
55 validators, artifact/overlay/audit, source locks and formatting passed.
No implementation changed after the **1,746 workspace +10 doctest** passing run.
Ledger: **1,056 verified /7,835 unresolved /251 excluded**. Full issue acceptance
remains incomplete.

### Settings merge and signed retry-advice data

The lossless settings DTO now supports the original nonzero/non-nil merge value
contract without passing through the analysis snapshot projection. Optional zero
values override; integer limits override only when positive; strings and stop
lists override only when nonempty. Base nil/allocated-empty state is retained,
and results own their data under the existing Rust migration decision.

Added `dto::ModelRetryAdvice` with signed 64-bit delay and exact `RetryAfterMS`
wire spelling. This does not change native Duration conversion or runtime retry
policy. The actual Go oracle adds **89 advice inputs (45 accepted, 44 rejected)**
and **534 Merge calls**, comparing complete binary64/nil state and exact bytes.
The combined data corpus has **1,901 inputs (1,120 accepted, 781 rejected)** plus
six nonfinite encoding cases. Thirteen mode/settings tests and five routing
regressions pass.

Reviewed all 55 lines of `internal/agent/model_settings.go`: two structs, twelve
fields and one merge method, with no runtime/callback implementation. Five new
retry declarations/aliases and the complete data/merge file have bounded claims;
existing settings-field and merge claims now include lossless state evidence.

Fresh Rust 1.88 Linux: **1,746 workspace tests +10 doctests passed; 0 failed,
30 ignored**. Strict all-feature lint/typecheck/rustdoc, formatting, facade matrix,
external consumer/purity, independently reproduced bound references, 55 validators,
artifact/overlay/audit and source locks pass. Ledger: **1,055 verified /7,836
unresolved /251 excluded**. Full retained acceptance remains incomplete.

### Go hyphen/range parsing in guardrail classes

Removed the blanket rejection of consecutive hyphens. The shared host/public
compiler now distinguishes simple characters, range operators/endpoints and
standalone Perl/Unicode/POSIX class atoms. Literal hyphens are escaped so Rust
cannot interpret them as set subtraction. Escape operands remain one atom;
a leading literal closing bracket is encoded explicitly so it can be a range
endpoint. A bracket used as a range endpoint is not mistaken for a POSIX atom.

The actual SDK oracle adds **490 patterns /261 inputs: 384 accepted, 106 rejected**,
with **100,224 membership decisions per native compiler**. The original rejection
was reproduced before editing. The first implementation exposed an incorrectly
escaped generated literal and Rust's special treatment of leading closing brackets;
both were corrected against the independent oracle. Obsolete unsupported-category
assertions now require invalid-input rejection for descending ranges instead.
All 14 guardrail oracle tests and seven host-rule tests pass. This removes the
specific hyphen gap, not all remaining Go/Rust regex syntax differences; no new
whole-compiler or ledger closure is inferred.

Fresh Rust 1.88 Linux: **1,743 workspace tests +10 doctests passed; 0 failed,
30 ignored**. Strict all-feature lint/typecheck/rustdoc, formatting, facade matrix
and standalone consumer/purity, Go test/vet/build and byte reproduction, all bound
references, 55 validators, artifact/overlay/audit and source locks pass. Ledger
remains **1,049 verified /7,842 unresolved /251 excluded**; full acceptance is
incomplete.

### Lossless nested routing-result decoding

`mode::ModeRoutingResolution` now supports both sealed byte entrypoints and uses
the lossless settings DTO. Repeated nested value structs retain raw field order
until finalization, preserving hidden stop-sequence backing slots. Unlike pointer
objects, a null nested settings value is a no-op. Existing pure routing output
and standalone composition remain unchanged.

The actual pinned-Go oracle adds **453 result inputs (275 accepted, 178 rejected)**,
including all 298 settings inputs under the correct nested depth. Successful
cases compare every result field, exact nested binary64/nil state and exact
output bytes. The combined corpus is **1,812 inputs (1,075 accepted, 737 rejected)**.
A dedicated regression verifies shortening a stop slice, encountering a null
settings object, and re-expanding through null elements without losing the tail.
Seven result data/alias records are scoped to this fresh-value byte codec, not
ordinary Serde in-place mutation, Go pointer identity or runtime execution.

Fresh Rust 1.88 Linux: **1,742 workspace tests +10 doctests passed; 0 failed,
30 ignored**. Strict all-feature lint/typecheck/rustdoc, formatting, full facade
matrix and external consumer/purity, independent bound references, 55 validators,
artifact/overlay/audit and source locks pass. Ledger: **1,049 verified /7,842
unresolved /251 excluded**. Full retained acceptance is still incomplete.

### Lossless model-settings compatibility record

Added a separate `dto::ModelSettings` to the sealed byte codec; the existing
analysis-oriented snapshot and native runtime settings APIs are unchanged.
Optional float/bool pointers retain explicit zero values, stop sequences retain
nil versus allocated-empty state, and Go omission rules remain independent of
that state. Integer spelling/bounds, null behavior, repeated fields and slice
backing slots use the existing Go decoder semantics.

The immutable SDK oracle adds **298 settings inputs (178 accepted, 120 rejected)**
and six actual-Go nonfinite encoding failures. Successful cases compare all nine
fields, with binary64 bits and explicit slice state rather than a lossy JSON
projection, and exact output bytes. Coverage includes signed zero, underflow,
subnormals, largest finite floats, overflow and exact half-way rounding. The
combined mode/settings corpus now has **1,359 inputs (800 accepted, 559 rejected)**.
Nine targeted Rust tests pass. The two new claims concern only the data type and
public alias, not Merge, provider application or ordinary Serde in-place parity.

Fresh Rust 1.88 Linux: **1,741 workspace tests +10 doctests passed; 0 failed,
30 ignored**. Strict all-feature workspace lint/typecheck/rustdoc, formatting,
full facade feature matrix and standalone consumer/purity, independent bound
references, 55 validators, artifact/overlay/audit and source locks pass. Ledger:
**1,042 verified /7,849 unresolved /251 excluded**; full acceptance remains
incomplete. All 46 remote checks passed for preceding head `6f006ae`, not this
subsequent settings implementation.

### Branch-conditioned mode settings defaults

Reviewed and verified all twelve literal-return records in the reasoning and
verbosity helpers. The existing independent Go fixture exercises 12 reasoning
inputs ×6 verbosity inputs, comparing complete settings maps and their separate
helper composition. It covers all tiers, empty/unknown labels, Unicode trimming
and simple lowercase; all seven original effort/budget assertions also pass.
These are branch-conditioned helper values, not global runtime defaults or
provider-specific clamps. Explicit `none` emits no budget; clearing an inherited
role budget remains a separate, independently tested routing operation.

Fresh independent reproduction, bound Rust tests, 55 ledger/audit validators,
artifact/overlay/audit and source locks passed. No implementation changed from
`af314db`, whose 1,739 workspace tests +10 doctests, strict lint/docs, complete
feature matrix and standalone consumer/purity passed. Ledger:
**1,040 verified /7,851 unresolved /251 excluded**. Full acceptance is incomplete.

### Open reasoning/verbosity labels and typed constants

Added compatibility `ModeReasoningLevel` and `ModeTextVerbosity` as open string
wrappers with ten typed associated constants, not closed enums. Arbitrary labels
remain unchanged; the empty default is distinct from explicit `NONE`. Static and
owned strings share comparison/hash identity, display raw text and borrow as
`str` for native settings helpers. Existing routing fields remain plain strings.

The immutable Go oracle adds 68 scalar cases (44 accepted, 24 rejected) and all
ten constant values. Current total: **1,061 codec cases, 622 accepted and 439
rejected**, including the original 993 structured-data cases. Successful raw
values and exact bytes, invalid kinds, null defaults, Unicode repair, constant
identity and native interoperability all pass. This closes 24 exact type/constant
records, not runtime enforcement or provider behavior.

Fresh Rust 1.88 Linux: **1,739 workspace tests +10 doctests passed; 0 failed,
30 ignored**. Strict lint/typecheck/rustdoc/format, full feature matrix and
standalone consumer/purity, independent Go reproduction, bound evidence,
55 validators, artifact/overlay/audit and source locks passed. Ledger:
**1,028 verified /7,863 unresolved /251 excluded**; full acceptance is incomplete.

### Direct model-alias resolution

Exposed `adk::codec::mode::resolve_model_for_provider`, sharing the pure resolver
already used by mode/role routing. The actual-Go oracle now checks 294 direct
alias results in addition to 1,764 routing results. The complete implementation
file, both API/alias records, original test and complete test file are reviewed
and covered. Unknown providers preserve trimmed model names; only Anthropic
supplies an empty-model default. No model construction or provider validation
is performed.

The newly required original Go regression was absent from the prior bounded
reference selection. Refreshing the claim-driven reference suite executed it:
**923 original Go test/subtest records passed, 5 skipped**. No missing reference
was treated as a pass. Fresh Rust 1.88 Linux: **1,737 workspace tests +10 doctests
passed; 0 failed, 30 ignored**. Strict lint/typecheck/rustdoc/format, full feature
matrix, standalone consumer/purity, independent reproduction, bound evidence,
55 validators, artifact/overlay/audit and source locks passed.

Ledger: **1,004 verified /7,887 unresolved /251 excluded**. Full retained
acceptance remains incomplete.


### Complete mode data package and original routing regressions

Reviewed the complete one-file `pkg/agentsdk/mode` package: four structs and
24 fields, no functions, initialization or host resources. Its exact source
hash matches the independent 993-case oracle; all 28 declarations already have
verified mappings. File/package closure is limited to this data-only package,
not the broader `sdk::mode` group or internal routing implementation.

Added exact counterparts for every assertion in the original 80-line
`mode_routing_test.go`: seven reasoning effort/budget cases, architect overrides,
planner inheritance and empty routing. Both original Go test entrypoints and
all seven table subtests were freshly executed and passed. This closes six
records: the original test file, its two tests and table site, plus the mode
data file/package.

Fresh Rust 1.88 Linux: **1,736 workspace tests +10 doctests passed;
0 failed, 30 ignored**. Strict workspace lint/typecheck, formatting,
independent reference reproduction, source-bound evidence, 55 validators,
artifact/overlay/audit and source locks passed. Runtime implementation is
unchanged from `470c870`, whose full feature/consumer/purity/docs checks passed.
Ledger: **999 verified /7,892 unresolved /251 excluded**. Full acceptance
remains incomplete.


### Pure top-level and per-role routing resolution

Added compatibility `resolve_mode_routing` and `resolve_role_mode_routing`,
with an owned `ModeRoutingResolution` supporting the sealed Go encoder (not
a Go decoder target). An immutable-SDK oracle executes 294 input groups and
1,764 resolutions, comparing unnormalized state and exact bytes. Coverage
includes aliases/defaults, unknown providers, all reasoning tiers, Unicode
case/whitespace, exact role keys, fallback inheritance/clearing and explicit
`none` clearing a prior thinking budget. Input immutability and independent
result/source fallback ownership are tested.

The standalone consumer now exercises byte decode → template conversion →
role resolution → exact byte encode without platform dependencies. This closes
only four pure helper API/alias records, not result DTO decoding, executable
provider construction, native arbitrary settings merging or runtime policy.

Fresh Rust 1.88 Linux: **1,734 workspace tests and 10 doctests passed;
0 failed, 30 ignored**. Strict workspace lint/typecheck/rustdoc and formatting
passed; the full feature matrix and standalone consumer/purity passed, including
strict lint for the independent consumer. Independent reference reproduction,
bound evidence, 55 ledger/audit validators, artifacts/overlay/audit and source
locks passed. Ledger: **993 verified /7,898 unresolved /251 excluded**; full
retained acceptance remains incomplete.


### Exact compatibility mode-routing conversion

Added `adk::codec::mode::mode_routing_from_template_spec` independently of the
native builder adapter. It copies raw labels and role keys while reproducing
Go's conversion of empty fallback slices and empty role maps to nil. Six
actual-Go cases now retain exact output bytes and compare unnormalized output,
unchanged input and source-mutation isolation. The helper closes one complete
pure-conversion API record; it does not claim model binding or runtime behavior.

Fresh Rust 1.88 Linux: **1,731 workspace tests and 10 doctests passed;
0 failed, 30 ignored**. Strict lint/typecheck/rustdoc, formatting, the complete
feature matrix, standalone consumer/purity, independent Go reproduction,
bound evidence, 54 validators, artifact/overlay/audit and source locks passed.
Ledger: **989 verified /7,902 unresolved /251 excluded**. Full retained issue
acceptance remains incomplete.


### Mode template and routing compatibility codecs

Added compatibility-only `ModeTemplateSpec`, `ModeConstraints`, `ModeRouting`
and `ModeRoleRouting`, covering the six template/runner Go data types through
sealed Go byte entrypoints. The independent immutable-SDK oracle records 993
cases: 578 accepted and 415 rejected. Successful cases compare unnormalized
state and exact marshalled bytes; rejected cases compare rejection only.
Coverage includes 64-bit integer boundaries/spelling, Unicode repair, nil/empty
containers, map entry reset, repeated pointer objects and slice backing slots.

The initial implementation failed the actual-Go pointer regression: shortening
then re-expanding a fallback slice through repeated pointer objects lost the
hidden `"tail"` element. Pointer decoding now retains the ordered object fields
until finalization, validating each occurrence before a later null can reset it.
All codec tests, including the 993-case comparator, pass. The 41 associated
claims cover data declarations only, not constraint enforcement, model binding,
routing resolution or full native conversion. See
`scripts/mode-codec-reference/README.md` for reproduction and limits.

Fresh Rust 1.88 Linux: **1,730 workspace tests and 10 doctests passed;
0 failed, 30 ignored**. Strict lint/typecheck/rustdoc, formatting, the full
feature matrix, standalone consumer and platform-purity checks passed.
Independent Go reproduction, source-bound evidence, 54 ledger/audit validators,
artifact checks and source locks passed. The first evidence run correctly
rejected missing mode-test execution; the explicit test list was fixed and
the complete evidence command rerun successfully. Ledger: **988 verified /
7,903 unresolved /251 excluded**. Full issue acceptance remains incomplete.


### Standalone mode-routing bridge

Added builder-gated `specialists::mode_routing_from_template_spec`, copying raw
SDK routing labels into owned standalone routing without validation, trimming,
model binding or session ownership. Missing templates/routing return `None`;
present empty routing remains `Some`. Six actual-Go conversion observations and
source-mutation checks verify labels, role keys, fallback ordering and ownership.
Native tests also exercise omitted fallbacks and deliberately exclude native
arbitrary settings maps from this SDK-label adapter.

This is not a mode-template/runner-routing wire codec: native nil containers
project to empty owned containers. No full serialization or helper acceptance
record is closed merely from this bounded adapter. Native settings users retain
the existing builder routing-resolution path.

Fresh Rust1.88 Linux: **1,725 workspace tests /0 failed /30 ignored;10 doctests
passed**. Strict lint/typecheck/rustdoc, formatting, full feature matrix and
standalone consumer/purity, independent Go reproduction, bound evidence,
53 validators, artifact/overlay/audit and source locks passed. Ledger remains
**947 verified /7,944 unresolved /251 excluded**; issue acceptance is incomplete.

### Inner regex brackets retain Go literal semantics

The translator no longer rejects Go-valid opening brackets within a class.
Recognized complete POSIX atoms retain their special meaning; other opening
brackets are literal, rather than acquiring Rust nested-class semantics.
Unknown complete POSIX names and unclosed classes remain rejected. Twenty-one
actual-Go patterns include17 accepted forms ×140 inputs (2,380 comparisons per
native compiler) and4 rejected forms. Multi-character input cases distinguish
literal closing brackets after a class from nested-class interpretations.
The prior Go-valid `[a[b]]` rejection was reproduced before the fix.

Fresh Rust1.88 Linux: **1,724 workspace tests /0 failed /30 ignored;10 doctests
passed**. Strict lint/typecheck/rustdoc, formatting, full feature matrix,
standalone consumer/purity, Go test/vet/build, bound references,53 validators,
artifacts/overlay/audit and immutable source locks passed. Double-hyphen syntax
and other parser gaps remain unclaimed. Ledger remains **947 verified /
7,944 unresolved /251 excluded**; issue acceptance is incomplete.

### Named regex groups

Rule compilation validates Go's capture-name grammar and compiles validated
named groups as noncapturing groups, because rule guards expose only membership.
This permits Go-valid digit-leading and duplicate names without accepting
Rust-only non-ASCII names. The22-pattern actual-Go oracle compares12 accepted
forms over12 inputs plus10 rejected forms, covering both spellings, nesting,
repetition, flags, quoting, malformed names and unsupported backreferences.
Digit-leading-name rejection was reproduced before the fix. This does not add
a capture-extraction API or assert full RE2 parser parity.

Fresh Rust1.88 Linux: **1,723 workspace tests /0 failed /30 ignored;10 doctests
passed**. Strict lint/typecheck/rustdoc, formatting, full feature matrix and
standalone consumer/purity, Go test/vet/build, bound pinned evidence,
53 validators, artifacts/overlay/audit and source locks passed. Ledger remains
**947 verified /7,944 unresolved /251 excluded**; full acceptance is incomplete.

### Octal escapes and braced escape delimiters

Go octal escapes now translate to Unicode hex atoms, including NUL, two-digit
forms and values above255. Single nonzero-digit backreferences remain rejected.
The45-pattern actual-Go oracle has41 accepted forms ×141 inputs (5,781 decisions
per native compiler) and4 rejected forms, with ranges, digit termination,
escaped backslashes and quoting controls.

The added braced-hex/property controls caught a regression introduced by the
malformed-count translation: `\x{ff}` was being rewritten to `\x\{ff}`.
Hex and Unicode-property arguments are now consumed as escape arguments, not
repetition delimiters. Both the original octal rejection and the braced-escape
regression were reproduced before fixing.

Fresh Rust1.88 Linux: **1,722 workspace tests /0 failed /30 ignored;10 doctests
passed**. Strict lint/typecheck/rustdoc, formatting, full feature matrix and
standalone consumer/purity, Go test/vet/build, pinned evidence,53 validators,
artifact/overlay/audit and source locks passed. Ledger remains **947 verified /
7,944 unresolved /251 excluded**; full acceptance remains incomplete.

### Literal class-set symbols

Go-valid ampersands, tildes and pipes inside classes now remain literal instead
of being rejected or acquiring Rust class-set semantics. Twenty-one actual-Go
patterns ×142 inputs verify2,982 membership decisions in each native compiler,
including negation, literal closing brackets, case folding, Perl and POSIX atoms.
Double-hyphen and arbitrary nested-bracket syntax remain explicitly unsupported;
this is still not a complete RE2 parser claim.

Fresh Rust1.88 Linux: **1,721 workspace tests /0 failed /30 ignored;10 doctests
passed**. Strict workspace lint/typecheck/rustdoc, formatting, full feature matrix
and standalone consumer/purity, Go tests/vet/build, all pinned evidence,
53 validators, artifact/overlay/audit and immutable source locks passed. Ledger
unchanged at **947 verified /7,944 unresolved /251 excluded**. Full issue acceptance
remains incomplete.

### ASCII Perl atoms inside regex classes

Removed the blanket rejection of valid in-class `\d`, `\s`, `\w` and their
complements while retaining their ASCII Go semantics. The actual SDK oracle
covers40 patterns:35 accepted forms ×142 inputs (4,970 match decisions per native
compiler), plus5 rejected forms. Negation, case-insensitive flags, mixed POSIX
unions, hyphens and leading literal brackets are covered. Word boundaries,
quoting escapes and invalid range endpoints inside classes remain invalid.
Arbitrary nested classes and Rust class-set syntax are still unsupported.

Fresh Rust1.88 Linux: **1,720 workspace tests /0 failed /30 ignored;10 doctests
passed**. Strict workspace lint/typecheck/rustdoc, formatting, feature matrix,
standalone consumer/purity, Go test/vet/build, all pinned evidence,53 validators,
artifact/overlay/audit and source locks passed. No additional ledger closure is
inferred: **947 verified /7,944 unresolved /251 excluded**, still incomplete.

### Literal malformed repetition syntax

The actual Go oracle reproduced a semantic mismatch: `^a{01}$` matched `a` in
native compilation instead of literal `a{01}`. The shared translator now escapes
an unquoted opening brace unless it begins Go's counted-repetition grammar,
including the no-leading-zero rule. Twenty-three patterns across23 inputs check
529 actual-Go membership decisions in each native compiler, with valid counts
and escaped/quoted strings as controls.

Fresh Rust1.88 Linux full rerun: **1,719 workspace tests /0 failed /30 ignored;
10 doctests passed**. Strict lint/typecheck, formatting, feature matrix and
standalone consumer/purity, Go tests/vet/build, bound evidence,53 validators and
source locks passed. The first full run hit an unchanged 1ms timeout regression:
`nested_tool_policy_timeout_resumes_without_cancelling_children` observed one
instead of two tasks (`specialist, slots=1`). Ten isolated reruns and the complete
workspace rerun passed without runtime/test edits; the initial failure is not
silently discarded. Ledger unchanged: **947/7,944/251 verified/unresolved/excluded**.

### Counted regex repetition boundary

A fresh regression reproduced native acceptance of `a{1001}`, rejected by the
actual SDK. Both public and host compilation now validate Go's 1,000-copy bound
for counted repetitions and nested products before compiling the executable
regex. Zero maxima shield outer products without masking independently invalid
inner repetitions. Forty independently executed Go compilation cases cover the
boundary, groups/alternation/concatenation, unbounded and lazy counts, and literal
quoted/escaped/class text. No complete RE2 syntax or error-text parity is claimed.
The host-only `regex-syntax` dependency reuses the existing locked package;
no new dependency version was introduced.

Fresh Rust1.88 Linux: **1,718 workspace tests /0 failed /30 ignored;10 doctests
passed**. Strict workspace lint/typecheck and rustdoc, formatting, feature matrix,
standalone consumer/purity, Go tests/vet/build, all pinned evidence regeneration,
53 validators, artifact/overlay/audit and immutable source locks passed. Ledger
unchanged at **947 verified /7,944 unresolved /251 excluded**; full acceptance is
not complete.

### Role/catalog wire codecs and POSIX guardrail classes

Compatibility-only `adk_codec::dto::{RoleSpec, RoleCatalog}` now expose sealed
Go JSON entry points. The 364-case independently reproduced pinned-Go oracle
checks unnormalized decoded state and exact encoded bytes, preserving nil versus
empty slices, duplicate-field slot reuse, scalar null no-ops, key folding and
Unicode repair. Failed inputs compare rejection, not error text or partial
mutation. Native specialist conversion and executable wrapper parity remain
separate unresolved contracts.

The shared guardrail regex translator now accepts precisely the fourteen Go
POSIX ASCII class atoms and their negations, while still rejecting arbitrary
nested classes and Rust set operations. Both public and host compilers match
62 actual-Go patterns across 142 inputs. This is not full RE2 parser parity.

Fresh Rust1.88 Linux verification: **1,717 workspace tests passed /0 failed /
30 ignored;10 doctests passed**. Strict direct-driver workspace lint/typecheck,
full feature matrix, standalone consumer and platform-purity checks passed.
All pinned evidence regeneration, **53 ledger validators**, artifact review,
overlay/audit and immutable source locks passed. Ten narrowly scoped declaration
and original-test claims bring the ledger to **947 verified /7,944 unresolved /
251 excluded**. Full acceptance remains incomplete; remote current-head CI and
credential-dependent live services are not claimed verified.

### Public compilation and standalone catalog composition

The public rule compiler now retains valid rules while reporting rejected rules
in source order, with regex/action/type validation precedence. Five actual-Go
compiler observations cover partial success, normalization and filtering; native
parser diagnostics and unsupported RE2 syntax remain explicit limits. Extracted
regex translation is shared with the host without conflating their glob/message
policies. A reproduced host bug rejected Go-valid `blocK` actions due to ASCII-only
lowercasing; Unicode normalization fixes it. The new actual-Go host case and
native integration pass (62 scenarios/63 runs,12 raw-input rule cases).

Standalone runtime-gated specialist APIs now separate raw conversion, routed
preparation, model binding and caller-owned session attachment. All54 actual-Go
catalog observations compare fields, access adaptation, routing/fallback settings,
wrapper names/flags and ordering. Native execution tests cover explicit snapshot
registration, output extraction/fallback and owner shutdown. Raw conversion's two
API/configuration records and the literal original access/routing regression are
closed; full wrapper schema/error behavior, live pointers and Agent codec parity
are not claimed. Independent scoped reviews found no behavioral blockers; an
ambiguous feature-doc phrase was corrected.

Fresh Rust1.88 Linux: **1,710 workspace tests passed /0 failed /30 ignored;
10 doctests passed**. Strict workspace/runtime/host lint/typechecks, rustdoc,
formatting, full facade matrix, external consumer and dependency purity passed.
The examples script's `cargo clippy` launcher hit this worker's missing
`/proc/self/exe`; equivalent direct-driver strict checks, offline build, host
session and all20 offline scenarios then passed. Go tests/vet/build, bound
independent oracles, **52 validators**, artifact review, overlay/audit and immutable
source locks passed. Ledger: **937 verified /7,954 unresolved /251 excluded**.
Full issue11 acceptance is still incomplete; remote current-head CI is not claimed.

### Direct public rule constructors

Added host-gated public input/output rule constructors, separate from ChatLoop
compilation. They preserve supplied compiled-regex precedence, raw message
whitespace, public edge-star matching, SDK diagnostic quoting and fail-closed
unnormalized actions. All124 independently executed Go observations match native
outputs/tripwires/replacement state. The literal original unknown-action test
passes; with the other eight original regressions this closes the hardening test
file, not the entire public compiler or RE2 syntax. Native regex compilation,
text-only output projection, absent durable identities and feature requirements
are explicit in docs/guardrails.md. Independent scoped review found no blockers.

Following worker restart, exact source pins/tag and Rust1.88 were restored and
verification rerun: **1,704 workspace tests passed /0 failed /30 ignored;
10 doctests passed**. Host-only and builder-only suites, strict workspace/host
lint/typecheck, rustdoc/format, pinned Go tests/vet/build and twice-exact fixtures
passed. Bound evidence,51 validators, artifact review,overlay/audit and source
locks pass. Ledger: **934 verified /7,957 unresolved /251 excluded**. Issue11 and
PR33 remain incomplete; no current-head remote CI success is claimed here.

### Separate public tool-pattern helper

Added `guardrails::match_tool_pattern` with the public SDK helper's trailing-star
precedence and literal internal stars, without changing ChatLoop's multi-star
matcher. All 120 actual-Go pattern observations compare exactly, including empty
strings, repeated stars, Unicode and newlines. The existing 97 builtin guardrail
cases still run independently. No unrelated original regression is used to claim
full rule-helper ledger closure; standalone rule constructors remain outstanding.

Fresh Rust1.88 Linux: **1,702 workspace tests passed /0 failed /30 ignored;
10 doctests passed**. All-feature and builder-only oracle tests, unchanged host
rule tests, strict workspace and builder-only lint/typecheck, rustdoc, formatting,
Go tests/vet/build and twice-exact fixture regeneration passed. Bound evidence,
51 validators, artifact review, overlay/audit and immutable source locks passed.
Ledger remains **932 verified /7,959 unresolved /251 excluded**. Full acceptance
is not complete.

### Original guardrail hardening regressions

Added literal original destructive/safe command sets, non-shell exclusion, added
secret signatures, invalid rule type/action and empty-action block cases. Seven
original test records are closed. The unvalidated direct public rule-function
constructor remains unported, so the complete hardening file is not closed.
The SDK's public guardrail-rule helper differs from ChatLoop: public helpers use
crd: names, raw messages and single-edge-star matching; ChatLoop uses config:
names, trimmed messages and multi-star globs. Existing native host behavior
correctly follows ChatLoop and was not changed. These narrow regression claims
do not imply public-helper equivalence across those differing contracts.

Fresh Rust1.88 Linux: **1,701 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/typecheck/format,18 builtin+5 rule tests under all features
and host-only, original Go guardrail tests, bound evidence,51 validators,
artifacts/overlay/audit and source locks passed. Production behavior is unchanged.
Ledger: **932 verified /7,959 unresolved /251 excluded**; full acceptance remains
incomplete.

### Secret-input original-text guardrail fix

The expanded actual-Go oracle reproduced a second raw-input bypass: opaque
credential text with a null parsed projection did not trip native secret-input
checks. The guard now scans original argument text before its existing stronger
decoded-value inspection. Its durable identity advances to v2. Three new recipe
cases cover raw GitHub/AWS/PEM inputs without storing credential-shaped values
in the fixture:97 cases total,96 exact comparisons and one intentionally stronger
native decoded-marker result. A real native runner regression verifies both raw
secret input and malformed destructive input trip their respective guards before
any tool execution, without leaking the secret into guardrail reports.

Fresh Rust1.88 Linux: **1,695 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace/builder-only lint/typecheck, rustdoc/format, full feature matrix,
external consumer/purity,20 scenarios, host-session, Go tests/vet/build and exact
fixture checks, bound evidence,51 validators/artifacts/audit/locks passed. Initial
integration-test field accesses were corrected to the native RunOutcome/Report
shapes before passing verification. Ledger unchanged: **925/7,966/251**. Full
issue11 acceptance remains incomplete.

### Destructive guardrail original-input parsing

Reproduced a fail-open guardrail result for the original malformed shell-input
regression: the raw argument text was ignored when its parsed Value was null.
The builtin now checks original text (canonical fallback for value-only calls),
with case-insensitive shell keys, ordered duplicate strings, null preservation
and fail-closed wrong types. Its durable identity advances to v2. The independent
Go oracle expands to94 cases:93 match disposition/output bytes and one retains
intentional stronger native secret inspection. All11 previously excluded raw
malformed cases are now compared; seven new parsing cases are independently run.
Only the original malformed-input test record is newly closed; deep native JSON
limits remain fail-closed, not claimed as unrestricted Go parsing equivalence.

Fresh Rust1.88 Linux: **1,694 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace and builder-only lint/typecheck, rustdoc/format, full feature
matrix/consumer/purity,20 offline examples, host-session, Go tests/vet/build,
twice-exact fixture checks, bound evidence,51 validators/artifacts/audit/locks
passed. The first matrix exposed a missing direct codec dependency under the
builder-only feature; the feature now binds the already-transitive dependency
and the final matrix passes. Ledger: **925 verified /7,966 unresolved /251
excluded**. Full issue11 acceptance remains incomplete.

### Original builtin secret-output regressions

Ported original GitHub/PEM/AWS/GCP and positive/negative corpus assertions through
the actual public builtin output guardrail. The existing corpus is byte-identical
to the pinned SDK fixture. Both partial signature names must resolve to registered
patterns to produce their asserted block/name outputs, covering the original
membership invariant without duplicating a production signature list. All eight
original test records and the complete source test file are closed; generic
normalization differences and whole builtin API parity are not inferred.

Fresh Rust1.88 Linux: **1,693 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/typecheck/format,12 builder-only and all-feature builtin
tests, original Go guardrail tests, bound evidence,51 validators, artifacts/
overlay/audit and source locks passed. Initial test compilation assumed Go's
separate replacement flag; corrected assertions use native Option replacement
content. Production behavior is unchanged. Ledger: **924 verified /7,967 unresolved
/251 excluded**; full acceptance remains incomplete.

### Original event adapter and critic regressions

Ported the three original line-writer cases with exact JSON/chunks and every
assertion: child parent/duration fields, subagent waiting/message fields, and
partial-line silence then combined compact log. Ported the original critic
approval/rejection case through real native runner execution with original task,
candidate and model replies. Closed four original tests and two complete source
test files, not the entire adapter/verifier implementations.

Fresh Rust1.88 Linux: **1,686 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/typecheck/format, five line-writer tests and the targeted
critic case, independent original Go tests, bound evidence,51 validators,
artifacts/overlay/audit and immutable source locks passed. Production behavior
is unchanged. Ledger: **915 verified /7,976 unresolved /251 excluded**; full
issue11 acceptance remains incomplete.

### Original user-input helper regressions

Ported all five original user-input tests: quick-action identity/legacy-key
omission, choice defaults, plan legacy-mode removal, question/plan/fallback
pause detection, and exact turn-cap text. Every original assertion passes;
original Go tests pass independently. Closed five originals and their complete
test file, not unrestricted raw-record codecs or the whole implementation.

Fresh Rust1.88 Linux: **1,682 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/typecheck/format, eight targeted userinput tests, original
Go tests, bound evidence,51 validators, artifacts/overlay/audit and immutable
source locks passed. Production behavior is unchanged. Ledger: **909 verified
/7,982 unresolved /251 excluded**; full acceptance remains incomplete.

### Original conversation helper regressions

Ported all five original conversation tests with their literal message IDs,
text, history floor, summary inputs and queued/immediate ordering. Native
RunBatch provenance preserves agent labels without claiming a complete Agent
codec. All five originals pass independently in Go; native tests pass alongside
the eight-helper reference oracle. Closed five originals and their source test
file, not the entire conversation implementation.

Fresh Rust1.88 Linux: **1,677 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/typecheck/format,12 all-feature and host-only conversation
tests, original Go tests, bound evidence,51 validators, artifacts/overlay/audit
and source locks passed. The first overlay generation rejected bare implementation
paths; exact symbol references were supplied and all evidence/checks regenerated.
Production behavior is unchanged. Ledger: **903 verified /7,988 unresolved /251
excluded**; full issue11 acceptance remains incomplete.

### Original autonomous-loop regressions

Ported all five original tracker/nudge tests: no-tool threshold, warning and
finish guidance, repeated two-tool cycle, diverse exploration, identical calls.
All original Go tests pass independently; native exact inputs and assertions pass
alongside the existing complete observation oracle. Closed the five originals
and their complete test file, not unrestricted counter/Unicode behavior.

Fresh Rust1.88 Linux: **1,672 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/typecheck/format, six targeted tracker tests, original Go
regressions, bound evidence,51 validators, artifacts/overlay/audit and source
locks passed. The initial native message constructor used incorrect enum fields;
it was corrected before successful verification. Production behavior is unchanged.
Ledger: **897 verified /7,994 unresolved /251 excluded**. Full acceptance remains
incomplete.

### Original CRD-shaped file configuration regression

Ported the original mode YAML and planner Markdown verbatim, checking every
original assertion for builtin/file mode listing, routing fallbacks and role
reasoning, constraints, normalized permissions, directive warnings and role
loading. The four-test source file is now covered; its original CRD test and
complete test-file record are closed, not the implementation/package contract.

Fresh Rust1.88 Linux: **1,667 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/typecheck/format,16 host-only and all-feature fileconfig
tests, original pinned Go fileconfig tests, bound evidence,51 validators,
artifacts/overlay/audit and source locks passed. Production behavior is unchanged.
Ledger: **891 verified /8,000 unresolved /251 excluded**; full issue11 acceptance
remains incomplete.

### Automatic builder delegation guide

The builder now injects the verified SDK guide rather than abbreviated task and
handoff text. It retains policy filtering: denied handoffs are not advertised,
and only the managed task tool enables catalog task guidance. Status/control-only
configurations remain non-delegating. Catalog filtering excludes handoff-only
fallback targets from task guidance. Twelve actual-Go builder observations now
carry complete guides; task-enabled cases compare the entire guide within the
assembled prompt. Four task/handoff permission combinations verify independent
filtering, a single signal suffix, and no mutation of the underlying handoffs.

Fresh Rust1.88 Linux: **1,666 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/typecheck/docs/format, full matrix/consumer/purity,20 offline
scenarios, host-session, exact Go regeneration, bound evidence,51 validators,
artifacts/overlay/audit and source locks passed. The initial targeted run caught
one old abbreviated-text assertion; it was updated to the SDK handoff heading
and the final full suite passed. Ledger unchanged: **889/8,002/251**. This does not
close the full runtime builder contract or the remaining issue11 acceptance.

### Caller-owned delegation-guide formatter

Added runtime-gated `specialists::build_delegation_guide`, preserving sorted
specialist keys, blank-key omission, raw descriptions/defaults, handoff order
and labels, exact task-packet guidance and the signal suffix. Six independently
executed Go cases compare entire output strings; the literal original compact
task-packet assertions pass. Model execution is forbidden by the test double.
The helper does not mutate agents or automatically inject builder instructions.
Its API/configuration records and original test are closed, not the whole
specialist builder or automatic guide integration.

Fresh Rust1.88 Linux: **1,665 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full matrix/consumer/purity,20 scenarios,
host-session, default/runtime/all-feature specialist tests, exact Go fixture
regeneration, bound evidence,51 validators, artifacts/overlay/audit and locks
passed. Ledger: **889 verified /8,002 unresolved /251 excluded**. Full issue11
acceptance remains incomplete.

### Original permission-access and OTel truncation regressions

Added the original typo/empty-default permission-access cases through actual
ChatLoop tool visibility and mutation attempts, with approvals disabled to isolate
access enforcement. Added the literal original OTel short/long ASCII truncation
assertions. Both tests and their now-completely-covered source test files are
closed (two permission tests and eleven OTel tests). No whole helper/package
equivalence or arbitrary UTF-8 byte splitting is inferred. Production behavior
did not change.

Fresh Rust1.88 Linux: **1,663 passed /0 failed /30 ignored;10 doctests passed**;
strict workspace lint/typecheck/format, bound references,51 validators, artifacts/
overlay/audit and source locks passed. Ledger: **886 verified /8,005 unresolved
/251 excluded**. Full acceptance remains incomplete.

### Original Anthropic compaction-origin regression file

Ported both original compaction-origin tests with identical native/legacy/foreign
inputs: only native and unstamped ciphertext survives, while a foreign plaintext
summary becomes one marked text block and opaque-only foreign state is dropped.
Both original tests and the fully covered two-test source file are now claimed;
no entire conversion helper/provider parity is inferred. Production code did not
change. An initial test compilation missed the existing stream argument; corrected
before final checks.

Fresh Rust1.88 Linux: **1,661 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/typecheck/format, bound references,51 validators, artifact/
overlay/audit and immutable source locks passed. Matrix/consumer/purity,20
scenarios and host-session passed on the unchanged implementation in the preceding
assembler checkpoint. Ledger: **882 verified /8,009 unresolved /251 excluded**.
Full issue11 acceptance remains incomplete.

### Independent partial Anthropic response assembler

Added caller-owned `anthropic::StreamAssembler` separately from strict native
transport validation. It handles partial blocks without message_start, stopped
block order, replacement starts, duplicate stops, reasoning/compaction ciphertext,
raw tool arguments and signed/nonzero usage updates.16 independent Go scenarios
compare117 snapshots exactly after every event; invalid accumulated tool JSON
fails response encoding, and malformed event input does not mutate state.

The literal original encrypted-compaction regression now passes without adding
a synthetic start or terminal. Only that test ID is newly closed; the UTF-8 Serde
event boundary is explicitly not complete Go JSON decoding. Existing transport
ordering tests still reject the malformed network sequence. The pinned-test
selector now retains claimed internal/anthropic tests during report refresh
(previous reports included them but regeneration omitted that package).

Fresh Rust1.88 Linux: **1,659 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace lint/docs/format, full matrix/consumer/purity,20 scenarios,
host-session, bound references,51 validators, artifacts/overlay/audit/source
locks passed. Pinned original Go report:922 passed/5 skipped across38 packages.
Ledger: **879 verified /8,012 unresolved /251 excluded**. Full acceptance remains
incomplete.

### Quoted host guardrail regex literals

Reproduced rejection of a Go-valid quoted literal containing `(?u)`. Host rules
now translate `\Q...\E` outside character classes, including quotes extending
to the end of the pattern. Rust-only flags are checked only in unquoted group
syntax; quoting does not enable them. Seven additional actual-Go ChatLoop cases
cover metacharacters, Unicode, empty/unterminated quotes, following operators
and nonmatches. All11 lexical-input scenarios agree on tool execution and
tripwire disposition. Unsupported class escapes and Rust-only flags remain
rejected; no whole RE2 equivalence or new ledger closure is claimed.

Fresh Rust1.88 Linux: **1,656 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full feature matrix, standalone consumer/
purity,20 scenarios, host-session, Go harness tests/vet/build, twice-exact fixture
regeneration, all bound evidence,51 validators, artifacts/overlay/audit and source
locks passed. The host fixture has61 scenarios/62 runs. Ledger remains
**878 verified /8,013 unresolved /251 excluded**; issue11 remains incomplete.

### SDK-shaped host progress callback

Corrected RunStatusSink::publish_progress to accept progress::ProgressSnapshot,
not the distinct native observability counters. A compile-time reproduction
identified the incompatible callback parameter. All107 pinned-Go tracker states
now retain every field through a dyn host sink, including model usage, signed
tokens, cost and recent events. Fixture timestamps use a fixed valid instant
for passthrough; no timing or general decoder equivalence is claimed. Publication
remains host-owned, not automatically invoked by ChatLoop.

Fresh Rust1.88 Linux: **1,655 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full feature matrix, external consumer/
purity,20 offline scenarios, host-session, bound reference evidence,51 validators,
artifacts/overlay/audit and immutable source locks passed. One callback contract
closed: **878 verified /8,013 unresolved /251 excluded**. Issue11 is not complete.

### Original host guardrail regressions and safe failure context

Reproduced two original regression failures: host rule compilation lacked its
operation context, and model-visible tripwires used enum-debug phase labels.
Compilation failures now preserve their category/cause with `compile guardrail
rules` context. Tripwires identify the lowercase phase and tool name without
copying arbitrary diagnostic Output into serialized errors. A four-phase privacy
regression verifies that Output remains in the typed cause but not ErrorInfo.

The original callback/configured input/output guardrail cases and invalid-regex
case now pass in Rust and pinned Go. Five test IDs and the two completely covered
original ChatLoop test files are closed (seven tests in chatloop_test.go, one in
chatloop_multi_approval_test.go). This is test coverage, not whole ChatLoop,
interface, error-formatter or regex-engine equivalence; diagnostic suffixes and
arbitrary error-label quoting remain outside these claims.

Fresh Rust1.88 Linux: **1,654 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full feature matrix, standalone consumer/
purity,20 offline scenarios, host-session, all bound references and51 validators,
artifacts/overlay/audit and immutable source locks passed. Ledger: **877 verified
/8,014 unresolved /251 excluded**. Full issue11 acceptance remains incomplete.

### Host input rules retain argument spelling

Reproduced a configured input guardrail allowing a tool that the pinned Go
ChatLoop blocks: canonicalizing `{"value": "secret"}` removed the whitespace
matched by the rule. Input rules now use original UTF-8 argument text when
available, with the existing serialized-value fallback. Four independent actual
Go runs cover whitespace, duplicate keys, escaped spelling and a literal
nonmatch. Native runs compare tool execution, tripwire disposition and final
text; the fixture retains lexical input as a string because RawMessage JSON
encoding compacts it. The host oracle now contains54 scenarios/55 Run calls.
No arbitrary non-UTF-8, full regex-engine or error-text equivalence is inferred.

Fresh Rust1.88 Linux: **1,651 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full feature matrix, standalone consumer/
purity,20 offline scenarios, host-session, Go harness tests/vet/build, twice-exact
host fixture regeneration, all bound evidence,51 validators, artifacts/overlay/
audit and immutable source locks passed. An initial Cow/String compile mismatch
was corrected before final verification. Ledger remains **870 verified /8,021
unresolved /251 excluded**; issue11 remains incomplete.

### ChatLoop approval regressions and default cache identity

The original single-approval regression reproduced a missing default model
prompt-cache key. ChatLoop now resolves a trimmed logical key (`run` by default)
and a per-invocation namespace before constructing the runner. Approval resumes
retain that identity; separate invocations do not share it unless explicitly
configured. The fixed-namespace key matches the pinned Go host fixture. Hosts
must explicitly supply a shared trace namespace; no implicit Go Trace object
mapping or lower-level Runner default equivalence is inferred.

Literal native counterparts of the original single-approval, parallel-approval
and approved-tool timeout regressions pass, as do the original pinned Go tests.
Only those three test IDs are closed. The timeout test uses the original real
one-second timeout/three-second upper bound; Tokio virtual-time support is not
enabled in this crate.

Fresh Rust1.88 Linux: **1,650 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full feature matrix, standalone consumer/
purity,20 offline scenarios, host-session, all bound reference evidence,
artifacts/overlay/audit,51 validators and immutable source locks passed. Ledger:
**870 verified /8,021 unresolved /251 excluded**. All46 remote CI checks passed
on prior dfdad2c7; new-head CI is not yet claimed. Issue11 remains incomplete.

### Usage integer and object decoding

Reproduced ordinary Serde rejecting integer `-0` in usage counters accepted by
the pinned Go SDK. `codec::dto::Usage` now uses the shared integer/object decoder
and supports the explicit Go JSON byte APIs. All113 independently generated Go
cases pass across the five counters: signed limits, overflow, fractions,
duplicates, null retention, folded keys, unknown values and nesting boundaries.
Native negative-zero and in-place regressions pass alongside existing arithmetic.

Fresh Rust1.88 Linux workspace tests and doctests, strict workspace Clippy/docs/
format, full feature matrix, standalone consumer/purity,20 offline scenarios and
host-session passed. Go oracle vet/build, bound evidence, artifact checks, all51
validators, overlay/audit and immutable source locks passed. The audit initially
rejected an incorrect CLI argument; invoking its documented interface succeeded.
No additional IDs are closed: **867 verified /8,024 unresolved /251 excluded**.
Full issue11 acceptance remains incomplete.

### Host cursor persistence and pinned JSON codec

`host::Cursor` now shares the canonical compatibility record, with Serde
persistence and explicit Go JSON byte decoding/encoding.41 direct pinned-Go cases
verify both fields and exact encoded bytes, including omission, signed limits,
integer negative zero, duplicate/null/folded keys, unknown values, invalid string
replacement and depth limits. Native persistence/in-place behavior and compat-only
embedding also pass. Ordinary i64 Serde rejected `-0` before the existing event
integer seed was shared with the cursor decoder. Four record/member/host-alias
IDs are closed; no other host types or lifecycle capabilities are inferred.

Fresh Rust1.88 Linux: **1,643 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full feature matrix, standalone consumer/
purity,20 offline scenarios, host-session, Go harness tests/vet/build, all bound
oracles,51 validators, artifacts/overlay/audit and source locks passed. Initial
evidence generation correctly rejected the missing explicit host_cursor test
target; after adding it, final generation and validation passed. Ledger:
**867 verified /8,024 unresolved /251 excluded**. All46 remote checks passed on
prior64311231; new-head CI is not yet claimed. Issue11 remains incomplete.

### Host approval gates retain original argument text

`ToolApprovalRequest` now exposes owned `input_json` alongside the parsed
`input`. The adapter copies `ToolCall::argument_text()` instead of making hosts
reconstruct text from a JSON value. A real ChatLoop approval test compares the
pinned Go gate's whitespace-preserving input, duplicate-key/numeric/escape text,
and native value-only fallback. Mutating the gate's owned copy leaves retained
history unchanged. This is a UTF-8 native capability, not a claim that arbitrary
Go byte slices are represented; no additional ledger IDs are closed.

Fresh Rust1.88 Linux: **1,640 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full feature matrix, standalone
consumer/purity,20 offline scenarios, host-session, bound evidence and all51
validators, artifacts/overlay/audit and immutable source locks passed. Ledger
remains **863 verified /8,028 unresolved /251 excluded**. Issue11 is incomplete.

### Host permission normalization uses simple Unicode lowercase

Reproduced the native host parser rejecting `worKspace-write` while the pinned
Go ChatLoop selected workspace-write. The parser now follows simple Unicode
lowercase, including `İ` → `i` rather than Rust's expanded lowercase form.
Unknown nonempty values still fail closed. Nine actual Go ChatLoop observations
and the original eight-input normalization regression pass. Only the original
test ID is closed, not the separate case-sensitive internal policy normalizer,
all host types or whole-file equivalence. Host oracle:50 scenarios/51 runs.

Fresh Rust1.88 Linux: **1,639 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full feature matrix, standalone
consumer/purity,20 offline scenarios, host-session, all bound evidence and51
validators, artifacts/overlay/audit and immutable source locks pass. Ledger:
**863 verified /8,028 unresolved /251 excluded**. Full issue11 acceptance remains
incomplete.

### Metadata discovery decodes paths before normalization

Reproduced a pinned-SDK mismatch: `/%76%31` incorrectly requested
`/%76%31/v1/models` instead of `/v1/models`. Metadata discovery now decodes path
bytes before Unicode-whitespace trimming, inference-suffix removal and `/v1`
detection, then escapes with Go path rules. Non-UTF-8 decoded bytes are preserved.
The independent oracle now compares **252 endpoints**, adding72 encoded-path
cases across standard, ChatGPT, Copilot and explicit-port hosts. The actual HTTP
401-refresh test uses an encoded `/v1/responses` endpoint and verifies both
requests reach `/v1/models`. Scope/transport dot-segment canonicalization and
malformed-escape parity are not claimed; no additional ledger IDs are closed.

Fresh Rust1.88 Linux: **1,637 passed /0 failed /30 ignored;10 doctests passed**.
Strict workspace Clippy/rustdoc/format, full facade feature matrix, standalone
consumer/purity,20 offline scenarios, host-session, refreshed bound evidence,
all51 validators, artifact/overlay/audit and source locks passed. The examples
script's `cargo clippy` launcher failed because this environment lacks
`/proc/self/exe`; the same strict check passed with the direct Clippy driver,
followed by successful example builds and executions. Ledger remains
**862 verified /8,029 unresolved /251 excluded**; issue11 remains incomplete.

### Original Anthropic partial compaction assembly regression

The literal native counterpart of `TestStreamAssemblerAssemblesCompactionBlocks`
feeds message start, compaction start, two content deltas and block stop, then
checks one block, concatenated summary and opaque ciphertext. No terminal event
is invented. Only this test ID is closed. The separate encrypted-content test
omits message start and remains unresolved because native stream lifecycle
validation rejects that sequence.

Fresh Rust1.88 Linux workspace verification: **1,637 passed /0 failed /30 ignored;
10 doctests passed**. All three partial-state tests and strict provider
Clippy/format passed. Refreshed bound evidence, artifact checks, all51 validators,
overlay/audit and immutable source locks passed. Original pinned Go report:
**921 passed /5 skipped across38 packages**. Ledger: **862 verified /8,029
unresolved /251 excluded**. These changes add tests/evidence only; full retained
acceptance is still incomplete.

### Two original Anthropic partial-state regressions

Literal native counterparts now verify the original omitted-input/cache usage and
explicit-false end-turn tests. They inspect partial accumulator state after the
same event sequences, without introducing terminal/content events absent from the
original tests. Only these two test IDs are closed, not the assembler API, whole
source file, terminal-response acceptance or lifecycle permissiveness.

Both native tests, strict provider lint/format, refreshed bound evidence,
51 validators, artifact/overlay/audit and source-lock checks pass. Original pinned
Go report: **920 passed /5 skipped across38 packages**. Ledger: **861 verified
/8,030 unresolved /251 excluded**. No production code changed after the full
1,634-test/10-doctest verification below.

### Anthropic usage accumulator parity

Native Anthropic SSE usage no longer overwrites known input/cache counters with
zero updates. Streaming merges nonzero counters like `StreamAssembler`; completion
keeps initial input/cache counts and replaces output tokens on every message delta,
including zero/omitted output, like the SDK's separate `BetaMessage` accumulator.
This aligns native accounting/context totals with compatibility snapshots rather
than fixing only the trace projection.

An independent HTTP oracle invokes actual pinned Go `Client.CreateMessage` and
`CreateMessageStream`/`CollectResponse` for eleven initial, zero, null, omitted,
nonzero and repeated-update cases. The native HTTP counterpart compares all four
counters and snapshots through both model APIs. It first failed with input tokens
zero instead of1200, then passed after the profile-specific update.

Fresh Rust1.88: **1,634 passed /0 failed /30 ignored**, **10 doctests**; strict
workspace Clippy/rustdoc/format, full facade matrix, standalone consumer/purity,
all20 scenarios, host-session, script checks, refreshed source-bound evidence,
51 validators, artifact/overlay/audit and source locks pass. All46 remote checks
passed on preceding731dc733; newer CI is not yet claimed. Ledger unchanged:
**859 verified /8,032 unresolved /251 excluded**. These valid nonnegative-count
cases do not claim whole-provider or signed/overflowing count equivalence.

### Raw Anthropic output schemas on the HTTP boundary

Anthropic transport now carries the original schema separately from the bounded
request `Value`, through completion/streaming and request-repair attempts. It
preserves duplicate keys, numeric spelling (including overflow-sized literals),
escaped surrogate text and explicit null. Whitespace compaction and HTML/separator
escaping follow the pinned SDK. Public `wire::request` remains a bounded inspection
projection, not a lossless raw encoder; OpenAI schema semantics are unchanged.

Seventeen independently executed pinned-Go marshaler cases include the 9,999-level
schema boundary. The SDK validates the output-format envelope independently of the
root request; applying the limit to the entire request would reject a supported
case. The captured HTTP regression first reproduced duplicate-key loss. It now
matches all cases through both model APIs and bounded effort repair, with excessive
depth rejected before model transport. An intermediate test incorrectly expected
non-streaming Anthropic completion; the corrected test retains the existing
streaming transport default.

Fresh Rust1.88: **1,633 passed /0 failed /30 ignored**, **10 doctests**. Strict
workspace Clippy/rustdoc/format, full facade matrix, standalone consumer/purity,
all20 offline scenarios, host-session and script syntax checks pass. Source-bound
evidence,51 validators, artifact/overlay/audit and immutable source locks pass after
restoring the generator's required continuation marker. All46 CI checks passed on
preceding `e5443b08`; this is not remote verification of the new transport change.
No new ledger closure is inferred: **859 verified /8,032 unresolved /251 excluded**.

### Exact Anthropic output-format regression

The original `TestToSDKParamsSendsOutputFormatForOutputSchema` now has a literal
native counterpart with the same model, token setting, user message and nested
object schema. Serialized output has `output_format.type = "json_schema"` and the
complete schema value. Only this regression ID is closed; raw lexical/depth/Unicode
fidelity and whole provider/file equivalence remain unclaimed.

The native regression and strict provider lint/format pass. The independently
re-executed pinned Go report is **918 passed /5 skipped across38 packages**, now
including this original test. Bound Go/native evidence,51 validators, artifact,
overlay/audit and source-lock checks pass. Ledger: **859 verified /8,032 unresolved
/251 excluded**. No production code changed since the full1,630-test/10-doctest
runtime verification below; issue11 remains incomplete.

### Completion-only runtime reasoning

A following HTTP integration regression exercises the actual provider through a
completion-only runner. The server waits for each observer acknowledgement before
sending the next chunk or final answer. This proves inline end-to-end delivery,
shared generation identity, no reasoning run events and preserved final reasoning
and answer. All four provider reasoning tests pass; no production code changed
from the full verification below.

Blocking completion-only bindings now pass a call-scoped reasoning observer when
`blocking_reasoning` is enabled and a generation observer exists. Nonempty chunks
retain the generation identity and delivery order without becoming streamed run
events. Visible reasoning prevents replay on retry; disabled/missing observers and
empty chunks do not suppress retries. Streamed completion bindings do not install
this blocking observer. Existing activity and idle-timeout bounds remain intact.

The callback lifetime can borrow the generation guard without extending its
lifecycle; a compile-fail doctest prevents promoting it to a static callback. Four
runtime regressions cover ordering/correlation/final response, the eight-way retry
matrix, idle timeout and streamed opt-out. The first three tests failed before the
runner wiring and passed afterward, together with the provider HTTP regressions.

Fresh Rust1.88 workspace: **1,630 passed /0 failed /30 ignored**, **10 doctests**.
Strict workspace Clippy/rustdoc/format, full facade matrix, independent consumer
and dependency purity, all20 offline scenarios and host-session pass. The preceding
OpenAI correction at `f6b62b2e` passed all46 remote checks; that result is not CI
verification of this new runtime change. Refreshed source-bound Go/native evidence,
51 validators, artifact/overlay/audit and immutable source-lock checks pass. Ledger
remains **858 verified /8,033 unresolved /251 excluded**. No whole context/package
or issue closure is inferred from this integration.

### Corrected provider-specific output-schema boundaries

The earlier all-provider scalar-forwarding expectation was wrong. Core/runner
`JsonDocument` acceptance remains correct, but pinned OpenAI `outputSchemaMap`
requires an object, maps JSON null to `{"type":"object"}`, and decodes values with
Go finite-float64 and replacement-string semantics. Chat and Responses now follow
that boundary; Anthropic scalar/array/null forwarding is unchanged. No local JSON
Schema compilation was added. Native127-container value depth and broader raw
Anthropic fidelity remain unclaimed.

Fourteen independent Go cases cover root shapes, null defaults, unknown schema
keywords, overflow (including overwritten duplicates), integer rounding, negative
zero, lone-surrogate replacement and duplicate keys. The native comparator first
failed on null; an intermediate fix exposed `json!` losing negative zero. Direct
Value insertion fixes that loss. Captured HTTP tests verify null normalization
and negative-zero bytes for both OpenAI protocols. The first full run rejected an
empty Responses test fixture; a valid nonempty fixture and the final rerun passed.

Fresh Rust1.88: **1,626 passed /0 failed /30 ignored**, **9 doctests**; strict
workspace Clippy/rustdoc/format, full facade matrix, standalone consumer/purity,
all20 offline scenarios, host-session, bound Go/native evidence,51 validators,
artifacts/audit and source locks passed. Ledger remains **858 verified /8,033
unresolved /251 excluded**. No full schema-map, provider or issue closure is claimed.

### Reasoning callable type evidence

The reasoning callback type alone is now verified, with retained capture, ordered
borrowed UTF-8 inputs (including empty and Unicode strings), and reuse after the
original callable is dropped. Provider tests bind inline delivery, empty filtering,
final response retention and wrapper forwarding. Context installation/extraction,
automatic propagation and whole-package closure are not inferred.

Fresh core targets: **29 passed /0 failed**; strict core Clippy, formatting, bound
Go/native oracles and regressions,51 validators, artifact/audit/source-lock checks
passed. The selected original Go report now includes the sink regression:
**917 passed /5 skipped**,37 packages. No production code changed after3b3827c2's
full **1,623-test /9-doctest** verification. Ledger: **858 verified /8,033 unresolved
/251 excluded**. Issue11 remains incomplete.

### Direct completion reasoning observers

Added `ModelReasoningSink` and optional `Model::complete_with_observers` for
provider completions that consume SSE internally. Providers, Routes and Copilot
forward nonempty reasoning chunks inline without changing the final response or
synthesizing deltas from ordinary JSON. Existing third-party models delegate to
the activity-aware path; runner reasoning continues to use native streaming.
The pre-existing missing-content-type SSE fallback remains buffered, not live.

Three HTTP regressions cover callback-before-completion handshakes, wrapper
forwarding, all three protocols, empty-chunk filtering, both SSE decoding paths
and no JSON replay. The pinned original `TestDrainStreamFeedsReasoningSink`
passed; its Go process emitted a host telemetry-sidecar warning, not a test failure.
Fresh Rust1.88: **1,623 passed /0 failed /30 ignored**, **9 doctests**; strict
workspace Clippy/rustdoc/format, full facade matrix, standalone consumer/purity,
all20 offline scenarios, host-session, bound evidence,51 validators, artifacts,
audit and source locks passed. Preliminary runs were stopped for the empty-chunk
correction and are not the reported final results. All46 remote CI checks passed
on preceding53f35535; this change's remote CI is not yet claimed.
Ledger remains **857 verified /8,034 unresolved /251 excluded**. No context-key,
whole-package or full issue closure is inferred from this bounded implementation.

### Unicode endpoint padding preserves scope identity

Provider scopes trim surrounding Unicode whitespace before URL parsing, matching
Go `strings.TrimSpace`. The pinned metadata oracle now contains180 endpoint
cases, including all25 whitespace characters for API-key/OAuth routes. A native
regression checks credential and prompt-cache identity across all five auth modes
and four endpoints, including an explicit default port. The pre-fix comparator
failed with `invalid provider endpoint`; the corrected comparator passes. Path
unescaping, dot segments and exact transport URL bytes remain unclaimed.

Fresh Rust1.88: **1,620 passed /0 failed /30 ignored**, **9 doctests**; strict
whole-workspace Clippy/rustdoc/format, facade matrix, standalone consumer/purity,
all20 offline scenarios and host-session passed. Independent Go reproduction,
source-bound evidence,51 validators, artifact review, audit and immutable source
locks passed. Ledger remains **857 verified /8,034 unresolved /251 excluded**;
this bounded fix does not establish whole-SDK acceptance or live-provider parity.

### Reviewed callback type and complete regression files

Five additional exact obligations are verified: the modelactivity Sink callable
and four complete test files (activity, static compaction defaults, metadata
compaction defaults and thinking deltas). Every file was reviewed in full, not
closed by module association. The compaction comparators contain all24 original
static cases and all3 metadata cases. Thinking fixtures now include both exact
original coalescing/4096-byte sequences, alongside the six earlier boundary cases
and four real runner attempt records. Runtime correlation and absence of duplicate
streamed JSONL reasoning remain explicitly tested. No package or complete transport
wrapper closure is inferred.

A new shared-capture test joins eight threads and checks800 callbacks plus a
retained invocation. Fresh core targets: **28 passed /0 failed**; the eight-case
native coalescer comparator, all four original Go thinking tests, source-bound
regressions/oracles, strict core/facade Clippy, formatting,51 validators, artifact
review, ledger/audit and source locks passed. Production code is unchanged since
9be85382's full **1,618-test /9-doctest** verification; all46 remote CI checks pass
on that head. No new whole-workspace count is inferred from this test-only slice.
Ledger: **857 verified /8,034 unresolved /251 excluded**. Issue11 remains open.

### Output-schema documents retain JSON shape, spelling and null presence

Core model requests, runtime agents and the facade builder now accept the existing
syntax-validated `JsonDocument` for output schemas instead of restricting them to
Schemars objects/booleans. Native Serde stores the document as a string, preserving
explicit JSON null versus no schema and binding that distinction into durable
configuration identity. Snapshot conversion retains raw schema text; tool/handoff
input-schema validation is unchanged. Updated examples and the external consumer
use the explicit document constructor.

The pinned Go matrix expanded from80 to240 cases across object, invalid-schema
object, null, string, number and array schemas. Native comparisons check exact
request schema text/instructions, parsed outputs and warnings in both run modes.
New regressions cover document round trips, all three provider protocol shapes,
raw snapshots and rejecting durable recovery after absent/null schema changes.
The core regression failed before the change with `expected object or boolean`.
Malformed raw schema bytes remain unsupported; provider Value projection still
limits Unicode/depth and exact wire-token parity. This is not whole-schema closure.

Fresh Rust1.88: **1,618 passed /0 failed /30 ignored**, **9 doctests**; strict
Clippy/rustdoc/format, facade matrix, locked consumer/purity, all20 scenarios and
host-session passed. Independent Go reproduction, bound evidence,51 validators,
artifact checks and source locks passed. Ledger remains **852 verified /8,039
unresolved /251 excluded**. Previous8135dbb8 CI had44 passing checks and two
quality failures from the stale audit report; correctioncfaf3d18 regenerates it.
The latest change's remote CI is not yet claimed. Issue11 is still incomplete.

### Transport progress resets model inactivity without exposing output

Added optional per-call activity callbacks to native model contracts and forwarded
positive response-body reads through direct providers, Routes and Copilot. The
runner resets attempt-local idle timers even when SSE heartbeat bytes produce no
model events. It retains the pending operation, absolute deadlines, cancellation,
retry visibility and host-backpressure semantics; stale callbacks cannot keep a
later attempt alive. Existing third-party models retain delegation defaults.

Five paused-clock regressions cover call modes, stopped activity, absolute limits,
retry eligibility and attempt isolation. Real HTTP chunk/callback handshakes cover
buffered JSON and SSE through all three provider entry points. The deterministic
three-read counterpart closes only the original modelactivity body-read test, not
the whole transport/context API.

Fresh Rust1.88: **1,614 passed /0 failed /30 ignored**, **9 doctests**; strict
Clippy/rustdoc/format, facade matrix, locked consumer/purity, all20 scenarios and
host-session passed. Pinned Go: **916 pass /5 skip**,37 packages. Bound evidence,
51 validators, artifact checks and source locks passed. The first evidence run
rejected an unqualified unit-test name; the corrected fully qualified identifier
passed. Ledger: **852 verified /8,039 unresolved /251 excluded**. Issue11 remains
incomplete; these results do not claim live-provider or whole-SDK parity.

### Explicit default ports retain backend and credential-scope distinctions

Scope canonicalization now retains explicit default/empty ports before reqwest
can erase them. Metadata classification and Codex query decoration use that
preserved distinction. Pinned Go observations expanded to13 backend cases and80
endpoint cases, including443,00443 and empty ports on ChatGPT/Copilot hosts.
Native tests cover API-key/OAuth metadata paths, all auth-mode query guards,
prepared inference/compaction URLs, IPv6/loopback normalization, idempotence and
distinct credential/prompt-cache scopes. The old expectation treating explicit
ChatGPT443 as special was corrected against the independently executed Go result.
Native transport URLs remain canonicalized; no raw URL/Host-header byte claim.

Fresh Rust1.88: **1,607 passed /0 failed /30 ignored**, **9 doctests**; strict
Clippy/rustdoc, Rust/Go formatting, facade matrix, locked consumer/purity, all20
scenarios and host-session passed. Independent metadata reproduction, bound
evidence,51 validators, artifact checks and source locks passed. All46 CI checks
passed on preceding1549abbb; this new tranche's CI is not yet claimed. Ledger
unchanged: **851 verified /8,040 unresolved /251 excluded**; issue11 is incomplete.

### Go-valid raw provider arguments and snapshots

Chat/Responses normalization and snapshot capture now validate Go JSON syntax,
not Serde Value representability. Sixteen independently executed pinned Go
vectors cover empty/malformed input, Unicode whitespace, lone/pair surrogates,
overflowing numeric tokens, duplicates, scalar values and depths128/10000/10001.
Native tests check buffered results, snapshot raw input, streamed arguments and
follow-up requests. Existing streamed outer-whitespace preservation is unchanged.
Valid raw payloads survive even when their native projection is Null.

JsonDocument now validates RawValue syntax rather than constructing a Value;
raw surrogate/deep documents round-trip unchanged through checkpoint-style Value
serialization. Format-specific depth limits remain in codecs. Tests formerly
classifying lone surrogates as malformed were corrected against the Go oracle;
malformed syntax still fails. The provider regression failed before the fix
(`{}` instead of the original surrogate-bearing object). The standalone lockfile
was refreshed for the provider's shared codec dependency.

Fresh Rust1.88: **1,606 passed /0 failed /30 ignored**, **9 doctests**; strict
Clippy/rustdoc/format, facade matrix, locked standalone consumer/purity, all20
scenarios and host-session passed. The example script's cargo-clippy launcher
cannot read /proc/self/exe in this host; equivalent checks passed with the direct
Clippy driver. Independent Go reproduction, bound evidence,51 validators,
artifact checks and source locks passed. No additional acceptance IDs claimed:
**851 verified /8,040 unresolved /251 excluded**. Anthropic handling, opt-in
native schema validation of argument projections and whole-provider parity remain
outside this fix; issue11 is still incomplete.

### Original agent helper examples

Added original-test counterparts for scalar clone overrides, generated handoff
metadata, and stop-at-tools membership. A fourth native regression verifies
explicit tools precede handoffs in a FullAccess model request; it does not claim
Go's unconditional GetAllTools helper or mutable collection aliasing parity.

Fresh Rust1.88 validation: **1,602 passed /0 failed /30 ignored**, **9 doctests**;
strict workspace Clippy/rustdoc/format, facade matrix, standalone consumer/purity,
all20 scenarios and host-session passed. Pinned Go report: **915 pass /5 skip**
in36 packages. Bound evidence,51 validators, artifact checks and source locks
passed. Ledger: **851 verified /8,040 unresolved /251 excluded**. All46 CI checks
passed on preceding791e37f; this new tranche's remote checks are not yet claimed.
Issue11 remains incomplete.

### Typed float negative-zero serialization

The shared DTO float serializer no longer reparses its finite formatted number,
which changed Go's `-0` to `0`. Eleven independent pinned Go ModelResponse cost
observations store f64 bits as hexadecimal strings and exact JSON tokens as
strings, covering signed zero, notation boundaries and finite extrema. Native
checks compare actual Serde and Go-compatible encoded bytes through RawValue.
The negative-zero byte regression failed before the fix and now passes.

Serde's generic `to_value` projection still reparses the private Number payload
and loses negative zero; that projection is not authoritative for exact bytes.
The old test is renamed to describe its projection assertions, which remain
unchanged. Nonfinite rejection and field omission rules are unchanged. This
adds no whole-format or generic-value round-trip claim.

Fresh Rust1.88 validation: **1,598 passed /0 failed /30 ignored**, **9 doctests**;
strict workspace Clippy/rustdoc, Rust/Go formatting, facade matrix, standalone
consumer/purity, all20 scenarios and host-session passed. Independent fixture
reproduction, bound evidence,51 validators, artifact checks and source locks
passed. Ledger unchanged: **848 verified /8,043 unresolved /251 excluded**.
Latest checked7c57bf5e CI:44 passed,0 failed,2 pending; no complete-CI claim.
Issue11 remains incomplete.

### Pruned approvals cannot authorize later calls on recovery

Fixed durable approval restoration to exclude journal entries with no current
history anchor. These entries remain necessary for audit/new-item encoding,
but pruning had left them eligible to authorize later identical calls. The
regression uses real checkpoints: an approved handoff prunes tool history, the
next agent emits the same ID/name/arguments, and recovery occurs before its
approval. Before the fix, the fresh approval host was called zero times instead
of once. Recovery now asks the host, pauses on defer, and transfers only after
explicit new approval. No fabricated checkpoint or weakened duplicate-ID guard
is involved. Existing completed-effect and paused-approval recovery tests pass.

Fresh Rust1.88 validation: **1,597 passed /0 failed /30 ignored**, **9 doctests**;
strict workspace Clippy/rustdoc, formatting, facade matrix, standalone consumer
and purity, all20 scenarios and host-session example passed. Bound Go/native
evidence,51 validators, artifact checks and source locks passed. Ledger remains
**848 verified /8,043 unresolved /251 excluded**; this fix adds no broad parity
claims and does not complete issue11.

### Filesystem contention stress-test budget

Windows CI job117228622620 on1b645024 failed in the six-writer project-state
stress test after a default30-second lock wait; the same-head duplicate job
passed. The test now gives its filesystem writers120 seconds, while retaining
all48-comment/48-memory and single-initialization assertions. Production defaults
and the separate25ms lock-timeout regressions are unchanged. Every writer is
joined before a panic is propagated, keeping the temporary directory alive
rather than causing secondary missing-file failures in still-running writers.

Fresh Linux project-state all-feature/all-target checks: **39 passed /0 failed**;
strict Clippy and formatting passed. Ledger/audit remain valid at848/8043/251.
Both Windows unsupported-target jobs passed on c18596bc (117232061705 and
117232048546), as did both macOS/Linux enforcing-backend jobs. At that check,
22/46 jobs had passed,24 remained pending and none had failed; overall CI was
not yet complete. No additional acceptance IDs are closed.

### Turn-limit error text and retained partial results

All three native turn-limit exits now include the effective limit using the
SDK's `max turns exceeded: N` spelling. A regression first observed the old
`maximum model turns exceeded` text. Buffered and streamed runs at limits10/50
now verify exact text, category, call counts, retained responses, paired partial
history and absence of an injected turn-budget warning. Budgeting is unchanged.
Only original `TestMaxTurnsExceededError` is newly closed.

The first attempt also exposed a separate boundary: pinned
`TestRunnerMaxTurnsExceeded` repeats the same call ID each turn, while native
history validation rejects cross-turn duplicate IDs. Both original Go tests
passed independently. Native coverage uses unique IDs; the original runner test
remains unresolved. Durable checkpoint validation also requires unique IDs, so
relaxing that guard needs separate execution/approval-correlation review.

Fresh Rust1.88: **1,596 passed /0 failed /30 ignored**, **9 doctests**; strict
Clippy/rustdoc, formatting, facade matrix, standalone consumer/purity, all20
scenarios and host-session passed. Original Go: **913 passed /5 skipped**,36
packages. Bound evidence,51 validators, artifacts and source locks passed.
Ledger: **848 verified /8,043 unresolved /251 excluded**.

Remote CI on preceding1b645024 reported a Windows stress-test failure:
`separate_handles_do_not_lose_updates_or_initialization` hit the30-second
filesystem `LockTimeout`; its duplicate same-head Windows job passed. This
failure remains under investigation, not a clean CI claim. Full issue11
acceptance remains incomplete.

### Original result and usage helper examples

Added exact counterparts for `TestRunResultFinalText`, `TestRunResultIsInterrupted`
and `TestUsageAddAndTotal`. The tests retain the original hello/42 outputs,
absent/present bash interruption, and 100+200 input /50+100 output arithmetic.
The interruption example maps to native unified pending approvals; it does not
claim equivalence for contradictory Go singular/plural interruption fields.
Only these three original test IDs are newly closed.

Fresh Rust1.88 validation: **1,595 passed /0 failed /30 ignored**, **9 doctests**;
strict workspace Clippy/rustdoc, formatting, facade matrix, independent
consumer/purity, all20 scenarios and host-session example passed. Original Go:
**912 passed /5 skipped**,36 packages. Bound evidence,51 validators, artifact
checks and source locks passed. Ledger: **847 verified /8,044 unresolved /251
excluded**. Full issue11 acceptance remains incomplete.

### Original model-settings merge and output-schema constructor tests

Added exact native counterparts for pinned SDK `TestModelSettingsMerge` and
`TestOutputSchemaNew`: preserve temperature, override max tokens, carry reasoning
and verbosity, and retain the supplied schema name with strict mode enabled by
default. These close only the two original test IDs, not the complete constructor,
raw-schema or mutable-alias API contracts.

Fresh Rust1.88 workspace verification: **1,592 passed /0 failed /30 ignored**,
**9 doctests**. Strict Clippy/rustdoc, formatting, facade matrix, standalone
consumer/purity, all20 feature scenarios and host-session example passed.
Pinned original Go execution: **909 passed /5 skipped**,36 packages. Bound
reference/native evidence,51 ledger tests, artifact checks and source locks
passed. Ledger: **844 verified /8,047 unresolved /251 excluded** across9,142 IDs.
Issue11 remains incomplete and PR33 remains draft.

### Untyped JSON numbers, duplicates and ordinary object keys

Added `adk::codec::value::decode` and used it for default output/handoff parsing.
It normalizes Go strings, decodes finite float64 numbers, validates duplicate
object entries before replacement, and preserves literal Serde marker names as
ordinary keys. Parsed numbers use Go JSON spelling. Re-parsing the generated
number spelling lost negative zero in the first implementation; the decoder now
constructs the number only from validated finite-float serializer output, retaining
`-0`. Custom parsers and callback raw input remain unchanged.

The independent actual-Go matrix has **80 final-output cases** and **26 handoff
cases**, the latter each run natively with tool validation on and off. Output
assertions now compare exact Go JSON stored as a string, not Python-normalized
JSON values. Cases include 2^53 rounding, overflow, overflow hidden by a later
duplicate, negative zero, float notation and private marker keys. The regression
first retained `9007199254740993` rather than Go's `9007199254740992`; after fixing
that, the exact-byte check exposed the negative-zero loss. Both now pass.

Three codec tests cover ordered validation, marker keys and the unchanged native
nesting bound; a compat-only facade test checks rounding and negative zero.
The decoder deliberately retains the existing **127-container** limit. Go's
10,000-container limit, schema shape restrictions and complete generic-value,
durable-serialization or provider equivalence remain unclaimed.

Fresh Rust1.88 validation: **1,590 passed /0 failed /30 ignored**, **9 doctests**;
strict workspace Clippy/rustdoc, formatting, facade matrix, independent consumer
and purity, all 20 feature scenarios and host-session example passed. Bound
reference/native evidence,51 validators, ledger/audit and source locks passed.
The new source is hash-bound; license artifact evidence was separately refreshed
after changing the validator inputs. Ledger remains **842 verified /8,049
unresolved /251 excluded**. This is not full issue11 completion.

### Handoff input-schema construction and Unicode

The handoff input-type path now also forwards its optional schema without local
compilation and uses Go-compatible string replacement during default advisory
parsing. Parser return values remain discarded and callbacks retain the original
raw input, even when the existing `ToolCall` value projection is null because
ordinary Serde cannot represent that input. The projection is not input authority.

Four new actual-Go buffered/streamed observations bring the handoff matrix to
**22 cases**, including a lone escaped surrogate and `{"type":27}`. Every case
runs natively with ordinary-tool argument validation both enabled and disabled,
asserting exact callback text, event/parser order, warning count, declared schema,
last agent and final text. The Unicode case first produced an extra warning;
after that fix, the schema case exposed a separate constructor rejection. Both
regressions now pass. Custom-parser behavior and ordinary tool validation are
unchanged. Native schema shapes, arbitrary numeric/depth behavior and broader
provider/raw-byte parity remain unclaimed.

Fresh full workspace: **1,586 passed /0 failed /30 ignored**, **9 doctests**.
Strict workspace Clippy/rustdoc, formatting, facade matrix, standalone consumer
and purity, all 20 feature scenarios and host-session example passed. Independent
Go/native bound evidence,51 validators, artifact reviews, ledger/audit and source
locks passed. Ledger unchanged: **842 verified /8,049 unresolved /251 excluded**.

### Output-schema construction without local compilation

Removed native output-schema compilation from `Runner::new`. The pinned SDK
accepts the schema as supplied and its default output validation only parses JSON.
The independent Go runner matrix now has **52 cases**, applying both the ordinary
object schema and `{"type":27}` to all buffered/streamed output cases. The native
regression first failed during construction; it now matches Go final values and
warnings. A facade regression separately verifies no dispatch during construction,
unchanged schema forwarding, successful parsed output and explicit owned closure.
Tool-schema validation remains intact; handoff input-type compilation is unchanged.
The native `schemars::Schema` object/boolean restriction, arbitrary JSON numbers
and depth, and complete output-schema equivalence remain unclaimed.

Three late-parent-failure lifecycle regressions previously used the now-accepted
output schema as their failure trigger. They now enable a parent input signal
without a poller and assert the exact configuration error. Automatic children do
not inherit input admission, so initialization still succeeds before the parent
fails; owned graph cleanup and borrowed-session/task preservation assertions are
retained. No cleanup test was skipped or weakened to permit a successful build.

Fresh Rust1.88 checks: **1,586 workspace tests passed /0 failed /30 ignored**, plus
**9 doctests**. Strict workspace Clippy/rustdoc, formatting, full facade matrix,
standalone consumer/purity, all20 feature scenarios and the host-session example
passed. Bound Go/native evidence,51 validators, artifact reviews, ledger/audit and
source locks passed. No new ledger closure: **842 verified /8,049 unresolved
/251 excluded**; issue11 remains incomplete.

### Structured-output Unicode replacement

Default structured-output parsing now reuses the existing codec string
normalization, replacing lone escaped UTF-16 surrogates instead of rejecting
Go-valid JSON. Custom parsers still receive the original text unchanged. Ten
additional buffered/streamed Go observations bring the final-output matrix to
**26 cases**: lone high/low surrogates in values and keys, valid pairs, escaped
literal backslashes and a lone surrogate followed by a valid pair. The native
regression first returned the original quoted escape instead of Go's replacement
character, then passed with the fix. This is not a claim about arbitrary JSON
number/depth behavior, non-UTF-8 native strings or constructor equivalence.

Fresh full Rust validation: **1,586 passed /0 failed /30 ignored**, **9 doctests**;
strict workspace Clippy/rustdoc, formatting, full facade matrix, standalone
consumer/purity, all20 feature scenarios and the host-session example passed.
Bound independent/native evidence,51 ledger validators, audit and source locks
passed. No new ledger closure: **842 verified /8,049 unresolved /251 excluded**.

### Structured-output parsing does not enforce JSON Schema

Removed the extra native `OutputValidationFailed` observation for parseable JSON
that violates the output schema. The pinned SDK's default validator parses JSON;
it does not enforce the schema. Empty final text also bypasses validation. Custom
parser success/failure handling is unchanged. The executable feature example and
builder/example documentation now reflect this behavior instead of asserting the
incorrect extra warning.

The existing independent handoff/input-schema probe now also executes **16 final
output cases** through the actual Go runner, buffered and streamed. Cases cover
schema-valid and schema-invalid objects, strings, arrays, booleans, null, malformed
JSON and empty output. The native regression failed before the fix (one warning
versus Go's zero for `{"n":-1}`), then matched all final values and warning counts.
Constructor-time schema rejection, arbitrary JSON number/Unicode/depth behavior
and complete output-schema API equivalence remain unclaimed.

Fresh full workspace: **1,586 passed /0 failed /30 ignored**, plus **9 doctests**;
strict workspace Clippy/rustdoc, formatting, facade matrix/standalone consumer and
purity passed. All **20 executable feature scenarios** and the owned host-session
example passed after correcting the example expectation. Final full workspace,
bound reference/native evidence, ledger/audit and source locks passed again.
The 51 ledger validators passed. An initial evidence invocation lacked explicit
`GOROOT` and failed before running its Go schema probe; setting `GOROOT=/usr/local/go`
restored the probe and the complete evidence run passed. All 46 remote checks for
the preceding pushed head `20029370` passed; this is not a claim about a later head.
No new ledger closure: **842 verified /8,049 unresolved /251 excluded**.

### Explicit compatibility item extraction helpers

Added `adk::codec::items::{extract_text, extract_last_text, extract_tool_calls}`
for constructed compatibility `RunItem` values. Text helpers preserve the SDK's
accumulator-dependent separators and last-message selection, including empty
messages and missing/mismatched payloads. Tool-call extraction preserves nil
results as `None` and every raw input byte without validation; returned calls own
cloned values. Mutable Go backing-array aliases remain unclaimed, as does the
complete `ExtractToolCalls` API and enclosing item types/file.

An independent pinned-Go probe supplies **136 item lists** (including all three
original extraction regressions and 125 empty/whitespace/newline/Unicode triples)
and **five raw-input cases**, including missing versus empty, invalid UTF-8 and
invalid JSON. Native assertions check text, ordered call IDs/names/counts/nilness
and exact raw hex bytes. The latter avoids Python fixture JSON normalization of
`1e0` into `1.0`; a parsed numeric projection is not the raw-input authority.
The initial test mistakenly used the sealed decoder on unsupported `RunItem`;
canonical fixture construction now uses ordinary Serde without claiming general
Go root-item decoding. The compat-only facade smoke test passes.

Exactly **two text-helper APIs and three original source tests** are newly
verified. Original pinned Go was freshly rerun: **907 passed /5 skipped** across
36 packages. Fresh full Rust workspace: **1,586 passed /0 failed /30 ignored**,
157 targets, plus **9 doctests**. Strict workspace Clippy/rustdoc, the feature
matrix, standalone consumer and purity checks pass. A final formatting check
caught module ordering; after correction, formatting and the complete workspace,
bound oracle/evidence, 51 validators, artifact reviews and source locks passed
again. Ledger: **842 verified /8,049 unresolved /251 excluded**. Issue11 remains
incomplete.

### Direct endpoint oracle and Copilot non-default ports

A new independent pinned internal-Go probe executes `modelMetadataEndpoint`
directly for50 ChatGPT/Copilot/ordinary-host and base-path combinations. This
exposed the analogous Copilot-port error: metadata discovery used `/models`
instead of `/v1/models` for `api.githubcopilot.com:8443`. Both recognized Copilot
hosts now use the ordinary versioned path when a non-default port is present.
The pre-existing native matrix's incorrect port expectation was corrected, not
skipped. The new regression failed before the fix and passes afterward.

The oracle's source and new harness file are hash-bound in the evidence runner;
separate license artifact evidence was refreshed after that validator edit.
Explicit default ports removed by `Scope::new`, arbitrary raw URL normalization
and full auth/inference behavior remain unverified; no new broad claim is made.

Fresh full workspace: **1,583 passed /0 failed /30 ignored**,156 targets, plus
**9 doctests**. Strict workspace Clippy/rustdoc, formatting, full feature matrix,
standalone consumer/purity, bound native/oracle evidence,51 validators, artifact
reviews and source locks pass. Ledger remains **837 verified /8,054 unresolved
/251 excluded**. Issue11 is not complete.

### Non-default-port ChatGPT classification

Ten independently executed Go backend-classification vectors exposed incorrect
native handling of `chatgpt.com:8443` and its subdomains. Go's host predicate
includes the port. Metadata discovery now adds the ordinary `/v1` segment for
these URLs, and shared Codex URL decoration no longer adds `client_version`.
Existing native metadata and prepared-inference tests that incorrectly expected
port-independent classification were corrected against this source evidence.
The new regression failed before the fix; the complete final rerun passes.

This is deliberately bounded: `Scope::new` still canonicalizes explicit default
ports away, and complete original-input URL normalization, Copilot port handling
and general host/auth parity remain unresolved. No broader endpoint/helper
ledger record is promoted. The Go oracle now additionally binds the exact
`auth_session.go` and `client.go` source hashes.

Fresh full workspace: **1,582 passed /0 failed /30 ignored**,156 targets, plus
**9 doctests**. Strict workspace Clippy/rustdoc, formatting, full feature matrix,
standalone consumer/purity, bound native/oracle evidence,51 validators, artifact
reviews and source locks pass. Ledger remains **837 verified /8,054 unresolved
/251 excluded**. Issue11 is not complete.

### Public in-place metadata ordering

Added `sort_model_metadata_for_picker(&mut [ModelMetadata])` and reused it in
the existing filtering picker. It retains hidden entries and stably orders by
positive priority, then Go-compatible simple lowercase ID. Twenty-six independent
Go observations rotate empty/singleton, signed-priority, Unicode, whitespace and
case-folded tie inputs; description markers verify stable identity. Native tests
compare both complete sorted records and filtered picker records and confirm the
filtering picker leaves its input unchanged.

The first oracle build exposed that the Go public provider wrapper does not
re-export the in-place helper; the harness now calls the pinned internal helper
directly. No source implementation was modified. Only that internal helper's
ledger record is newly verified, not the complete metadata/provider surface.

Fresh full workspace: **1,581 passed /0 failed /30 ignored**,156 targets, plus
**9 doctests**. Strict workspace Clippy/rustdoc, formatting, full feature matrix,
standalone consumer/purity, bound native/oracle evidence,51 validators, artifact
reviews and source locks pass. Ledger: **837 verified /8,054 unresolved /251
excluded**. Issue11 remains incomplete.

### Signed resolved-context helper

The pinned SDK returns `MaxContextWindow` unchanged when `ContextWindow` is
nonpositive, including a negative maximum. The previous Rust helper discarded
that result. Its public return type is now `i64` rather than `Option<u64>`;
missing fields resolve to zero. Consumers requiring a usable capacity should
check `> 0`. Compaction retains that positive-window guard and its existing
signed wrapping arithmetic.

Independent Go observations now cover 363 helper/compaction cases, including
both signed extrema for primary and maximum windows. The native regression
failed before the correction (`0` versus `-1`) and passes afterward. This closes
only `ModelMetadata.ResolvedContextWindow`, not the enclosing type or parser.

Fresh full workspace: **1,580 passed /0 failed /30 ignored**,156 targets, plus
**9 doctests**. Strict workspace Clippy/rustdoc, formatting, feature matrix,
standalone consumer/purity, bound native/oracle evidence,51 validators, artifact
reviews and source locks pass. Ledger: **836 verified /8,055 unresolved /251
excluded**. Issue11 remains incomplete.

### Metadata Unicode replacement and original HTTP bytes

Twelve new independently executed Go cases exposed rejection of lone UTF-16
surrogates and invalid UTF-8 that Go accepts with replacement. Metadata input now
preserves valid surrogate pairs and replaces lone surrogates and each invalid
UTF-8 byte as Go does. Literal escaped backslashes remain literal; malformed
escapes and invalid structural bytes still fail. The regression failed before
the fix and passed afterward.

The catalog oracle contains57 cases. Five non-UTF-8 responses carry explicit
base64 originals because Go's fixture JSON encoder would otherwise replace their
invalid bytes. Both direct decoding and actual Rust HTTP fetches use those original
bytes, not the normalized display string. Authentication and the original-byte
8MiB response limit are unchanged. No complete parser/type/file claim is added.

Fresh full workspace: **1,580 passed /0 failed /30 ignored**,156 targets, plus
**9 doctests**. Strict workspace Clippy/rustdoc, formatting, full feature matrix,
standalone consumer/purity, all bound oracles,51 validators, artifact reviews and
source locks pass. Ledger remains **835 verified /8,056 unresolved /251 excluded**.
Issue11 remains incomplete.

### Metadata JSON field and merge semantics

The pinned-Go catalog oracle now contains45 cases, including27 new cases for
case-insensitive/Unicode-folded field names, duplicate scalar/object merging,
null retention versus pointer reset, reused slice elements after shortening and
regrowing arrays, and retaining an earlier type error despite later valid values.
The native regression failed on mixed-case Codex fields before the fix.

A metadata-local ordered raw-field decoder now implements those merge rules.
Output mapping, first model-ID deduplication, authentication and the8MiB response
limit are unchanged. Unknown-field nesting matches Go at10,000 levels and rejects
10,001. Every catalog case also traverses an actual Rust loopback HTTP fetch.
Invalid UTF-8/lone-surrogate replacement remains unverified; no complete parser
or enclosing file claim is promoted.

Fresh full workspace: **1,580 passed /0 failed /30 ignored**,156 targets, plus
**9 doctests**. Strict workspace Clippy/rustdoc, formatting, full feature matrix,
standalone consumer and platform-purity checks pass. All bound native/oracle
evidence,51 validators, license artifact reviews and source locks pass. Ledger
remains **835 verified /8,056 unresolved /251 excluded**; issue11 is incomplete.

### Streamed OpenRouter reasoning detail bytes

Stream assembly now retains raw reasoning-detail fragments and applies the
existing Go-compatible JSON compactor before encoding signatures, instead of
encoding parsed maps. The semantic native raw payload remains unchanged. Five
independent pinned-Go stream vectors cover member order, duplicate fields, number
spelling, whitespace, escapes and HTML/Unicode separators; the SSE decoder runs
in17-byte chunks. Buffered vectors also cover the separator/HTML case. The new
stream regression failed before the fix and passes afterward.

The original OpenRouter stream/cache regression now checks exact decoded bytes,
not merely equivalent JSON. This closes only that original source-test record;
interleaved reasoning blocks, outbound detail byte preservation and full stream
implementation equivalence remain unclaimed.

Fresh full workspace: **1,579 passed /0 failed /30 ignored**,156 targets, plus
**9 doctests**. Original pinned Go: **904 passed /5 skipped**,36 packages.
Strict workspace Clippy/rustdoc, formatting, feature matrix, standalone consumer,
purity, all bound oracles,51 validators and source locks pass. Evidence initially
rejected unqualified new test identifiers; the fully qualified bindings and
complete rerun passed. Ledger: **835 verified (833 executable-tested +2
artifact-reviewed) /8,056 unresolved /251 excluded**. Issue11 remains incomplete.

### Deterministic cancellation reference observation

Reproduced the intermittent pinned-Go subagent exporter failure and captured the
actual diff: four cancellation views sometimes contained `agent "slow" cancelled:
context canceled` instead of `cancellation requested`. The model could return
between cancellation and view collection; waiting on the cancelled registry task
was not a runner-return barrier. The fixture now holds that model return until
its intended cancellation-requested observations have been captured, then releases
it. No errors are normalized away and no alternative output is accepted.

**200 consecutive independent Go regenerations matched** after the barrier.
The golden changed only its generator provenance hash; all six scenarios and
their expected response fields are unchanged. This resolves the previously
unexplained exporter race documented below, not a production scheduler change.
Go vet/build and formatting pass; all six native subagent-reference tests, the
complete bound native/oracle evidence run,51 validators, source locks and
ledger/audit checks pass. Ledger remains834 verified/8,057 unresolved/251 excluded.

### Buffered OpenRouter reasoning signature bytes

A new independently executed four-case Go oracle reproduced byte loss in buffered
Chat reasoning signatures: canonical JSON reordered detail members before base64
encoding. Native reasoning and diagnostic snapshots now encode the original
`reasoning_details` bytes on the HTTP/`response_json` path. Tests cover ordering,
whitespace, number spelling, escapes and duplicate members, including actual
loopback HTTP responses. The regression failed before the fix and passes after.
Value-only callers retain their already-canonical representation. Streaming and
outbound structured-detail byte preservation remain unresolved; no original
roundtrip test or broader ledger obligation is newly promoted by this fix.

Fresh full workspace: **1,578 passed /0 failed /30 ignored**,156 targets, plus
**9 doctests**. Strict workspace Clippy/rustdoc, formatting, feature matrix,
standalone consumer and platform-purity checks pass. Bound oracle/native evidence,
51 validators and source locks pass. The first evidence run failed in the
unchanged subagent exporter; isolated reproduction and the complete rerun passed
without fixture changes. Its intermittent failure remains unexplained, not
silently waived. Ledger remains **834 verified /8,057 unresolved /251 excluded**.

### Responses tool-output pairing

The adapter now drops results without a previously emitted call, normalizes call
IDs, generates missing call IDs from the emitted input position, and substitutes
`(no output)` for whitespace-only Responses results. Images survive orphan-result
filtering. Chat and Anthropic behavior is unchanged. Ten independently executed
pinned Go vectors compare complete input arrays, and a loopback HTTP regression
checks the actual request. The original orphan-result regression failed before
the fix and passed afterward. Only the two original orphan/matched-result tests
are newly claimed; this does not close the enclosing provider implementation.

Fresh Rust1.88 full workspace: **1,576 passed /0 failed /30 ignored**,156 targets,
plus **9 doctests**. Strict workspace Clippy/rustdoc, formatting, facade feature
matrix, standalone consumer and platform-purity checks pass. The first attempt
found a borrow error in the new HTTP test; the corrected full rerun passed.
Original pinned Go: **903 passed /5 skipped**,36 packages. All bound native/oracle
evidence,51 validators, source locks and ledger/audit checks pass. Ledger:
**834 verified (832 executable-tested +2 artifact-reviewed) /8,057 unresolved
/251 excluded**. Issue11 remains incomplete; unavailable live/native-target
checks below remain unverified.

### Original streamed Chat regression coverage

Added nine exact original source-test mappings for streamed reasoning/text,
opaque signatures, narration-only tool-call finish reasons, fragmented and
multiple tools, reasoning followed by tools, complete reasoning conversion,
plaintext/signature request roundtrips, and Copilot nested-budget omission.
The SSE cases run through the actual decoder in17-byte chunks; Copilot checks
an actual loopback request at stream creation. Production code is unchanged.

Two additional OpenRouter tests establish semantic JSON/detail/cache roundtrips
only. Their original byte-preservation assertions and the choice-error body
assertions remain unverified; the enclosing source file is not closed.

Fresh bound native/reference evidence, strict provider all-target Clippy,
formatting,51 validators and source locks pass. Original pinned Go: **901 passed
/5 skipped**,36 packages. Full workspace/matrix results below apply to53e8db66,
before these ten additional native test cases. Ledger: **832 verified (830
executable-tested +2 artifact-reviewed) /8,059 unresolved /251 excluded**.
Issue11 remains incomplete.

### Chat thinking-type gate fixed against independently executed Go

A new 30-input/60-projection oracle reproduced Chat incorrectly emitting a nested
budget when thinking was disabled. The one-line fix gates nested Chat budgets on
exact type `enabled`, while retaining the pinned Responses behavior that ignores
type. Negative/zero/positive budgets, empty/disabled/adaptive/case-varied types,
and explicit-effort precedence match Go. Explicit `thinking_budget` settings are
unchanged. The original enabled-budget test and all seven original effort cases
also pass; four exact test/subtest/file obligations are newly verified.

Provider continuation fixture generation now has a non-overwriting `--check`
mode. Evidence requires independent reproduction and binds generator/probe/fixture
hashes; the validator rejects a missing reproduction marker.

Fresh Rust1.88 full workspace: **1,563 passed /0 failed /30 ignored**,156 targets,
plus **9 doctests**, zero warnings. Strict workspace Clippy/rustdoc, facade feature
matrix, standalone consumer/purity, all bound oracles,51 validators, formatting,
shell syntax and source locks pass. Original pinned Go: **892 passed /5 skipped**,
36 packages. The regression failed before the fix and passed afterward.

Ledger: **823 verified (821 executable-tested +2 artifact-reviewed) /8,068
unresolved /251 excluded**. Issue11 remains incomplete; no broader client or
metadata-parser closure is claimed.

### Original chat audit regressions

Five native tests now reproduce every assertion in the original
`audit_fixes_test.go`: nonempty complete/streamed refusal text, the three
truncation/tool-call stop-reason combinations, multipart base64 PNG forwarding,
and plain-string text-only messages. Production behavior passed unchanged.
Six exact test/file claims are added; general chat codec parity is not claimed.

Fresh bound native/reference evidence, strict provider all-target Clippy,
formatting,51 validators and source locks pass. Pinned Go: **883 passed /5
skipped**,36 packages. The full workspace/matrix results immediately below apply
to preceding c0131fc3, before these five additional test-only regressions.
Ledger: **819 verified (817 executable-tested +2 artifact-reviewed) /8,072
unresolved /251 excluded**. Issue11 remains incomplete.

### Model fallback serialization and actual HTTP calls

Added all four original fallback-table cases, the exact JSON omission/array
assertions, and actual loopback HTTP calls with and without fallback models.
These reproduce all four original tests in `model_fallback_test.go`; six
source-test/subtest/file records are closed without claiming complete client
configuration or retry behavior. Existing production behavior passed unchanged.

Fresh Rust1.88 workspace: **1,556 passed /0 failed /30 ignored**,156 targets,
plus **9 doctests**, zero warnings. Strict workspace Clippy/rustdoc, facade feature
matrix, standalone consumer/purity, bound independent evidence,51 validators and
source locks pass. Original pinned Go: **878 passed /5 skipped**,36 packages.
Ledger: **813 verified (811 executable-tested +2 artifact-reviewed) /8,078
unresolved /251 excluded**. Issue11 remains incomplete.

### Explicit Chat Completions model validation

Added `factory::validate_chat_completions_model` for host-invoked validation,
using the existing compatibility predicate and actionable `InvalidInput` errors.
Diagnostics omit caller-supplied model text consistently with the Rust provider
boundary; exact Go error-string equivalence remains unresolved. Requests are not
implicitly rejected or rerouted by this helper.

The original test file's two assertions are now reproduced in Rust, closing
four test/subtest/file records, not the full API contract. Fresh checks: **156
provider tests +1 doctest**, strict provider all-target/all-feature Clippy and
rustdoc, formatting, all bound native/reference evidence,51 validators and
source locks pass. Original pinned Go: **870 passed /5 skipped**,36 packages.
The first evidence attempt failed at the unrelated subagent fixture check;
its isolated check and the complete evidence rerun passed without fixture changes.
No full-workspace rerun is claimed for this small helper addition.

Ledger: **807 verified (805 executable-tested +2 artifact-reviewed) /8,084
unresolved /251 excluded**. Issue11 remains incomplete.

### Complete original metadata test-file coverage

Added an isolated counterpart of the original Copilot max-prompt-token fallback
regression, preserving its payload and exact 200000 context-window assertion.
All eight original metadata tests now support the test-file acceptance claim;
this does not close the production parser, transport, or provider package.

Fresh pinned Go execution: **867 passed /5 skipped**,36 packages. Native bound
evidence, strict provider all-feature/all-target Clippy, Rust2024 formatting,
51 ledger/audit validator tests and overlay/audit checks pass. No production code
changed in this checkpoint. Ledger: **803 verified (801 executable-tested +2
artifact-reviewed) /8,088 unresolved /251 excluded**. Issue11 remains incomplete.

### Standard metadata discovery and exact source regressions

Five additional native regressions reproduce every assertion in the original
Codex/Copilot catalog, picker, alphabetical-list and standard HTTP discovery tests.
The HTTP regression exposed and fixed root-base discovery requesting `/models`
instead of `/v1/models`. Parsed-path normalization now retains ordinary versioned
base paths and the ChatGPT/Copilot exceptions, with a 26-case endpoint matrix.
Configured queries/fragments remain rejected and hostname guards ignore explicit
ports; complete Go URL normalization equivalence is not claimed.

Scoped independent review approved the five test mappings and production fix.
Rust1.88 workspace: **1,550 passed /0 failed /30 ignored** across156 non-doc
targets, plus **9 doctests**. Strict workspace Clippy/rustdoc, full facade matrix,
standalone consumer/purity, all bound oracles,51 validators and source locks pass.
Pinned Go: **867 passed /5 skipped**,36 packages.

Ledger: **802 verified (800 executable-tested +2 artifact-reviewed) /8,089
unresolved /251 excluded**. These five original tests—not the enclosing metadata
file/type/parser or general transport—are newly closed. Issue11 remains incomplete.

### Codex client version and exact OpenAI regressions

Added immutable session-owned Codex client-version configuration, shared by the
metadata resolver and the actual inference/compaction/retry request path. Blank
values restore 0.153.4; nonblank values are trimmed and URL-encoded. OAuth/ChatGPT
host guards, endpoint validation, auth headers and credential/cache identity stay
intact. First query-value semantics match Go; arbitrary preexisting-query byte
ordering is not claimed, and configured endpoint queries remain prohibited.
Scoped review approved the production wiring and refresh/rotation isolation.

Added exact native counterparts for all **21 CalculateCost +15 EstimateCost**
source cases, including every expected amount/known flag and the original 1e-9
tolerance. Original internal OpenAI tests now run from explicit, package-bound
claim selections; tests reject regex/subtest expansion and preserve historical
agent regressions. Existing metadata catalog evidence is rebound only to ten
specific fields/methods and four complete original regressions, not whole parser
or metadata-type parity.

Fresh Rust1.88 workspace: **1,544 passed /0 failed /30 ignored**,156 targets,
**9 doctests**, no warnings. Strict workspace Clippy/rustdoc, full facade matrix,
standalone consumer/purity, every bound oracle, separate license artifact proof,
and **51 validators** pass. Original pinned Go: **865 passed /5 skipped** across
36 packages. All46 remote checks passed at preceding `1ea305a6`; latest-head CI
is not yet claimed complete.

Exactly **25 further records** are verified:14 existing narrow provider records,
five complete cost-test/table/file obligations, and six exact version/default
configuration mappings. Ledger: **797 verified (795 executable-tested +2
artifact-reviewed) /8,094 unresolved /251 excluded**. Remaining metadata source
assertions and full reusable acceptance continue; no external blocker or new
scope exclusion is asserted.

### Strict license-artifact evidence

The acceptance contract permits explicit artifact review for nonexecutable
obligations. The ledger now has a separate, strictly typed route restricted to
the exact SDK LICENSE artifact/file pair. It requires both explicit claims,
complete archived/current/reviewed-commit byte equality, immutable inventory and
manifest checks, committed independent review, preserved NOTICE/source-lock
hashes and fresh per-ID command evidence. It neither invents Rust tests nor
allows executable obligations, examples, documentation or distribution groups
to inherit closure. Failed Rust evidence still fails overall verification.

The independent technical review is retained in
`issue-11-license-review.json`; it approves only byte retention and provenance,
not legal compliance, release or relicensing. `verify-artifacts` records the
fixed profile and exact inputs separately from Rust evidence. Audit output
distinguishes **770 executable-tested** from **2 artifact-reviewed** records.
CI's quality checkout now retains history because the proof compares bytes at
the reviewed implementation commit; a shallow clone needs that commit available.

Fresh artifact verification, overlay/audit checks, **49 validator tests**, source
locks and immutable inventory validation pass. No Rust implementation changed in
this tooling slice; the latest full Rust result remains 1,537 passed / 0 failed / 30
ignored. Ledger: **772 verified / 8,119 unresolved / 251 excluded**. No additional
scope exclusions or approvals were created. Issue #11 remains incomplete.

### Failed-child findings in errors

Child runner partial-error outcomes now append the pinned progress heading and
latest trimmed nonblank/non-`[SYSTEM]` message, using the 1,600-character Unicode
middle excerpt. This covers ordinary/phased message items without introducing a
role filter absent from the source. Native retained result text and named-tool
recovery envelopes are unchanged; complete failure-envelope parity is not claimed.

Four new integration tests cover empty/tool-only input, latest/system/phased
selection, exact Unicode boundaries, actual findings → echo → provider failure,
and the enriched error in named/managed parent requests and summary views. Scoped
review found no blocking issue or missing assertion for the two original Go
tests. Broader scheduler cancellation can still select a separate cancellation
outcome and is not newly claimed by these failure tests.

Fresh Rust 1.88 workspace: **1,537 passed / 0 failed / 30 ignored**,155 targets,
no warnings; strict workspace Clippy and all source-bound oracle evidence pass.
Original pinned Go: **819 passed / 5 skipped**;27 validators pass. The preceding
payload checkpoint's9doctests, strict docs and full matrix/consumer checks remain
the latest such checks; no public API or feature configuration changed here.
All46 remote checks passed for `b1a1a981`.

Three exact records close the two original regressions and their complete
two-test source file: **770 verified / 8,121 unresolved / 251 excluded**. Full
issue acceptance remains incomplete. The next tooling slice is a strictly scoped
artifact-review route already permitted by the acceptance contract, initially
limited to retained SDK LICENSE bytes—not a waiver for executable obligations.

### Payload byte fidelity and cancellation visibility

Added sealed `adk::codec::{from_go_json, to_go_json_bytes}` boundaries for nine
item payload structs and the open signed `RunItemType`, excluding `RunItem` and
`Agent`. Shared decoding preserves ordered/folded duplicate fields, scalar null
updates, root null, reused image slots and Go string replacement. Raw input
retains lexical JSON, duplicate keys, number spelling and malformed UTF-8 through
`RawJson::EncodedBytes`; `bytes()` is lossless and text/value projections are
fallible. Generic Serde and native tool/approval admission remain strict.

The independently executed pinned oracle compares **all 1,267 cases**, including
all 16 malformed-UTF-8 raw cases, without unsupported-case exemptions. It compares
decoded fields, raw presence/bytes and marshaled bytes separately. Direct public
RawMessage construction additionally verifies the 10,000/10,001 encoding boundary
for both raw-bearing payloads. Review found a UTF-8 `RawValue` depth bypass;
representation-independent validation and Go/native regressions fixed it, with
scoped re-review approval. Nil/empty image vectors have the same omitted wire
form; Rust errors do not expose Go partial mutation or promise identical text.
See `scripts/payload-reference/README.md` and the root README.

Cancellation status now exposes live intent while resume persistence is blocked,
without treating that projection as a committed result. Both resume paths retain
checkpoint publication, no-dispatch fencing, failed-write poisoning, committed
waits/parent joins and durable cancellation acknowledgement. Restored steering
tests prove in-flight-before-queued delivery and clone isolation. Independent
security review found no blocking issue in these boundaries. Existing scheduler
schema/metadata and narrow behavior proofs were freshly rebound to their exact
source obligations; the six-scenario Go fixture changed only toolchain provenance
from Go 1.26.2 to 1.26.8, not observations.

Fresh Rust 1.88: **1,533 passed / 0 failed / 30 ignored**, **155 targets**, **9
doctests**, no warnings. Strict workspace Clippy/rustdoc, formatting, full facade
feature matrix, standalone consumer/purity, all bound independent oracles and
**27 audit validators** pass. Original pinned Go regressions: **817 passed / 5
skipped**. All 46 remote checks passed at preceding `7855a883`; this checkpoint's
latest remote checks are not yet claimed complete.

Exactly **88 additional records** are verified: 66 payload declarations/aliases,
20 narrow scheduler metadata/behavior records and two restored-task regressions.
Ledger: **767 verified / 8,124 unresolved / 251 excluded**. Issue #11 remains
incomplete. The next confirmed implementation gap is failed-child partial
findings missing from the error projection; full reusable acceptance remains
active work, not an external blocker. No CLI/evaluation/release scope was added.

### SDK progress, runtime accounting and registry selection

Added the separate `adk::progress::{ProgressTracker, RunProgress}` API without
changing native observability. It preserves SDK snapshot fields, signed/wrapping
accounting, per-model/cache totals, session resets, pending events versus the
20-event ring, text/inference rules, nested forwarding and structural span
pairing. Raw extraction/text helpers preserve ordered aliases/nulls, partial LSP
decoding, malformed UTF-8/surrogates, deep ignored JSON and distinct truncation
limits. See [ownership and usage](../observability.md#sdk-shaped-progress-state).

An optional nonfallible runtime interface records per-call usage separately from
policy hooks and generation/content observers. USD units are explicit; uncertain
nonzero subtotals remain available. Accepted, compaction, genuinely superseded
and retained failed-stream responses have distinct accounting boundaries. The
SDK tracker excludes Complete → Error usage while native budget accounting
retains it. Session completion status survives external span ownership.

Child progress follows the immediate parent across executor generations and
durable descendant-first recovery. Ancestor restoration replays neither usage
nor lifecycle starts. Generation-local roots and shared bindings ensure new
children follow refreshed/disabled observers while accepted old tasks and
explicitly unobserved parents retain their original scope. Review found and
regression tests fixed concurrent callback delivery returning before flush, and
stale child observers after refresh. Focused re-review found no remaining blocker.

Independent pinned SDK evidence now includes **16 tracker scenarios / 88
observations**, **236 helper cases**, **12 real runner accounting cases**, and
**79 registry configurations**. The latter compares actual constructor types,
definitions, option booleans, sorted enumeration and missing lookups. It maps Go
defaults explicitly to native selection; it does not relabel native empty/safe
defaults, browser resource construction, duplicate registration or tool execution
as equivalent. Harnesses and normalization limits are documented in
`scripts/progress-reference/README.md` and `scripts/registry-reference/README.md`.

Fresh Rust 1.88 workspace: **1,515 passed / 0 failed / 30 ignored**, across **153
targets**, with **7 doctests** and no warnings. Strict workspace Clippy/rustdoc,
the full facade feature matrix and platform-free standalone consumer/purity,
fresh bound oracle evidence, **27 audit validators** and immutable source locks
pass. Original pinned Go reference tests: **813 passed / 5 skipped**. A first run
with an overridden temporary directory failed one upstream test that assumes
paths beneath `/tmp`; rerunning with the host's default temporary-directory
semantics passed, without changing the pinned source.

Exactly **102 additional records** are closed: 56 registry selection/lookup
records, 19 helper API/alias records and 27 tracker operation/alias records.
Ledger: **679 verified / 8,212 unresolved / 251 excluded**. Whole tracker types,
complete runtime/registry parity and the rest of the reusable ledger are not
implicitly closed. Live provider/exporter credentials remain unverified.
**Issue #11 and draft PR #33 remain incomplete.**

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

### Per-agent tool stopping and explicit output

Per-agent stop-all, exact-name stop sets and first-tool/explicit-JSON output now
have native counterparts. Thirty-four pinned normal/streamed observations verify
case/whitespace-sensitive membership, continue/disabled selection, batch output
selection, raw JSON strings/null/false, model-schema bypass, null/output
validation and tripwire, steering, handoff and pause precedence. Raw-JSON result
origin preserves the SDK text-helper distinction and survives serialization.
Native recovery preserves the stop decision and output origin without repeating
effects; configuration changes are rejected. Ambiguous Go post-tool stopping
requires finalization reconciliation; terminal Go migration accepts verified
output and origin. Three existing SDK source regressions passed independently.

Fresh locked Linux Rust 1.88.0 workspace/all-features/all-targets:
**1,273 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, changed-file Rustfmt, facade matrix, platform-free consumer, 20 offline
scenarios and host-session example passed. Pinned source/fixture reproduction,
18 ledger validators, source locks, Python compilation, Go formatting and
whitespace checks passed. Three agent-field obligations are newly verified:
**251 verified / 8,640 unresolved / 251 excluded**. Full issue #11 remains open;
credentialed live checks remain unverified. No excluded CLI/evaluation adapters,
release work or production-default switch was added.

### Public owned handoff construction

`Handoff::new` and `AgentConfig::handoff_description` preserve constructor defaults,
shared target identity and description metadata. Forty-four independent pinned
cases cover naming, Unicode/invalid runs, empty fallback, custom/whitespace
metadata and explicit name/description overrides. Catalog observations verify
role/generic descriptions separately from overridden tool descriptions. The
native dispatch test and offline handoff example use the constructor; durable
recovery rejects changed description configuration.

The initial full run exposed a pre-existing nested-timeout fixture race: paused
Tokio time did not order a leaf's fixed delay against wall-clock deadlines, so a
child could complete where the assertion expected pending. The two-slot fixture
now gates leaf completion until the worker receives its pending result. No
assertions were weakened. Five focused repetitions passed, followed by a fresh
locked Linux Rust 1.88.0 workspace/all-features/all-targets run:
**1,276 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, changed-file Rustfmt, facade matrix, standalone consumer, 20 offline
scenarios and host-session example passed. Source oracles, 18 ledger validators,
source locks, Python compilation, Go formatting and whitespace checks passed.
Two constructor/description obligations are newly verified:
**253 verified / 8,638 unresolved / 251 excluded**. Arbitrary handoff filters,
callbacks and dynamic enable predicates remain separate missing capabilities;
the full issue remains incomplete. Credentialed live tests remain unverified.

### Awaited handoff callbacks and originating hook order

Added host-owned `HandoffCallback` with borrowed context, cancellation/deadline
bounding and durable identity checks. Corrected transfer observation to invoke the
originating agent's hook before the run hook and callback, not the target's hook
after activation. A committed handoff resumes without repeating the callback;
unsafe or changed callback identities are rejected. Schema diagnostics are
advisory and contain no argument values.

The independent pinned Go oracle reproduces 16 normal/streamed cases and the
original `TestHandoffOnHandoffReceivesInput` regression. Native tests compare
structured input, shared-state seeding, hook/callback ordering, first matching
call and skipped siblings; a cancellation test proves the callback future is
dropped before target dispatch. Native parsed JSON cannot retain raw whitespace
or distinguish absent arguments from null, and borrowed contexts intentionally
do not expose mutable Go agent pointers. The native always-present definition
schema also differs from optional SDK `InputType` warning semantics. These are
explicit unresolved compatibility obligations: no callback ledger IDs are closed.

Fresh Linux Rust 1.88.0 locked workspace/all-features/all-targets run:
**1,279 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, changed-file Rustfmt, facade matrix, platform-free consumer, 20 offline
feature scenarios and host-session example passed. All pinned fixture checks,
18 ledger validators, source locks, Python syntax, Go formatting and whitespace
checks passed. Ledger remains **253 verified / 8,638 unresolved / 251 excluded**.
Full issue #11 remains incomplete; credentialed live tests remain unverified.
Excluded CLI/evaluation adapters and release/default-switch work remain untouched.

### Dynamic handoff availability and stale-name behavior

Added owned `HandoffPredicate` / `Handoff::is_enabled`, defaulting to enabled.
Predicates filter request definitions and are rechecked during response
classification. First-enabled-sibling selection and callback suppression match
the independent 12-case normal/streamed pinned oracle. Exact result comparisons
preserve the SDK's advertised-then-disabled quirk: wrapper success text, but no
transfer. Initially hidden names instead return unknown-tool errors. Durable
checkpoints retain classification/exposure outcomes and reject unsafe or changed
predicate identities rather than reevaluating the pending call on recovery.

Fresh Linux Rust 1.88.0 locked workspace/all-features/all-targets:
**1,281 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, Rustfmt, facade matrix, standalone consumer, 20 offline examples and
host-session example passed. Pinned oracle reproduction and original
`TestHandoffIsEnabledGatesTool` execution, 18 ledger tests, source locks, Python
syntax, Go formatting and whitespace checks passed. The predicate-field
obligation is verified: **254 verified / 8,637 unresolved / 251 excluded**.
Whole-handoff parity, arbitrary input filters, exact callback argument provenance
and the rest of issue #11 remain open. Credentialed live tests are unverified;
no CLI/evaluation adapter, release or default-switch work was added.

### Custom handoff history filters

Added owned asynchronous `HandoffHistoryFilter` with explicit provenance and
approval anchors. It sees current-turn transfer and skipped-sibling outputs plus
the accumulated output-only snapshot, replaces continuation history before target
activation, and leaves emitted/new items unchanged. Eight independent pinned
normal/streamed cases compare preserve/messages/summary/empty transformations,
callback/filter order, target inputs and final history. Native tests cover invalid
pairs/provenance, cancellation/drop, retained/removed approval anchors, rejection
of forged/out-of-range markers before replacement, stable durable identity and
no repeated filter after a committed handoff. Conflicting custom/built-in filters
are rejected. Security invariants are explicit rather than weakened to accept
invalid native histories or fabricated approval authority.

Fresh locked Linux Rust 1.88.0 workspace/all-features/all-targets:
**1,284 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, all changed Rust files' formatting, facade matrix, standalone consumer,
20 offline feature scenarios and host-session example passed. Independent oracle
reproduction, original `TestHandoffInputFilterSeesCurrentTurnItems`, 18 ledger tests,
source locks, Python syntax, Go formatting and whitespace checks passed. The
history-filter field is verified: **255 verified / 8,636 unresolved / 251 excluded**.
Whole-SDK completion and exact raw callback/optional-schema compatibility remain
open; credentialed live tests remain unverified. No excluded CLI/evaluation,
release or production-default work was introduced.

The runnable `handoffs_subagents` example now composes a shared availability gate,
awaited callback and custom message-history filter, checks preserved provenance
and unchanged emitted handoff items, then disables the same gate for a second run.
Fresh `cargo run --locked -p adk --all-features --example features -- all` passed
all 20 scenarios. Strict direct-driver Clippy for the changed example, Rustfmt,
`verify-rust` with all pinned oracles, 18 ledger validators, source locks and
whitespace checks passed. Library code is unchanged from the 1,284-test checkpoint;
the full workspace suite was not redundantly rerun for this example-only change.

### Optional handoff input types and advisory parsing

Added `HandoffInputType` and effective `Handoff::tool_definition()` projection.
Source verification corrected an earlier assumption: the pinned SDK's default
`OutputSchema::Validate` parses JSON but does not enforce JSON Schema. Optional
custom parsers run only when a callback exists; their returned values are discarded,
and errors warn without suppressing callback or transfer. Native diagnostics expose
only the tool name. Strict ordinary-tool argument validation no longer makes
handoff parsing fatal. Durable identities bind optional schema presence/content;
custom parsers are rejected, matching the existing output-parser replay limit.

The independent 18-case normal/streamed oracle covers absent type, wrong shape,
null, parser success/transformation/failure, absent callback and missing/malformed
JSON. Native tests compare the 14 representable cases with strict argument
validation both on and off; four raw missing/malformed cases remain explicitly
unverified in the native Value API. No full InputType/raw-callback ledger claim
was added. The runnable example declares its callback schema explicitly.

Fresh Linux Rust 1.88.0 locked workspace/all-features/all-targets:
**1,286 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, changed-file Rustfmt, facade matrix, standalone consumer, 20 offline
feature scenarios and host-session example passed. Exact oracle reproduction,
18 ledger validators, source locks, Python syntax, Go formatting and whitespace
checks passed. Ledger remains **255 verified / 8,636 unresolved / 251 excluded**.
Full issue #11 and credentialed live verification remain incomplete; excluded
CLI/evaluation and release/default-switch work remain untouched.


### Exact native handoff argument text

`ToolCall` now retains optional UTF-8 argument text independently of its parsed
value. Handoff callbacks borrow the call and advisory parsers receive its exact
text, skipping absent input. All 16 callback and 18 input-type oracle cases now
run, including missing/malformed input that earlier checkpoints excluded. Model
and history boundaries reject contradictory raw/parsed projections. Core/codec,
provider request replay, native recovery, host history, approval identity and
capture-redaction regressions cover the new representation. Native approval
journals preserve a text sidecar across Go marker JSON compaction. Malformed raw
JSON is rejected by SDK document bridges rather than silently converted to null.

Valid provider argument spelling is retained where the adapter exposes it;
malformed provider normalization/rejection and Anthropic parsed outbound replay
are not claims of full provider-byte parity. Go non-UTF-8 bytes and mutable
context/target pointers also remain open. The broad Rust-file diff outside these
boundaries initializes the new optional field in existing ToolCall literals.

Fresh final Linux Rust 1.88.0 locked workspace/all-features/all-targets:
**1,294 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, all 77 modified Rust files' formatting, facade feature matrix, standalone
consumer, 20 offline scenarios and host-session example passed. Independent
pinned oracles, 18 ledger validators, source/license locks and whitespace checks
passed. The evidence command now includes core/provider contract tests. No broad
ledger claims were inferred: **255 verified / 8,636 unresolved / 251 excluded**.
Full issue acceptance and credentialed live verification remain incomplete; no
excluded CLI/evaluation or release/default-switch work was introduced.

### Handoff-history compaction and composition

Added a separate deterministic `HandoffHistoryPolicy`, its public helper and
runner application after custom/built-in filters, before target activation and
checkpoint. Builder feature selection, legacy compaction enablement and explicit
host-policy precedence now match the SDK's distinct handoff defaults. Native
pair/provenance/approval repair and carry-forward remain authoritative; audit
items are unchanged. The work also fixed local compaction dropping a native
handoff's matching call and switched tool/approval estimates to retained raw text.

The new independently reproduced pinned oracle covers 18 helper cases, 12
normal/streamed runner cases and 10 builder cases. Tests compare exact message
history/summary bytes, token estimates, default normalization, filter/carry order,
no-op/failure behavior, unchanged audit output and actual target requests.
Separate regressions cover approval-bearing durable recovery, all enabled policy
fields in the replay fingerprint, no repeated committed compaction, cancellation
while carry-forward is pending, raw estimates and native handoff pair repair.
The catalog-handoff offline example now actually triggers history compaction.

Fresh final Linux Rust 1.88.0 locked workspace/all-features/all-targets:
**1,300 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, changed-file formatting, facade matrix, standalone consumer, 20 offline
scenarios and host-session example passed. Pinned oracles and original source
regressions, 18 ledger validators, source/license locks, Python syntax, Go
formatting and whitespace checks passed. Three narrowly evidenced composition
records are closed: **258 verified / 8,633 unresolved / 251 excluded**.
Native opaque-content/pair protection and unsigned policy counters are documented;
no full arbitrary Go wire-history/helper claim was inferred. Full issue acceptance
and credentialed live verification remain open. Excluded CLI/evaluation and
release/default-switch work remain untouched.

### Measured handoff spans and raw tool trace inputs

The runtime now brackets a transfer with start/completion observations while
preserving the existing transition hook. The trace adapter retains its handoff
span through source/run hooks, callback, filtering, compaction/carry-forward and
the target checkpoint, then closes the source-agent envelope. Owner cleanup
closes interrupted spans before the agent/root; dropping a pending operation
precedes trace cleanup. Progress still counts a transfer once, and the schema-2
TraceWriter transition feed retains its original single record. Tool function
span inputs and metadata digests now use retained raw argument text rather than
a parsed/re-serialized projection.

Eight independent pinned normal/streamed cases compare actual span openness and
hook/callback/filter/carry ordering, including cancellation. Native tests also
cover dropping each pending stage, persistence failure while the span is open,
exactly-one span end before its parent/root, no fabricated compaction counts,
ordered progress events and exact raw input/digest transport. Native agent-parent
envelopes and explicit owner cleanup remain documented differences from the SDK;
no broad trace-graph parity or new ledger closure is claimed.

Fresh final Linux Rust 1.88.0 locked workspace/all-features/all-targets:
**1,306 passed / 0 failed / 30 ignored**. Strict direct-driver Clippy, strict
rustdoc, changed-file formatting, facade matrix, standalone consumer, all 20
offline scenarios and host-session example passed. Independent pinned oracles,
18 ledger validators, source/license locks, Python syntax, Go formatting and
whitespace checks passed. The normally ignored executable-Go runner/config codec
differential was explicitly run separately and passed (1 test). The 30 default
ignored entries are explicit Go/MCP/sandbox helpers, not 30 credentialed live tests.
Ledger remains **258 verified / 8,633 unresolved / 251 excluded**. Full issue
acceptance and credentialed provider/remote-collector verification remain open;
excluded CLI/evaluation and release/default-switch work remain untouched.

### Refreshed MCP interoperability and trace-store feature coverage

The pinned MCP interoperability harness was rerun against the unchanged SDK
revision and protocol module v1.4.1. All **28 cross-wire cases passed**, followed
by **112 MCP regressions passing** (the 28 explicit cross-wire tests remain
ignored in that second ordinary invocation). The evidence artifact records the
actual commands, source hashes, module verification and 28 observations. An
initial cache-only attempt lacked a transitive Go module; the subsequent run
resolved public build dependencies and passed. These are local Go/Rust peers,
not credentialed remote-service tests.

The Linux trace-store feature scenario now demonstrates the pinned example's
metadata and score-document round trips, ordered category appends, finish-time
update and run listing/filtering, alongside owned runner tracing. It additionally
checks candidate/start-time filters and private file modes. The category backend
still requires Linux `openat2`; macOS only exercises native observation storage.
No evaluation adapter or CLI behavior was added.

Fresh locked workspace/all-features/all-targets verification: **1,306 passed,
0 failed, 30 ignored**. Strict Clippy, strict rustdoc, changed-example formatting,
offline build and all 20 feature scenarios plus host-session passed. Pinned
reference/ledger checks and 18 ledger validator tests passed; source/license
locks and whitespace checks passed. Ledger remains **258 verified / 8,633
unresolved / 251 excluded**. This checkpoint does not claim full issue acceptance
or credentialed provider/remote-collector verification.

### Explicit provider image-analysis helper

Added borrowed `ImageAnalysisRequest` and `Provider::analyze_image`, exposed through
`adk::providers`. The helper uses Responses independently of ordinary Chat
selection and shares the existing client/session. Default model/detail, MIME
normalization, request budget/storage/cache settings, SSE text extraction and
validation match 11 independently executed pinned public-helper observations.
The existing image-loading tool is unchanged. Native tests also cover cancellation
before dispatch and while a stream is open, unsupported Anthropic protocol and
redacted HTTP failures. Transient retries remain host-owned; no credentialed
OAuth or live image-understanding parity is claimed. Corrected older trace-store
documentation: ordered provider snapshot normalization already exists with
bounded fixture coverage, distinct from native raw payloads.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,309 passed / 0
failed / 30 ignored**. Strict Clippy/rustdoc, formatting, facade feature matrix,
standalone consumer, all 20 offline feature scenarios and host-session passed.
The 11-case Go oracle and original pinned image-client regression reproduced;
18 ledger validators and source/license locks passed. Read-only specialist review
found no blocking issue (its sandbox could not independently execute compilers;
these execution results are from the main build environment). Ledger remains
**258 verified / 8,633 unresolved / 251 excluded**; no broad provider capability
was silently closed. Full issue acceptance remains open.

### Host-owned autonomous progress helpers

Added `adk::runtime::autoloop::{AutoTracker, CircuitBreakerResult,
DEFAULT_MAX_AUTO_LOOPS, build_smart_nudge}`. The host supplies newly emitted items
and decides whether to continue; no runner loop, worker CLI or global policy was
introduced. Tracking retains bounded tool/error history, argument-aware FNV-1a
signatures, SDK cycle/stall/error priorities, exact continuation strings and
rune-based truncation. Native saturating counters and multi-part text projection
are explicit limits; untrusted error text is not promoted to trusted instructions.

The pinned Go oracle covers **20 scenarios / 167 states**, comparing exact
counters, breaker booleans/reasons and nudge strings. It also executes all five
original SDK tracker/nudge regressions. Seven unit tests cover additional depth,
raw-spelling, Unicode, priority and saturation boundaries. The offline runtime
example now feeds an actual runner result through the public facade helper.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,317 passed / 0
failed / 30 ignored**; strict Clippy/rustdoc/formatting, feature matrix, standalone
consumer, all 20 scenarios and host-session passed. Pinned oracles, 18 ledger
validators (including missing-oracle rejection), source/license locks and
whitespace checks passed. Three narrow records—the default cap and result
boolean/reason fields—are closed; the overlay is now **261 verified / 8,630
unresolved / 251 excluded**, not a count of missing features. Full issue acceptance
and credentialed live verification remain open; excluded work is unchanged.

### Typed content-event formatter

Added `adk::codec::events::compact_content_event` for the existing SDK DTO. Ten
independent pinned cases compare exact optional-field ordering, error-only output,
first-LF extraction, preserved whitespace/CR, Unicode and uncapped long strings.
The helper neither logs nor redacts, and does not relabel native observations or
claim session-stream construction/JSONL parser parity.

Fresh workspace/all-features/all-targets: **1,318 passed / 0 failed / 30 ignored**;
strict Clippy/rustdoc, changed-file formatting, pinned oracles, 18 ledger validators,
source/license locks and whitespace checks passed. One typed formatter record is
closed: **262 verified / 8,629 unresolved / 251 excluded**. Full acceptance and
credentialed live-service verification remain open.

### Canonical content-event fields and exact float parsing

The independent Go oracle now reflects all **83 ContentEvent fields**, with
**284 canonical documents** and **six nonfinite rejection cases**. Exact typed
Rust decode/re-encode initially exposed eight mismatches in the two cost fields
near numeric formatting thresholds and for large exponents. Enabling
`serde_json/float_roundtrip` at the workspace dependency fixes the typed-float
parsing loss while retaining `arbitrary_precision` for raw numbers. All canonical
cases and nonfinite rejection checks now pass. No dependency version or lockfile
changed. The ledger now hash-binds the workspace manifest; a negative validator
rejects stale feature evidence.

**78 scalar field records** are closed using explicit source/field mappings and
independently generated canonical documents. Timestamp, slices, permissive Go
JSON decoding, complete type/stream behavior and invalid-UTF8 Go strings are not
silently included in those claims. The overlay is **340 verified / 8,551
unresolved / 251 excluded**.

Fresh Linux Rust 1.88.0 full workspace/all-features/all-targets: **1,320 passed /
0 failed / 30 ignored**. Strict Clippy/rustdoc, formatting, facade matrix,
standalone consumer, all 20 offline scenarios and host-session passed. Pinned
oracles, **19 ledger validators**, source/license locks and whitespace checks
passed. Scoped independent review found no blocker and rechecked hashes/ledger;
its compiler/oracle execution was sandbox-blocked, so executable results remain
from the main build environment. Full issue acceptance and live-service checks
remain open.

### SDK JSONL event helpers

Added `adk::codec::events::{parse_content_event_line, compact_content_event_line}`
for UTF-8 Rust line inputs. The independent pinned Go oracle executes **1,429
line cases** covering all 83 fields, malformed records/types, duplicate/null and
case-fold behavior, repeated slices, Unicode escapes, permissive timestamp
parsing, marshal failures and the 10,000-container boundary. Successful records
compare exact Go JSON bytes; non-event records compare exact first-line fallback.
No line/output size cap, logger, redaction or session-stream construction was added.
The existing strict timestamp DTO decoder remains strict; only the SDK JSONL path
accepts Go's permissive timestamp forms. Arbitrary invalid-UTF8 Go string input is
not a Rust `&str` capability.

Differential verification caught two subtle parser differences: unknown values
beyond Go's nesting limit and integer `-0` (which Serde classifies as a float).
Both are corrected. The first full negative-zero run caught one omitted multiline
field; the final focused and full runs pass after correcting that field too.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,321 passed / 0 failed /
30 ignored**. Strict direct-driver Clippy, strict rustdoc, changed-file formatting,
facade feature matrix, standalone consumer, all 20 offline scenarios and the
host-session example passed. Codec doctest check passed with no doctests present.
Pinned reference reproduction, **19 ledger validators**, source/license locks and
whitespace checks passed. Two narrowly scoped UTF-8 helper claims are closed:
**342 verified / 8,549 unresolved / 251 excluded**. These counts are inventory
obligations, not a count of missing features. Full issue acceptance and credentialed
live-service verification remain open; CLI/evaluation exclusions are unchanged.

### Owned typed subagent progress projection

Added `adk::codec::events::subagent_event` and `dto::SubagentStreamEvent`. Selection
requires exactly `subagent_status` and a nonempty task ID; the agent name falls
back to the subagent type only when empty. All **21 fields** are projected,
including independent dependency/waiting lists, signed counters and exact costs.
Empty lists project to Go nil. No runtime event emission, native-schema conversion,
line writer or whole stream-type decoder is claimed.

The pinned oracle compares **294 projections**, varying every source content-event
field, selector/fallback cases, signed limits, escaped strings, slice values and
float boundaries. Both Go and Rust checks mutate source lists after projection
and verify the output remains unchanged. The checker also executes the original
SDK subscription and public line-writer subagent regressions. Nonfinite costs
remain representable in memory but fail JSON serialization.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,322 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, changed-file formatting, full facade matrix,
platform-free consumer, all 20 offline scenarios and host-session example passed.
Pinned oracles, **19 ledger validators**, source/license locks and whitespace
checks passed. Twenty-three specific records (21 fields and the internal/public
projection helper) are closed: **365 verified / 8,526 unresolved / 251 excluded**.
These are inventory obligations, not counts of missing features. Full acceptance
and credentialed live-service verification remain open; exclusions are unchanged.

### Owned SDK JSONL host writer

Added `adk::codec::line_writer::{LineWriter, LineEvent}` with synchronous ordered
`FnMut` dispatch and `std::io::Write`. Partial bytes, CR removal, empty-write
behavior and subagent/child/log classification match **22 independently executed
pinned SDK scenarios**. All three original SDK line-writer regressions also pass.
Boundary cases cover the 16-MiB scanner threshold, overflow after earlier tokens,
fragmented input and complete oversized JSON error output. The oracle compares
small text exactly and large text using full-output SHA-256 plus byte lengths,
not truncated prefixes. Its raw-byte length records make the invalid-UTF8 boundary
visible: Rust text matches Go JSON string rendering, not arbitrary raw Go bytes.

The native lifecycle is explicit: flush is an empty write, drop does not flush,
shared ownership uses a host mutex, and a no-op closure replaces a nil sink. Tests
exercise these choices and the API includes a runnable doctest. The SDK's unusual
overflow fallback (which may repeat the first line as a log) is retained and
documented rather than silently changed. Redaction remains host-owned. The only
dependency change is a dev-only edge to already locked `sha2` 0.10.9 for fixture
hashes; no dependency version changed and standalone consumers do not acquire it
from this crate's runtime dependencies.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,324 passed / 0 failed /
30 ignored**, plus the new doctest. Strict Clippy/rustdoc, changed-file formatting,
facade matrix, platform-free consumer, all 20 offline scenarios and host-session
passed. Pinned oracles, **19 ledger validators** (now requiring the line-writer
oracle marker), source/license locks and whitespace checks passed. Constructor
and write claims bring the overlay to **367 verified / 8,524 unresolved / 251
excluded**. SDK session event-stream construction, full runtime event mapping,
remaining ledger obligations and credentialed live services are still open.

### Owned content-stream metadata and subscriptions

Added `adk::content_stream::{ContentStream, ContentSubscription}` under the existing
`observability` feature. Parent/child streams share the owned writer and bus while
copying metadata. Emission fills only unset context, recognizes zero instants even
in nonzero offsets, recalls subagent names, broadcasts before best-effort JSONL
writes and reports subscriber loss through source-compatible drop markers.
Subscriptions close explicitly or on drop; queued records survive explicit close,
and the last stream owner releases the writer. Independent Rust event ownership
and unsigned capacity (`0` selects the SDK's 64-slot default) are documented.

The pinned Go oracle has **166 recorded event views**, including exact wire
serialization and subscription observations, and executes six original internal
stream tests plus the public session-stream constructor regression. Four Rust
tests compare those views and additionally verify owned lifetimes, independent
subscriber counters and **200 concurrent complete records with per-producer
ordering**. The native clock uses UTC; the oracle explicitly selects and verifies
the UTC Go profile. Non-UTC automatic clock behavior is not claimed. Bus and writer
serialization are separate, not a global atomic cross-sink sequence.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,328 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, changed-file formatting, facade feature matrix,
platform-free consumer, 20 offline scenarios, host-session and both facade/codec
doctests passed. Pinned oracles, **19 ledger validators**, source/license locks and
whitespace checks passed. Only seven constructor/setter/subscription records are
closed: **374 verified / 8,517 unresolved / 251 excluded**. Whole `Emit`, child
logger inheritance, logger attachment, typed convenience emission and complete
`NewSessionEventStream` remain open, despite the tested no-logger core path.

A source review corrected misleading SDK comments: `AgentLogger` uses Go's global
`log.Printf`, whose default is stderr, not a hard-wired stdout destination. Future
logger composition must retain host destination control. Full issue acceptance
and credentialed live-service verification remain open; no exclusions or release
defaults changed.

### Owned content-event logger and stream inheritance

Added `ContentLogger` for exact SDK `AgentLogger.Event` message bodies and
`ContentStream::set_logger` with child snapshot inheritance. Event annotation
precedes logging; logging precedes subscriber delivery and JSONL serialization.
The pinned logger oracle covers **42 event cases**, all event branches, partial
UTF-8 byte previews, inherited names and logging despite serialization failure.
A complete **10,137,174-byte** log over all **1,112,064 Unicode scalar values** is
compared by length and SHA-256. Generated Go 1.26.8 Unicode 15.0.0 printable ranges
retain BSD attribution/license and independently checked source hashes.

Three Rust regressions cover that oracle, attachment/detachment isolation,
writer-failure ordering, explicit flush and last-owner resource release. The
embedding doctest composes owned log and JSONL writers without global state.
This does not implement arbitrary Go global logger flags/prefixes/timestamps,
turn/tool logger methods or Go printf arguments. Logs are unredacted and require
a trusted destination; native event redaction is a separate pipeline.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,331 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, facade feature matrix, standalone consumer,
20 offline scenarios, host-session, two facade doctests and one codec doctest
passed. All pinned oracles, **19 ledger validators**, source/license locks and
whitespace checks passed. Three narrow logger-setter/child-constructor claims
bring the overlay to **377 verified / 8,514 unresolved / 251 excluded**. No
immutable inventory, exclusion, CLI/evaluation behavior or release default changed.
Full logger/session construction, other ledger obligations and credentialed live
services remain open; offline observations do not establish live-service parity.

### Owned turn/tool logger bodies

Added `AgentLogger`, signed `LogLevel`, and `ModelResponseSummary` under the
existing observability feature, sharing the tested content logger sink. The
pinned Go oracle now also records complete raw-byte transcripts at levels -2,
0, 1 and 2. Coverage includes normal/debug bodies, all item discriminants,
nullable message/compaction, byte-split UTF-8, exact preview/chunk boundaries,
interior raw-JSON whitespace, signed extremes and terminal newline handling.
Raw hex fixtures retain invalid UTF-8 produced by unquoted byte previews.

Three native regressions compare the oracle, prove disabled debug avoids
rendering, exercise shared sink ownership/explicit flush, and verify multi-line
calls continue after errors while retaining the first error across intervening
successful writes. Focused independent review found no blocking correctness
issues; both suggested boundary/recovery regressions were added. Parent-side
execution then passed despite the review worker's toolchain/cache restrictions.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,334 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, feature matrix, standalone consumer, 20 offline
scenarios, host-session, three facade doctests and one codec doctest passed.
Pinned oracles, **19 ledger validators**, source/license locks and whitespace
checks passed. Only seven log-level/type/predicate records are newly closed:
**384 verified / 8,507 unresolved / 251 excluded**.

The native mapping is documented, but whole logger parity is not claimed: Go
process-global output/prefix/timestamp flags, arbitrary printf arguments and
invalid/raw-whitespace Go RawMessage inputs remain outside this API. No immutable
inventory, exclusion or release default changed. Full issue acceptance and
credentialed live-service verification remain open.

### Typed text, thinking, compaction and hook emission

Five `ContentStream` helpers now construct the SDK event fields and use the
existing ordered emission path. The pinned stream oracle adds 21 records for
empty/multiline/escaped Unicode, 68,000-byte untruncated text, child metadata,
attempt identifiers and signed 64-to-32-bit compaction narrowing. The Rust
regression compares complete JSONL records and independently delivered records.

Fresh Linux Rust 1.88.0 workspace/all-features/all-targets: **1,335 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, feature matrix, standalone consumer, 20 offline
scenarios, host-session, three facade doctests and one codec doctest passed.
Pinned oracles, 19 ledger validators and source/license locks passed. Five named
emitter records are closed: **389 verified / 8,502 unresolved / 251 excluded**.
Session construction and remaining typed helpers are not claimed by this slice.

Remote CI on the preceding `55fb4c6` commit is not fully green: 44 jobs passed,
but two dependency jobs failed on **RUSTSEC-2024-0449**, unmaintained
`unsafe-libyaml 0.2.11` via `serde_yaml_ng 0.10.0`. The audit reports bans,
licenses and sources passing; advisories fail. No exception was added. Replacing
the YAML implementation requires preserving mode/role compatibility and the
Rust 1.88 MSRV; the newly released `serde-saphyr 2.0.0` declares Rust 1.89 and
is not a drop-in supported-target solution. This readiness blocker and the
remaining issue acceptance/live-service work remain open.

### YAML dependency-audit remediation

Replaced `serde_yaml_ng 0.10.0` with exact `serde_yaml_bw 2.5.8`, preserving
mode/role typed deserialization and using string-key removal for the changed
`Value` API. Both workspace and standalone-consumer locks now resolve the
replacement dependency graph. No advisory exceptions or policy weakening were
introduced. `cargo-deny 0.20.2 --locked check` reports **advisories ok, bans ok,
licenses ok, sources ok**. Research records source revisions, licenses, parser
budgets, the remaining unsafe-library risk, and why `serde-saphyr 2.0.0` is not
compatible with our Rust 1.88 minimum.

The focused builder/file-configuration suite passes **72 tests**, including the
pinned Go oracle and a new regression for aliases, multiline Unicode, envelopes,
duplicate keys, undefined aliases, malformed input and unknown fields. Full
workspace/all-features/all-targets passes **1,336 tests / 0 failed / 30 ignored**;
strict Clippy/rustdoc, source/reference checks and 19 ledger validators pass.
The initial embedding check correctly rejected the stale standalone lockfile;
it was updated, not bypassed. The rerun passed the locked feature matrix,
standalone consumer, 20 offline scenarios, host-session and facade/codec doctests.
No extra capability ledger records are closed by this dependency change.

### Session-stream construction and typed boundaries

Added owned `SessionStreamOptions`, `SystemInit` and `SessionEnd` records plus
`ContentStream::session`, `emit_system_init` and `emit_session_end`. Initialization
reaches the writer before construction returns; later subscriptions do not replay
it. Metadata, logger ownership, complete terminal usage/cost fields, list ordering,
empty values and signed turn-count narrowing match four independently executed
pinned SDK scenarios. A further native regression covers failing log/JSONL sinks,
NaN and both infinities, continued subscription delivery, and explicit flush.
Independent source review found no blocking issues; its coverage suggestions were
added before final verification.

Only the two typed emitter ledger records are newly closed: **391 verified /
8,500 unresolved / 251 excluded**. Host-owned logger composition is documented;
Go process-global logger allocation, prefix/timestamp flags and whole constructor
parity are not claimed. Neither session-end emission nor dropping one owner
implicitly flushes or closes another owner's resources.

After a worker restart, both immutable source checkouts and the missing SDK tag
were restored and source locks reverified. Fresh Linux Rust 1.88.0 full workspace,
all-features/all-targets: **1,338 passed / 0 failed / 30 ignored**. Strict
Clippy/rustdoc, feature matrix, independent standalone consumer, 20 offline
scenarios, host-session, four facade doctests, one codec doctest, all reference
oracles and **19 ledger validators** passed. Changed-file formatting and whitespace
checks passed. No source inventories or excluded CLI/evaluation scope changed.

The workspace and standalone locks now use **rustls 0.23.46**, addressing
RUSTSEC-2026-0346 through 0349. Both manifests pass advisories, bans, licenses and
sources under cargo-deny 0.20.2 after fetching RustSec revision `caee11f`. The
standalone fixture's existing GPL license and local ADK version are now explicit;
no audit policy was relaxed. An unused AGPL allowance warning for the standalone
consumer is informational, not an audit failure.

Other typed emitters, complete SDK ledger closure and full issue acceptance remain
open. Offline checks do not establish live provider/service behavior; unavailable
credentialed tests remain unverified, and ignored tests are not counted as passes.

### Typed tool-start/tool-end events

Added owned `ToolStart` and `ToolEnd` records and the public 32,768-byte default
limit. The two emitters cap only raw input/output, including error output, reserve
three bytes for `...` and back up to a UTF-8 boundary. Summaries remain uncapped;
whitespace is preserved. Generic emission and trace-store limits did not change.

The independent pinned oracle compares **27 complete records** via both JSONL and
subscriptions, using complete SHA-256 hashes and byte lengths to keep large
fixtures compact. Cases include exact/overflow limits, all four four-byte-character
alignments at the reserved cut, 80,000-byte Unicode data, empty fields, inherited
metadata and signed duration extremes. Original SDK `TestTruncateBytes` regressions
also execute. A separate native end-to-end test feeds both helpers into `LineWriter`
and verifies ordered, typed child-tool progress with capped input/error output.

Fresh Rust 1.88.0 full workspace/all-features/all-targets: **1,340 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, facade feature matrix, standalone consumer,
20 offline scenarios, host-session, four facade/one codec doctests, all pinned
oracles, **19 ledger validators**, source locks, formatting and whitespace checks
pass. No dependency changes were made after the preceding successful audits.
Four precise emitter/constant claims bring the overlay to **395 verified / 8,496
unresolved / 251 excluded**. Generic `TruncateBytes`, invalid Go string bytes,
remaining subagent/LLM helpers, automatic runtime wiring and full issue/live-service
acceptance are not implied by these claims.

### Typed subagent lifecycle/status events

Owned started/completed/task-status helpers now match 73 complete pinned Go
JSONL/subscriber records. The cases cover the 4,096-scalar trim/newline preview,
all Go whitespace, BOM/zero-width-space retention, signed counts and stored
duration extremes, cache usage, inherited identity, exact terminal spellings,
empty dependencies, and result-versus-error semantics. Native `LineWriter`
composition verifies ordered owned progress and subscriber delivery even when a
nonfinite cost cannot serialize. These APIs do not mutate scheduler state or
implicitly convert native task snapshots.

Fresh Rust 1.88.0 full workspace/all-features/all-targets: **1,343 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, facade matrix, independent standalone
consumer, 20 offline feature scenarios, host-session, four facade/one codec
doctests, pinned reference oracles, 19 ledger validators, immutable source locks,
formatting and whitespace checks passed. Four precise emitter claims bring the
overlay to **399 verified / 8,492 unresolved / 251 excluded**. Remaining ledger
and issue acceptance are open; credentialed provider behavior remains unverified.
Prior pushed head `8223bc99eb1dd98be5e4adaff9c4766eb789d6e6` had all 46 reported
GitHub checks successful; that CI result is not attributed to this newer slice.

### Typed LLM-attempt event normalization

`LlmAttempt` and full `ContentEvent` inputs now use the same emitter. A pinned Go
oracle compares 25 complete subscriber/JSONL records: empty inputs, signed attempt
limits, all 16 independent default/conflict combinations, no reverse defaults,
fixed-offset nanosecond timestamps, context inheritance/overrides, full usage and
retry/fallback data, and uncapped 40,000-byte unrelated payloads. Four additional
native tests validate the mapping. Rust uses explicit `i32` attempts; invalid Go
vararg types/counts and implicit platform-int narrowing are not replicated.

Fresh Rust 1.88.0 workspace/all-features/all-targets: **1,348 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, facade feature matrix, standalone consumer,
20 offline examples, host-session, five doctests, pinned oracles, 19 ledger
validators, source locks, formatting and whitespace checks pass. Overlay:
**400 verified / 8,491 unresolved / 251 excluded**. This closes only the typed
emission contract, not automatic provider/runner event wiring, credentialed
provider parity, remaining reusable ledger entries or full issue acceptance.

### Independent trace-finalization persistence

Fixed a source-confirmed ordering gap: a failed health-file write previously
prevented updating finished-at metadata. `finalize_run` now attempts both and
returns the first health serialization/write error, otherwise the metadata error.
Append failures retain existing health accounting. Empty IDs skip append/health,
not the metadata attempt. No shared store is closed implicitly.

A new independently executed Go oracle covers all eight combinations of append,
health and metadata failures for both populated and empty run IDs. Native tests
compare all 16 ordered call sequences, normalized terminal records, health and
finished state, and separately validate timestamps/error precedence.

Fresh supported Rust 1.88.0 full workspace: **1,349 passed / 0 failed / 30 ignored**;
strict Clippy/rustdoc, facade matrix/standalone consumer, 20 offline scenarios,
host-session, five doctests, pinned oracles, 19 ledger validators, immutable source
locks, formatting and whitespace checks passed. An exploratory specialist check
reported Clippy warnings in unchanged code; the supported Rust 1.88 strict gate
above passed without warnings. Ledger: **401 verified / 8,490
unresolved / 251 excluded**. This is finalization-method closure, not whole issue
acceptance; live-service and remaining reusable-capability obligations stay open.

### Host-owned runner reasoning correlation

Added `ContentGenerationObserver` and ordered observer composition, backed by
runtime callbacks for reasoning, model return and correlated acceptance. Blocking
native stream consumption is explicitly selected with `blocking_reasoning`;
default completion and ordinary streamed delivery remain unchanged. Surfaced
reasoning commits an attempt against retry/immediate-input replacement; hidden
text/tool fragments are not published or retained on retry. The option is included
in durable fingerprints only when true, preserving default replay identity.

The adapter correlates real attempt/delta/final-thinking IDs, flushes coalesced
residue before terminal events, and never turns a failed retained response into
accepted thinking. It preserves trace composition without implying trace capture
redacts this separately trusted content sink. Completion usage uses SDK
`input_tokens`/`output_tokens`; failed events carry bounded error output without
invented completed usage. Stores, exporters and writer flushing remain host-owned.

Independent pinned Go evidence now includes six fake-clock coalescer cases and
four complete actual-run attempt records (success/failure), plus the original
blocking/streamed reasoning regressions. Native coverage adds ten runtime tests,
eight real-run facade tests and three private adapter tests. A runnable embedding
doctest shows explicit attachment. Source review of the runtime found no concrete
blockers; that read-only reviewer could not execute its own compiler checks.

Fresh Rust 1.88.0 workspace/all-features/all-targets: **1,370 passed / 0 failed /
30 ignored**. Strict Clippy/rustdoc, feature matrix, independent standalone consumer,
20 offline scenarios, host-session, **six doctests**, all pinned oracles, 19 ledger
validators, source locks and changed-file formatting/whitespace checks pass.
Original Go reference refresh: **786 passed / 5 skipped** across 35 packages;
skips are helper/platform/environment cases, not live-provider validation.

The first combined gate stopped because the old reference-refresh script omitted
ten previously added internal regression references, not because a Rust or Go
behavioral test failed. The driver now retains the bounded historical regressions
and includes current claimed internal test identifiers. Fresh reference execution,
Rust evidence regeneration, all ledger checks and the remaining matrix passed.

Six precise coalescer/regression claims bring the overlay to **407 verified / 8,484
unresolved / 251 excluded**. No whole-runner closure is claimed: native numbering
under interruption, provider-specific completion-only live reasoning and automatic
child configuration selection remain explicit limits. Full issue acceptance and
credentialed live-provider verification remain open; no excluded CLI/evaluation,
release, merge or default-switch work was performed.

### Narrow public-session metadata and child projection closure

A source/evidence audit identified seven already-implemented session metadata
members with complete existing pinned constructor/boundary proof. Added individual
claims for session, phase, optional initialization, model, permission-mode report,
directory report and signed turn narrowing. These are not permission enforcement,
working-directory mutation, model selection, whole-constructor or global-logger
claims.

For the public child-tool projection, added twelve direct pinned Go helper cases
instead of relying on older unbound baseline fixtures. Complete output comparisons
cover both rejection guards, exact kind matching, empty/default fields, whitespace
parent IDs, exclusive start/end payloads, signed duration extremes and large
uncapped Unicode payloads. Owned native results survive input mutation.

Fresh full workspace **1,371 passed / 0 failed / 30 ignored**; strict Clippy/rustdoc,
feature matrix, independent standalone consumer, 20 offline scenarios, host-session,
six doctests, all pinned oracles, 19 ledger validators, source locks and formatting/
whitespace checks pass. Eight precise claims bring the overlay to **415 verified /
8,476 unresolved / 251 excluded**. Full issue acceptance and live-provider proof
remain open; no excluded scope or production/default-switch work was added.

### Standalone specialist tool-list helpers

Added runtime-independent `adk::specialists` normalization, access-adapter filtering,
and default/custom signal stripping. SDK role normalization maps to the same native
normalizer. The two-tier selection enum is deliberately separate from execution
permissions. The pinned oracle covers 24 access strings, Unicode whitespace/case,
role defaults, adapter ordering/renaming, default and custom exact signal maps;
native tests additionally preserve ordered duplicate Arc identity and owned maps.

Fresh full workspace: **1,378 passed / 0 failed / 30 ignored**. Minimal facade:
**six specialist tests passed** without default features. Strict Clippy/rustdoc,
feature matrix, standalone consumer, 20 offline scenarios, host-session, six
doctests, pinned oracles, 19 ledger validators, source locks and formatting pass.
Nine narrowly scoped claims bring the overlay to **424 verified / 8,467 unresolved /
251 excluded**. Empty Rust vectors do not model Go nil-slice identity. No runtime
execution-authorization, whole specialist-builder, live-provider, or full-issue
closure is claimed; excluded CLI/evaluation/release work remains untouched.

### Specialist mode/role reasoning composition correction

Source review found that merging mode and role labels directly into base settings
retained a mode `high` budget after an explicit role `none`. Specialist construction
now resolves routing separately (`ModelRouting::settings_for_role`) before base
merge. Four additional pinned public-builder cases prove base none/high/none has
no budget, base low/high/none retains 2048, and invalid/blank roles inherit 8192.
A 16-combination native regression preserves arbitrary settings-map precedence;
explicit native mode budgets are not mistaken for label-derived budgets.

Fresh full workspace **1,379 passed / 0 failed / 30 ignored**, strict Clippy/rustdoc,
feature matrix, standalone consumer, 20 offline scenarios, host-session, six
doctests, all pinned oracles, 19 ledger validators and source locks pass. Focused
catalog tests: **28 passed**. No additional whole-routing claim is made; ledger
remains **424 verified / 8,467 unresolved / 251 excluded**. Active-parent role
routing and native explicit-empty fallback semantics remain unchanged. Full issue
acceptance and credentialed live-provider verification remain open.

### Host-owned automatic child session lifetime

Added `Builder::build_session`: ordinary catalog/child assembly returns a separate
exclusive `SessionState` owning its scheduler and child tool/MCP/project-state
resources. Borrowed-empty-session mutation is still rejected. Turn bundles borrow
the handle and own independent resources; closing them does not revoke the
session's child tools. Explicit session close cancels, attempts every resource
cleanup and joins children; drop uses existing revocation/abort RAII semantics.
An existing explicitly transferred owner retains earlier resource sets. Configuration
and session context are construction-time snapshots, not hot reconfiguration.

Six new lifecycle regressions prove cross-turn survival using the original real
owner-bound probe after the first turn closes/cancels, failed-turn isolation,
late factory rollback, borrowed factory rejection, joined close and owner drop.
Two private regressions retain actual role-tool handles and prove revocation on
close/drop with a fresh uncancelled context. Three integration regressions verify
real MCP process retention and joined reap, plus injected filesystem-backed
project-store owner retention/release. The project-store tests use ownership counts,
not automatic filesystem-host construction or child project-tool execution.
The offline feature example now runs two separately closed turn bundles against
one automatically assembled session and preserves scheduler/task history.

Fresh full workspace: **1,390 passed / 0 failed / 30 ignored**. Strict Clippy/rustdoc,
feature matrix, standalone consumer, 20 offline scenarios, host-session, six
doctests, pinned Go oracles, 19 ledger validators and source locks pass. Focused
catalog/lifecycle tests: **34 passed** in both builder-only and builder/MCP/project-
state configurations; private owner tests pass in both. Independent source review
found no concrete blockers; its own compiler launcher was unavailable, so compiler
claims here come from the parent/direct-driver verification.

No broad SDK scheduler/reconfiguration closure is claimed. Ledger remains **424
verified / 8,467 unresolved / 251 excluded**. Full acceptance and credentialed live
provider checks remain open; no CLI/evaluation/release/default-switch work.

### Feature-selection field audit and review queue

Reviewed twelve selection fields in the pinned `runtime/features.go`: three
managed-subagent gates, six MCP selectors and three project-state tool gates.
Their API and configuration facets have separate claims backed by existing
independent Go matrices and native registration/host-policy tests. These 24
closures do not establish whole scheduler, MCP protocol or project-store parity.

Added a deterministic, CI-checked review queue covering all 88 verification
groups. It preserves all IDs and dispositions and only identifies related
declarations for review, never automatic closure. Eight negative/determinism
tests pass. The initial evidence run rejected incorrect module-qualified test
names in the pending claims; corrected names passed the fresh evidence run.

Fresh workspace: **1,390 passed / 0 failed / 30 ignored**; strict Rust 1.88
Clippy/rustdoc, feature matrix, standalone consumer, 20 offline scenarios,
host-session example, six doctests, pinned oracles, 19 ledger validators and
source locks pass. Overlay: **448 verified / 8,443 unresolved / 251 excluded**.
Unresolved inventory IDs are not a count of missing capabilities. Full issue
acceptance and credentialed live checks remain open.

### Signed SDK usage helpers

Added `codec::dto::Usage::{add,total_tokens}` with explicit wrapping signed
arithmetic, independent of native unsigned runtime accounting. A new pinned-Go
generator produces 64 operand pairs covering zero, negative, cache-only and
signed extrema; Rust checks every serialized result and total. The borrowed
operand is unchanged. Request counters match 64-bit Go `int`, not 32-bit Go.
Seven member claims are closed; no whole cost-estimator/runtime claim is made.

Fresh workspace: **1,391 passed / 0 failed / 30 ignored**; strict Clippy/rustdoc,
feature matrix, platform-free standalone consumer, 20 offline scenarios,
host-session, six doctests, pinned oracles, 19 ledger validators and eight audit
tests pass. Original Go regression refresh: **787 pass / 5 skip**, 35 packages.
Overlay: **455 verified / 8,436 unresolved / 251 excluded**. Full issue acceptance
and credentialed live-provider checks remain open.

### Child handoff-history configuration

The pinned `internal/agent/subagent_registry.go` copies the parent's nested-run
handoff-history policy into the child snapshot and child run config. Native
automatic child construction now preserves that already-resolved policy instead
of silently resetting it to disabled defaults. A four-variant regression failed
on the original implementation and passes after the one-field correction in
both minimal-builder and integrated configurations. The existing ten-case
pinned builder-selection test also passes. This is construction-time inheritance,
not session hot reconfiguration or a whole child-runner parity claim.

Fresh workspace **1,392 passed / 0 failed / 30 ignored**; strict Clippy/rustdoc,
matrix/platform-free consumer, 20 offline scenarios, host-session, six doctests,
pinned oracles, 19 ledger validators, eight queue tests and source locks pass.
Ledger remains **455 verified / 8,436 unresolved / 251 excluded**. At the preceding
`f542bc7` checkpoint, CI reported 42 successful and four pending checks, no failures;
that is not a completed-CI claim for this change. Full acceptance remains open.

### Unknown provider costs in automatic generation traces

`CostEstimator::estimate_cost` now preserves absent prices; numeric-only host
estimators default to known values. `BaselineCosts` forwards the route's optional
estimate instead of turning an unknown zero into a known generation cost.
Accounting remains numeric with zero for unknown prices. Retained completions
from failed streams preserve the exact per-response estimate instead of deriving
it by subtracting cumulative floating-point totals. Host estimators run once per
response, not once for accounting and again for tracing.

Three new runtime regressions cover twelve unknown/zero/priced × run/stream ×
success/failure combinations, default numeric estimators, and a 0.25 failed-response
cost after a 1e16 prior total. Extended provider integration checks named fallback
routes and generation observers in both modes. The initial unknown-Anthropic test
assumption was corrected: that provider has a known default price; unknown OpenAI
pricing is the actual absent-price case. Existing provider pricing was unchanged.

Fresh full workspace: **1,395 passed / 0 failed / 30 ignored**. Strict Clippy/rustdoc,
matrix/platform-free consumer, 20 offline scenarios, host-session, six doctests,
pinned oracles, 19 ledger validators, eight audit tests and source locks pass.
No additional whole cost-aggregation or live-provider claim is made. Ledger remains
**455 verified / 8,436 unresolved / 251 excluded**; full issue acceptance is open.

### Stateless prompt-constructor facets

Separately reviewed the configuration records for `BuildWorkspaceContext` and
`BuildAutoTurnCapPrompt`: both source bodies only format text, with no extra
registration or configuration mutation. Existing exact helper/composition tests
cover these same narrow contracts; their prior input/target limits still apply.
Fresh claim-bound oracle/native evidence and all 27 ledger/audit tests pass.
Overlay: **457 verified / 8,434 unresolved / 251 excluded**. The queue now has no
same-declaration candidates; this does not close any remaining verification group
or establish full SDK parity. No Rust behavior changed in this audit checkpoint.

### Explicit public budget-context helpers

Added `codec::userinput::{build_run_budget_context,build_subagent_budget_context}`.
Seven independent pinned public-alias inputs per helper compare complete prompt
text, signed extrema and nonpositive defaults (100 top-level, 50 child). Original
SDK helper regressions execute alongside the oracle; source and harness hashes
retain provenance. Eight API/alias/configuration/test facets are individually
closed. Rust functions replace Go alias indirection, not mutable global callbacks.

These helpers do not set/enforce limits or inject prompts. Source review confirms
that `subagent_run.go` separately appends child guidance and forces final summary;
that native composition work remains open. Do not infer warning behavior from
legacy prose: the source runner regression explicitly forbids turn-budget warning
injection. No automatic warning behavior was added.

Fresh workspace **1,396 passed / 0 failed / 30 ignored**, strict Clippy/rustdoc,
feature matrix, standalone consumer, 20 offline scenarios, host-session, six
doctests, source locks, pinned oracles and all 27 ledger/audit tests pass. Original
Go regression refresh: **789 pass / 5 skip**, 35 packages. Ledger: **465 verified /
8,426 unresolved / 251 excluded**. Full acceptance and credentialed live checks
remain open; this checkpoint does not claim automatic child composition parity.

### Child final-summary selection

Matched the unconditional `ForceFinalSummaryTurn = true` in pinned
`internal/agent/subagent_run.go` within the native child execution scope. The
registered runner remains unchanged for direct invocations. A new actual-scheduler
regression covers one- and two-turn children, denial of hallucinated final-turn
tools, earlier-turn tool execution and reuse of the unmodified registered runner.
It reproduced the missing directive before the fix. All 26 child-integration
tests pass, including descendant joins, steering and durable child recovery.
Original Go final-summary and child partial-progress regressions also pass.

Fresh workspace **1,397 passed / 0 failed / 30 ignored**; strict Clippy/rustdoc,
feature matrix, platform-free consumer, 20 offline scenarios, host-session, six
doctests, pinned oracles, 27 ledger/audit validators and source locks pass.
Ledger remains **465 verified / 8,426 unresolved / 251 excluded**. Automatic child
workspace/budget guidance, dynamic instruction wrapping and complete ledger
acceptance remain open. No turn-budget warning injection or live-provider claim.

### Automatic child workspace and budget guidance

The shared child engine now appends the pinned workspace and child-budget blocks
to its cloned initial agent. Dynamic providers receive the cloned agent and have
the same suffix appended to their output. The workspace formatter is shared with
the facade through `adk-runtime`; its existing public facade path is unchanged.
An empty workdir omits only the workspace block. Budget guidance uses the resolved
scheduler limit, including scheduler-clamped submissions. Read-only child ceilings
are reflected in the guidance; prompt text itself grants no tool authority.

The new scheduler matrix covers static/dynamic instructions, durable/non-durable
execution, empty/nonempty workdirs, full/read-only ceilings, repeated invocations,
three requested/resolved limits and subsequent direct runner reuse. A separate
handoff regression verifies that the suffix is not inherited by handoff targets.
All **28 child-integration tests** pass. The initial workspace implementation
exposed a real durable-resume regression: narrowing access changed generated
instructions and therefore the configuration fingerprint. The fingerprint now
uses the registered instruction graph, while the execution graph regenerates
policy-derived guidance. The crash/recovery regression verifies both preserved
tool cursors and the change from full to read-only prompt guidance.

Fresh pinned Rust **1.88** workspace results: **1,399 passed / 0 failed / 30 ignored**.
Strict Clippy on both changed crates and their dependencies, strict rustdoc,
**seven doctests**, changed-file format checks and diff hygiene pass. The full
facade feature matrix, independent platform-free consumer, **20 offline feature
scenarios** and host-session example pass. The example script's `cargo clippy`
launcher hit the worker's missing `/proc/self/exe`; equivalent direct-driver
Clippy and offline example commands passed. Claim-bound pinned Go oracles and Rust evidence were
regenerated; all **27 ledger/audit validators** and source locks pass. The attached
SDK/platform checkouts initially drifted and were restored to their clean recorded
revisions; the inspected child-engine source was identical at the newer and pinned
SDK revisions. System Rust 1.97 strict Clippy reported an existing unrelated
`collapsible_if` lint; authoritative checks use the repository-pinned compiler.

Ledger counts remain **465 verified / 8,426 unresolved / 251 excluded**. This
composition change is not a whole-child-engine or full reusable-SDK closure claim.
Remaining session assembly/reconfiguration and ledger obligations are unfinished;
credentialed live services remain unverified. No automatic turn-budget warning,
CLI/evaluation/Terminal-Bench, production release or default-switch work was added.

### Owner-controlled session refresh

`Scheduler::reconfigure` now accepts a complete resolved future configuration and
executor through the exclusive owner. Admission captures an executor/semaphore
generation under the scheduler transaction lock. Queued and running tasks retain
their original runner, security and turn cap; new tasks use the refreshed catalog
and limits. Equal semaphore capacities reuse accounting; changed capacities create
separate generations, matching the pinned SDK snapshot behavior. Structural native
graph limits and the shared accounting budget remain immutable. Replaced host
executors are dropped after releasing scheduler locks, with a regression proving
that their destructors can reenter scheduler getters.

`Builder::owned_session(owner).subagent_host(host).build_session(...)` refreshes
future automatic children after all runner/tool construction succeeds. Both old
and new tool/MCP/project-state owners remain retained by the session. Borrowed
turns cannot reconfigure it. An explicitly empty catalog removes future admissions
without deleting existing task history. The actual facade regression blocks an
old child before tool dispatch, refreshes to a different model and owner-bound tool,
then proves both generations complete against their own resources. The offline
feature example now also demonstrates owner refresh and a third child call.

Durable schedulers with tracked records **reject refresh without mutation**. Their
checkpoint format cannot reconstruct multiple executor generations after restart;
silently replacing an executor would either invalidate recovery fingerprints or
resume against the wrong configuration. This is an explicit remaining acceptance
blocker, not completed durable-reconfiguration parity. Hosts must use fresh-owner
recovery with reconstructible wiring. In-memory refresh and immutable native
safety ceilings are documented separately from Go partial-pointer/default rules.

Fresh Rust 1.88 workspace: **1,404 passed / 0 failed / 30 ignored**, without compiler
warnings. Scheduler/facade regressions cover running and queued generations,
same/changed concurrency, catalog/security/turn updates, invalid-update rollback,
empty catalogs, resource lifetime, durable rejection and destructor reentrancy.
Pinned Go `TestSubAgentRegistryConfigurePreservesTrackedTasks` and
`TestSubAgentRegistryConcurrentConfigureAndSpawn` pass. The all-feature facade
matrix/platform-free consumer, 20 offline scenarios, host-session example, strict
rustdoc and seven doctests pass. Final strict Clippy, formatting, source locks,
regenerated claim-bound evidence and all 27 ledger/audit validators pass. Ledger
counts remain 465 verified / 8,426 unresolved / 251 excluded. Existing claims are
not broadened; full SDK acceptance, durable automatic assembly and live-provider
evidence remain open.

### Explicit persistence for automatically assembled children

`Builder::subagent_store` now binds a host-owned `SchedulerStore` to a new
automatically assembled scheduler before admission. This makes the existing native
durable child engine reachable through ordinary facade assembly without requiring
the host to hand-build every child runner. It does not supply a database, file path
or lease; hosts still implement the documented atomic revision/fencing contract.
Missing child hosts, disabled/empty catalogs and attempts to replace an existing
scheduler's store are rejected rather than silently ignoring the store.

The new facade crash/recovery test uses a revision-checking journal. It suspends
after a tool completion has been persisted, drops the original owner, rebuilds a
fresh session, restores the authoritative checkpoint and explicitly resumes. The
tool executes exactly once and the second model call produces the recovered result.
A separate failed-admission test proves no model dispatch and preserves the host
error from explicit close after ambiguous persistence failure. Borrowed-session
rejection preserves its owner; failed construction with transferred ownership
closes that owner, as required by the existing lifecycle contract.

Fresh Rust 1.88 workspace: **1,407 passed / 0 failed / 30 ignored**, no warnings.
Strict Clippy/rustdoc, seven doctests, the full facade feature matrix and standalone
consumer, 20 offline scenarios and host-session example pass. Claim-bound pinned
oracles/evidence, all 27 ledger/audit validators, formatting and source locks pass.
The source mapping is documented accurately: Go exposes its public scheduler's
checkpoint callback, not a `runtime.Builder.Config` store field; Rust binds it at
construction instead of allowing unsafe live store replacement.

The ledger remains **465 verified / 8,426 unresolved / 251 excluded**. Persistent
assembly and explicit recovery are now implemented, but refreshing a durable
scheduler with tracked executor generations is still unsupported and must not be
called complete. Full per-record SDK acceptance and live-provider evidence remain
open; CLI/evaluation/Terminal-Bench and production release/defaults are unchanged.

### Complete signed Usage type and public compatibility path

The three remaining type/alias/file records for `internal/agent/usage.go` and the
public `Usage` alias now have bounded closure. All seven members were already
verified independently. A new facade test accesses `adk::codec::dto::Usage` and
compares the same 64 pinned Go operand pairs, including zero, negative values,
cache-only counters and signed extrema. It checks all five serialized counters,
zero-cache omission, wrapping addition, input/output-only totals and preservation
of the borrowed operand. This is the signed compatibility DTO, not a claim about
unsigned native runtime accounting or 32-bit Go integers.

The test passes with only `compat` enabled and is absent with default features
disabled. Its JSON dependency is explicitly test-only; the initial minimal-feature
build exposed that missing declaration, which was corrected before verification.
Strict Clippy and formatting pass. Refreshed pinned oracles, claim-bound Rust
evidence, all 27 audit validators and immutable source locks pass. Counts are now
**468 verified / 8,423 unresolved / 251 excluded**. The usage group's cost helpers
and source cost regressions remain unresolved; neither the whole usage group nor
the complete aliases file is claimed.

Final Rust 1.88 all-feature/all-target workspace: **1,408 passed / 0 failed /
30 ignored** across 145 targets, no warnings. All 46 remote CI checks for the
preceding implementation head `2956f24` passed, including Linux/macOS enforcing
backends and durable PostgreSQL; that result does not establish remote CI for
this subsequent audit-only change.

### Cumulative price certainty, compaction and durable restoration

Native `RunMetrics::cost_known` now preserves the distinction between known zero
and a partial numeric total containing unknown prices. Response accounting keeps
the optional estimate instead of replacing it with a known zero before recording
usage, and still invokes the estimator exactly once per response. Retained failed
stream responses preserve the same distinction. Unknown prices cannot become known
merely because a later response has a price.

The audit also found two compaction paths that called the numeric-only estimator:
LLM summaries and the provider `NativeCompactor`. Both now use optional estimates.
`CompactedHistory::cost_known` lets custom compactors retain a partial numeric charge
without presenting it as a complete estimate; deterministic local compaction sets
known zero explicitly. A budget regression checks that an uncertain nonzero
compaction charge is still enforced, not discarded. Native checkpoint restoration
retains certainty; missing historical fields and reconciled Go checkpoints remain
unknown. No-response fallback display pricing and child outcome projection remain
separate parity obligations.

Independent pinned Go execution reports **790 passing tests/subtests and five
skips**, including both `cost_test.go` regressions. Native tests reproduce their
exact 0.25+0.75 known sum and 0.25+unknown partial sum, plus reversed unknown order,
known zero, failed streams, summary prices, provider price routing and checkpoint
restoration. Only those two source tests and their complete test file are closed:
**471 verified / 8,420 unresolved / 251 excluded**. The `cost.go` implementation
file and entire usage group remain unresolved rather than treating this bounded
cost-reporting fix as full helper equivalence.

An initial strict Clippy run caught the model-name tuple element made unused by
single-pass estimate recording. It was removed, not suppressed. The corrected
workspace passes strict Clippy, strict rustdoc and seven doctests. Final bound
evidence, all 27 audit validators and immutable source locks pass.

Final all-feature/all-target Rust 1.88 verification: **1,411 passed / 0 failed /
30 ignored**, 145 targets, no warnings. The full facade feature matrix,
platform-free independent consumer, all 20 offline feature scenarios and the
host-session example pass with the updated compaction contract.

### Child completion price projection and persistence

Managed child outcomes and scheduler task snapshots now carry exact `cost_usd`
and `cost_known` from run metrics, independently of rounded budget micros. The
fractional-price regression now covers priced fractions, known zero, unknown
prices in either response order and fully unknown prices. It verifies the exact
display sum, single estimation per response and unchanged cumulative budget
rounding. Completion and explicit host reconciliation preserve both display
fields in persisted task snapshots. Legacy snapshots retain budget counters but
do not fabricate a known display price.

Host executor prices are validated before publication: negative/nonfinite values
fail the task while preserving separately reported consumption. Reconciliation
and restore reject invalid prices without partially publishing state. Tests cover
each boundary and durable round trips. Model-facing task-tool JSON is deliberately
unchanged; the source adapter does not expose these fields in those projections.

Fresh Rust 1.88 workspace: **1,413 passed / 0 failed / 30 ignored**, 145 targets,
no warnings. Strict Clippy/rustdoc and seven doctests pass. Refreshed claim-bound
oracles/evidence, 27 audit validators, source locks and formatting pass. The ledger
remains **471 verified / 8,420 unresolved / 251 excluded**: this completes native
price propagation but does not claim source residual/fallback display estimation,
every other child outcome field, or complete SDK acceptance.

### Ordered filesystem argument decoding

Write, Edit, Move and Delete now retain raw JSON field order during typed input
updates, including duplicate fields, case-folded aliases (with Go's Unicode
folding), null scalar no-ops and first scalar type errors. Operation-specific
unknown fields are ignored instead of being decoded as another operation's
parameters. Raw/parsed projection disagreement fails before filesystem mutation.
The workspace boundary still checks the final decoded path; an independent
regression covers conflicting path aliases for all four operations and verifies
both internal and external files remain unchanged on rejection.

Pinned Go comparison passes **166 filesystem + 18 skills cases**, comparing
results and resulting trees/modes. The 89 added filesystem cases exercise ordered
input decoding. Malformed JSON diagnostic parity remains unclaimed. The Go
telemetry sidecar reports unavailable `/proc/self/exe` in this environment; the
comparison process itself exits zero after all 184 comparisons pass.

Fresh Rust 1.88 workspace: **1,419 passed / 0 failed / 30 ignored**, 146 targets,
no warnings. Strict Clippy, rustdoc, doctests, formatting and whitespace checks
pass. Bound Rust evidence, all 27 audit validators and immutable source locks
pass. Ledger counts remain **471 verified / 8,420 unresolved / 251 excluded**;
this bounded decoder correction does not close the whole filesystem group.

### Filesystem parameter acceptance

The evidence runner now independently regenerates the pinned SDK tool schemas
with a non-mutating `--check` mode and runs the 184 lifecycle/tree comparisons
before recording Rust evidence. It binds the schema manifest, generators, inputs
and independent schema/runtime regression tests. Validator regressions reject
missing schema or lifecycle execution markers.

Exactly **15 parameter records** for Write/Edit (both access variants), Move and
Delete are now verified: advertised property types/descriptions, required/default
metadata, typed decoding and runtime parameter semantics. ApplyPatch parameters,
whole-tool records, malformed-JSON diagnostic spelling and OS-specific errors are
not covered by these closures. Ledger: **486 verified / 8,405 unresolved / 251
excluded**. The immutable baseline is unchanged.

An initial verification attempt incorrectly expected the schema generator to emit
JSON on stdout; it actually writes the fixture. That integration was corrected
with the non-mutating check mode. Final reference/Rust verification, 27 validators,
Python compilation, Go vet, source locks and whitespace checks pass. No Rust
implementation changed after the 1,419-pass workspace verification above.

### Edit diff regressions and JSON boundary follow-up

All 19 pinned `editdiff_test.go` regressions now have native counterparts: exact
line/hunk rendering, nearby and distant replacements, cumulative line-number
shifts, newline joins/cascades, EOF behavior, no-match/truncation behavior and
actual Edit result text in both access modes. Source tests and native tests pass.
The 19 test records and their complete test file are closed, not the entire
`editdiff.go` implementation/private-helper API or whole Edit tool.

Further direct comparison exposed two valid-input bugs after the parameter audit:
Go replaces lone UTF-16 surrogate escapes while serde_json rejects them, and Go
accepts deeply nested ignored fields beyond serde_json Value's recursion limit.
Both were reproduced as failing Go/Rust comparisons before fixing them. The
filesystem decoder now normalizes lone surrogates without changing valid pairs,
escaped text or ordered duplicates, and validates/retains raw field values rather
than recursively allocating ignored values. It enforces Go's 10,000-level bound;
malformed JSON diagnostic spelling remains unclaimed. The explicit raw-value
feature is declared on adk-tools rather than relying on workspace unification.

The permanent oracle now compares **200 filesystem + 18 skills** cases, including
surrogate paths/content, ignored fields and depth-limit boundaries. All 218
comparisons pass. Parameter evidence requires the additional surrogate/depth
regressions. An exploratory 460-case deterministic Edit probe also passed against
Go (plus 18 skills); its scratch inputs are not used to claim ledger closure.

Final Rust 1.88 verification for this follow-up: **1,425 passed / 0 failed /
30 ignored**, 146 targets, zero warnings; **seven doctests pass**. Strict workspace
Clippy/rustdoc, formatting, full facade feature matrix and the external standalone
consumer pass. The consumer's complete reusable dependency closure remains
platform-free. Refreshed source-bound evidence, 27 audit validators and immutable
source locks pass. Ledger: **506 verified / 8,385 unresolved / 251 excluded**.
This closes 20 additional test/test-file records while strengthening, rather than
broadening, the existing 15 parameter claims. Issue #11 remains incomplete.

### Private diff/input mapping and constructed-tool metadata

Source review maps the private diff structures/builders and complete diff module
to native byte-indexed Lines/Run/render code. Added independent Go/native cases
cover empty new ranges, blank lines and the exact 256/257 rendered-match boundary;
actual file replacement remains complete when only the display is truncated.
Ten additional implementation/type/field/configuration/file records are verified.

Thirteen private input type/field records now map to the typed native inputs and
ordered decoder. New runtime tests and Go comparisons explicitly preserve zero
values: omitted/null Write content and Edit replacement text are valid empty
strings despite required schema metadata; omitted/null replace_all remains false;
missing paths fail without mutation. Both write access variants are exercised.

Thirty constant metadata-method records are verified using both the per-dimension
schema tests and actual Registry-constructed filesystem tool instances across all
three access modes and four environment samples. This checks names, descriptions,
parameter schemas, read-only/approval flags and no per-tool timeout (Go zero maps
to native None). It is not just a manifest comparison; IsEnabled predicates,
Execute implementations and whole-tool records are not claimed by this slice.

Fresh workspace: **1,429 passed / 0 failed / 30 ignored**, 146 targets, zero
warnings. Strict Clippy on changed crate/tests, formatting, Python compilation,
**244 Go/Rust comparisons**, refreshed bound evidence, 27 audit validators and
source locks pass. No production implementation changed after the prior full
facade matrix/consumer and strict workspace docs checks. Ledger: **559 verified /
8,332 unresolved / 251 excluded**. The 53 new closures do not complete #11.

### ApplyPatch raw-input preview safety

A native regression and independent pinned-Go probe reproduced an unsafe input
projection mismatch: `"dry_run":false,"DRY_RUN":true` previewed in Go but wrote
files in Rust because ApplyPatch interpreted sorted projected fields. ApplyPatch
now uses the same ordered raw decoder as the other filesystem tools. Null scalar
updates retain prior values, aliases obey input order, and the first type error
or inconsistent raw projection prevents mutation.

Two new native tests cover preview/mutation ordering, null updates, scalar type
errors and projection mismatches. Twenty-eight new independent lifecycle cases
cover aliases, duplicates, defaults, root types, lone/paired surrogates and the
10,000-level JSON nesting boundary. All **272 lifecycle results/trees** and
**1,176 ApplyPatch results/trees** match the immutable Go SDK. The evidence runner
now binds the full patch implementation, replay, tests, inputs and live oracle,
and requires both comparison counts; no additional acceptance IDs are claimed.

Rust 1.88 verification: **1,431 passed / 0 failed / 30 ignored**, 146 workspace
targets and zero warnings; **7 doctests** pass. Strict workspace Clippy/rustdoc,
facade feature matrix, standalone consumer/dependency purity and Go vet pass.
The replay example's final formatting was corrected and all source-bound evidence
regenerated. After a runtime restart, the scratch toolchain/logs were lost but
the final evidence survived and validates against the restored sources. Restarted
reference checkouts initially drifted; both exact source pins and the SDK tag were
restored, then immutable locks and all **27 audit validators** passed again.
Ledger remains **559 verified / 8,332 unresolved / 251 excluded**. This safety fix
does not complete #11 or remove the remaining durable-generation recovery work.

### ApplyPatch input acceptance

Five additional records now map the private two-field input type, its Patch and
DryRun fields, and the corresponding two schema parameters. This uses the preview
safety tests and both independently executed Go corpora above; the complete
source-bound evidence was regenerated after recording the exact claims. All 27
audit validators and immutable source locks pass. Ledger: **564 verified /
8,327 unresolved / 251 excluded**. This is input/default/schema acceptance, not
whole-Execute, rollback, availability or exhaustive malformed-JSON parity.

### Named durable scheduler generations

Durable owned sessions can now opt into immutable host-defined executor generation
IDs from construction. Version-2 checkpoints retain each accepted task's generation,
complete scheduler configuration, active generation and concurrency-pool identity.
Named refresh persists its activation before publishing wiring and poisons the owner
on failed or cancelled acknowledgement. Fresh recovery requires exact explicit
host reconstruction bindings; historical task execution, security and both resume
paths use their accepted generation rather than the latest catalog. Equal-capacity
consecutive generations retain shared permits across restart. Legacy v1 recovery
and unnamed tracked-refresh rejection remain unchanged; no callback serialization
or implicit v1 relabeling was introduced.

The facade adds `subagent_generation_id` and `subagent_historical_generations`.
A parent review caught that an empty refreshed active catalog could not be rebuilt
for recovery. Named empty assembly now creates its scheduler without advertising
new child tools; the offline continuation test covers both empty and replaced
catalogs, historical native checkpoints and no replay of an already completed tool.
A further store test commits activation, blocks before acknowledgement, then cancels:
the original owner is poisoned and recovery uses the authoritative new generation.

Independent security review found no additional blockers under the explicit trusted
reconstruction/fenced-store contracts. Hosts remain responsible for retaining exact
recipes and historical resource owners. Automatic serialization of historical
assemblies and migration of existing unnamed v1 sessions are not supported. See
`docs/subagents.md` for APIs, limits and runnable recovery test commands.

Final Rust1.88 verification after these fixes: **1,440 passed / 0 failed / 30 ignored**,
146 workspace targets, zero warnings; **7 doctests** pass. Strict workspace
Clippy/rustdoc, formatting, full facade matrix, standalone consumer/dependency
purity, fresh source-bound Go/Rust evidence, **27 audit validators** and immutable
source locks pass. The earlier full job was intentionally stopped while the empty
catalog fix was underway; only the final complete run is reported here. Ledger
remains **564 verified / 8,327 unresolved / 251 excluded**: this implementation
removes the tracked-task refresh limitation for named sessions but does not claim
whole-ledger closure. All **46 remote checks** passed at the preceding `21e8322`
checkpoint; that result is not remote verification of these new changes.

### Successful-child residual display estimates

Completed child outcomes now preserve frozen response prices and estimate the
remaining nonnegative token/cache usage once with the original child model, as
in the pinned SDK cost helper. Requests-only residuals do not trigger pricing.
Actual cumulative cost and shared budget micro-units remain unchanged; failed,
cancelled and nonterminal outcomes retain their accounting projection.

`RunMetrics.response_cost` and native checkpoints retain the response subtotal
and certainty. Missing legacy/reconciled provenance stays unavailable: valid
cumulative USD remains visible but unknown, without re-estimating or double-pricing
history. `CostEstimator::estimate_display_cost` is default-compatible and can
represent a nonzero uncertain display estimate, which the previous Option-only
pricing hook could not express. Invalid/overflowing display prices retain only
valid available amounts and mark certainty unknown.

The independently executed Go oracle calls the original private helper in an
immutable source archive and records **21 cases**, including exact fallback usage
arguments/call counts; both original Go cost regressions pass. The native comparator
matches every case. Integration tests cover different actual versus display prices,
original versus active models, native durable restoration, small-price precision,
error separation and invalid persisted amounts. Independent review found no concrete
blockers under the existing trusted-checkpoint model; cross-field corruption bounds
are not claimed as a new integrity guarantee.

Final Rust1.88 workspace: **1,451 passed / 0 failed / 30 ignored**, 146 targets,
zero warnings; **7 doctests** pass. Strict whole-workspace Clippy/rustdoc,
formatting, full facade feature matrix, standalone consumer/dependency purity,
refreshed source-bound evidence, **27 audit validators** and immutable source locks
pass. Source/harness/native-helper inputs and independent execution markers are
bound into the evidence runner. Ledger remains **564 verified / 8,327 unresolved /
251 excluded**: these implementation and fixture results do not silently close
whole cost methods, signed-overflow behavior or unrelated retained contracts.

### Bounded child-event and collection-field mappings

Thirteen additional event records are verified: nine owned UTF-8 ChildToolEvent
projection fields and the ContentEvent tools/MCP-server/dependency/waiting collection
fields. Child projections use twelve complete pinned-Go input/output cases, including
rejection, zero-value field presence, start/end exclusivity, signed duration extremes,
uncapped payloads and ownership after input mutation. Each collection has separate
canonical and compatibility-parser cases for nil/empty/populated values, null
string elements and wrong types. Tools-specific duplicate/backing-array cases are
not generalized as independent test evidence for the other collections.

These are field/projection/parser claims, not whole DTO decoding/type closure,
invalid-UTF8 Go string support, timestamp parity, runtime producers or in-memory
nil-versus-empty identity. Existing source-bound tests and live Go oracles were
re-executed after recording the exact claims; all **27 validators** and immutable
source locks pass. No runtime implementation changed after the 1,451-test workspace
verification above. Ledger: **577 verified / 8,314 unresolved / 251 excluded**.
