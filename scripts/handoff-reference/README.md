# Independent pinned Go handoff oracles

`fixtures/handoff/sdk-handoff-filter.json` executes the actual exported
`agentsdk.RemoveAllToolsHandoffInputFilter(input, newItems)`. No filtering logic
is reimplemented by the generator; no Rust expected values or worker CLI are
involved. The standalone module uses Go 1.26.2 and the local replacement
`github.com/gratefulagents/sdk => ../../repos/sdk`. The SDK must be clean at
`1dc92b73900fac74dc357a938e4b5eee6392b418` (v0.0.115).

## Reproduce and verify

From the repository root, with the caller's Go environment configured:

```sh
export GOROOT=/usr/local/go GOTOOLCHAIN=local
export GOPATH=/workspace/scratch/go GOCACHE=/workspace/scratch/go-cache
export PATH=/usr/local/go/bin:$PATH
(cd scripts/handoff-reference && go run -mod=readonly .) \
  > fixtures/handoff/sdk-handoff-filter.json
python3 scripts/handoff-reference/check.py
(cd scripts/handoff-reference && go test -mod=readonly -count=1 ./...)
(cd scripts/handoff-reference && go vet -mod=readonly ./...)
(cd scripts/handoff-reference && go build -mod=readonly -o /dev/null .)
test -z "$(gofmt -l scripts/handoff-reference/*.go)"
```

Regeneration is intentional; `check.py` does not rewrite files. It inherits the
caller environment and accepts `GO` (default `go`). Following
`scripts/fileconfig-reference/check.py`, it verifies the clean pin, resolves the
actual local module replacement, checks fixture revision/schema, regenerates
twice in memory, compares exact bytes against the fixture each time, and
rechecks SDK cleanliness. No environment paths are hard-coded in the checker.

Verified using Go 1.26.8 linux/amd64: test, vet, build and two byte-identical
regenerations passed. A Go telemetry-sidecar warning about `/proc/self/exe`
appeared on stderr in this environment, but the commands exited successfully.

## Fixture contract

Schema v1:

```text
{schema_version: 1, sdk_revision, cases: [
  {name, source_only: [reason, ...], input, new_items, output}
]}
```

`input` and `new_items` are the first and second SDK arguments, respectively;
each is null or an array. `output` is the actual returned SDK sequence encoded
as an array. There are **23 cases, 51 input items, 8 second-argument items and
28 returned items**. Eight cases have nonempty `source_only`; 15 do not. This
is coverage accounting, not a claim that a Rust comparator has passed.

Each item retains the SDK numeric `Type` and exported field names `Message`,
`ToolCall`, `ToolOutput`, `HandoffCall`, `HandoffOutput`, `Reasoning`, `Compaction`,
`ToolApproval`, and `Agent`. Payloads are the actual SDK structs serialized by
Go's encoding/json, including their existing snake_case tags and omitempty
behavior; unused pointer fields remain null. The numeric types are:

| Type | SDK meaning | Observed treatment |
| ---: | --- | --- |
| 0 | message | kept |
| 1 | tool_call | stripped |
| 2 | tool_output | stripped |
| 3 | handoff_call | stripped |
| 4 | handoff_output | stripped |
| 5 | reasoning | stripped |
| 6 | tool_approval | stripped |
| 7 | compaction | kept |
| -1, 99 | unknown | kept |

### Necessary Agent serialization boundary

**This is not a claim that raw `json.Marshal([]agentsdk.RunItem)` works for
attributed items.** SDK `Agent` has public function fields (including
`InstructionsFn`) without JSON exclusions; even a Name-only agent with nil
functions raises `json.UnsupportedTypeError`. To record distinct labels,
`encodedItem` embeds the actual returned SDK RunItem and shadows only `Agent`
with `null` or `{"Name": "exact returned agent name"}`. All supplied agents
have only Name populated. This narrowly documented serialization adapter is
not an SDK Agent configuration serializer or a Rust result projection.

Tests verify that every non-Agent field is identical to direct SDK JSON,
agent labels are unchanged, null/empty slices are not collapsed, and the
returned SDK items retain the original data and agent pointers. No type
conversion, role insertion, sorting, text trimming, payload rewriting,
unknown-type coercion or semantic normalization is performed. JSON escaping
and indentation are ordinary encoding/json behavior. No snapshots are used:
they would rename/coerce unknown types and lose nil/empty distinctions.

## Observations and native boundaries

- Nil input, empty input and all-stripped input each return nonnil empty `[]`.
  The fixture keeps nil input as null, separate from empty input as `[]`.
- All six stripped kinds have populated individual cases. The all-six and
  mixed cases exercise the same exported helper, not handwritten outputs.
- Filtering is stable and shallow: retained user/assistant messages, phases,
  compaction payloads, repetitions and distinct alpha/beta agent labels remain
  unchanged and ordered. Go tests also check pointer identity and unchanged
  argument slices.
- `new_items` is ignored, not appended or independently filtered. Three cases
  pass nonempty second arguments, including messages and compaction that would
  otherwise survive, with nil, empty and nonempty first arguments.
- The default switch branch preserves unknown numeric types and the zero-value
  RunItem (Type 0 with no Message payload). These are source observations, not
  validation guarantees.
- SDK MessageOutput has **no Role field**. Nil Agent is conventionally user;
  a nonnil agent is conventionally assistant. Agents named `system` and
  `developer` are valid labels, **not explicit system/developer role messages**.
  Those two cases are source-only for this role boundary. Native system and
  developer role behavior requires separate Rust tests; the Go oracle cannot
  honestly represent it.
- Native `adk_core::RunItem` has no separate handoff-output or tool-approval
  variant. Their individual cases and the two combined cases containing them
  are source-only. Approval sidecars and codecs are different representations,
  not permission to manufacture native history variants.
- Unknown numeric kinds and a message without a payload cannot be faithfully
  represented by the native enum; their cases are source-only too.
- Native handoff carries a call_id that the SDK HandoffCallData does not, and
  SDK HandoffCallData carries from_agent absent from that native variant. The
  strip-handoff-call and native-mixed cases can check removal of a native
  handoff; they do **not** establish a lossless payload conversion or synthetic
  call-ID policy. Do not describe a fabricated ID as Go evidence.

`source_only` is a comparator boundary marker, not an expected Rust error.
Do not count excluded cases as native passes. `native_mixed_sequence` excludes
SDK-only handoff-output/approval kinds while preserving the other interleaved
items. Agent attribution must remain independent of message role, not silently
discarded. This oracle does not implement or verify the Rust comparator, full
Agent configurations, arbitrary malformed payloads, or handoff execution.

## Files, license and provenance

- `main.go`: SDK execution, schema, and the explicit Agent encoding boundary.
- `scenarios.go`: executable inputs only.
- `main_test.go`: semantic observations, serialization boundary and fixture
  repeatability.
- `check.py`: clean-pin, module replacement and exact-byte checks.
- `go.mod`, `go.sum`: isolated local-replacement module.
- `../../fixtures/handoff/sdk-handoff-filter.json`: generated observations.

This harness and fixture follow the repository's **GPL-3.0-only** source-derived
contract policy (`../../NOTICE.md`, full license `../../LICENSE`). The upstream
SDK is Grateful Agents SDK, GPL-3.0, with its original license at
`../../repos/sdk/LICENSE`; no upstream code is relicensed. The checker is adapted
from `../fileconfig-reference/check.py`, and the module setup from
`../fileconfig-reference/go.mod` (tidy removes unused yaml). SDK behavior is
reused by importing and executing the original exported helper, not copying or
reimplementing its switch. Source provenance at the immutable pin:

- `pkg/agentsdk/aliases.go`: exported helper and type aliases.
- `internal/agent/handoff_filters.go`: actual filtering implementation.
- `internal/agent/items.go`: item types, numeric values and payload JSON tags.
- `internal/agent/agent.go`: Agent labels and function-field JSON limitation.
- `internal/agent/handoff_enhancements_test.go`: upstream basic filtering test;
  scenarios here independently broaden its bounded coverage.

Source URLs share this base:
`https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/`.
Only this new module and its fixture are owned by this work; no SDK, Rust,
shared ledger or other reference script is changed.

## Runtime catalog composition oracle

The default `go run -mod=readonly .` invocation and filter fixture bytes/schema
remain unchanged. `go run -mod=readonly . catalog` independently calls the
**actual exported `runtime.BuildAgentWithSpecialists`** and generates
`fixtures/handoff/sdk-catalog-handoffs.json`. Generate from the root:

```sh
(cd scripts/handoff-reference && go run -mod=readonly . catalog) \
  > fixtures/handoff/sdk-catalog-handoffs.json
python3 scripts/handoff-reference/check.py
```

The checker verifies the clean pin before/after generation and regenerates
**each fixture twice**, without writing either. The new fixture contains
**30 cases, 44 returned specialist-map entries, 43 handoffs, and 7 source-only
cases** (23 cases without exclusions). These are source coverage counts, not
native comparator passes. The filter fixture remains 23 cases and SHA-256
`a6c9f0448ba4d8251ec43153009e41e979795ae57063c9a691f464bf9f2a926e`.

### Catalog schema v1 and projection boundary

```text
{schema_version: 1, sdk_revision, scope, filter_probe, cases: [
  {name, source_only: [reason, ...], input,
   output: {parent, returned_tools, specialists: null | {role_key: agent, ...},
            handoffs: null | [handoff, ...]}}
]}
agent = {name, instructions, dynamic_instructions, handoff_description,
         model, fallback_models, model_settings, tools, mcp_servers}
tool = {name, read_only, implementation}
handoff = {tool_name, description, target: agent, specialist_key: null | string,
           has_input_filter, filter_output, has_input_type, has_on_handoff,
           has_enabled_predicate, tool}
```

This is a **bounded public-definition projection, not full Agent/Handoff
serialization**. `instructions` comes from public `GetInstructions(nil)`:
specialists retain their exact static role text, while the parent exposes the
SDK-generated delegation guide. No text is rewritten or trimmed by the
projector. `specialist_key` is determined by returned **pointer identity**, not
by matching agent names; null identifies a target outside the returned map.
The target definition is recorded even for generic handoffs. The installed
input filter is invoked on the top-level `filter_probe`, with nil new items;
this is a small attachment check, not a replacement for the original filter
oracle. The existing explicit RunItem/Agent projection is reused.

Input uses snake_case harness keys and original exported Go RoleSpec,
TemplateSpec, HandoffFeatures and SubAgentFeatures field names; ModelSettings
retains its SDK JSON tags/omitempty rules. `catalogInput` is the complete
config subset supplied by this harness, not a serialized runtime.Config.
Provider is fixed to `openai` for **pure model-name routing**, WorkDir is empty,
and explicit Features is always nonnil. Only the features shown in input are
enabled; all others are zero. Two legacy enable booleans are supplied solely
to observe explicit-feature precedence. `parent_tools` sets Tools.ExtraTools;
`mode_routing` sets Modes.ModelRouting. There is no whole-Builder normalization
or BuildRunConfig call here.

Every case explicitly injects `NewRunnerWithModel(offlineModel{})` and a host
ToolBundle. No provider registry, credentials, client, MCP connection, model
response, tool execution, scheduler, or network access is used by generation.
The model's execution methods panic; no model emulation or GetModel/Name lookup
is needed for this pinned composition function. Host tools are inert named
test tools except `Bash`, an actual SDK `shell.BashTool`. Its actual
ToolAccessAdapter supplies `shell.ReadOnlyBashTool`; neither executes. This
records source adapter behavior, not a fake adapter implementation. MCP server
names are inert labels only. Module dependency downloads during setup are not
provider traffic; generation/checking works from the populated Go cache.

Nil/empty distinctions and **all array sequences** are retained (catalog,
tools, handoffs, fallbacks and MCP labels). Only JSON map keys are sorted by
encoding/json. The source itself sorts the specialist listing inside its
delegation-guide string; the harness does not sort or normalize that string.

### Independently observed source behavior

- Handoffs alone do not build catalog specialists. Any of async Task, Status,
  or Control gates specialist construction; GenericFallback alone does not.
  Explicit features override legacy EnableHandoffs/EnableSubAgents flags.
- Empty catalogs can independently produce a generic subagent named `agent`
  and a generic handoff target named `specialist`. The generic handoff target
  is not in the specialist map and has no tools or fallback models, even when
  the parent has them. Disabling handoff generic fallback returns no handoff.
- Role names are trimmed; catalog order, not map-key order, determines handoff
  order. Role descriptions/instructions remain exact on specialists. Handoff
  descriptions are trimmed and blank descriptions use the generated fallback.
- Sanitization lowercases and keeps ASCII letters/digits, translates each
  space/hyphen/underscore/dot to underscore without collapsing runs, drops
  other runes, trims outer underscores, and falls back to `specialist`.
  `Équipe .-- X_9!?` becomes `quipe_____x_9`; Unicode-only `审查💡` becomes
  `specialist`. Custom role and host-tool names are inputs; this runtime API
  has no custom handoff-tool-name option, so no such input is invented.
- Duplicate trimmed roles are first-wins; blanks are skipped. Distinct roles
  `a-b`/`a b` produce two `transfer_to_a_b` handoffs with distinct targets.
  Host-tool/handoff collisions also survive composition. This does not assert
  that the later runner accepts these tools.
- Model precedence is base → role ModelOverride → mode default → exact-name
  mode role override. Parent model/fallbacks remain the supplied base values
  in this function. Role, mode, then mode-role fallback overrides apply only
  when nonempty; explicit empty lists do not clear inherited lists. Order is
  preserved. Routing settings merge into the specialist's base settings.
- Config.ModelSettings is not used by this function: Reasoning, Verbosity,
  MaxTokens and Runtime.ParallelToolCalls construct the agent settings instead.
  BuildRunConfig has a different settings scope, not covered here.
- Read-only roles filter mutating tools after applying the actual Bash access
  adapter. Blank access is full; unknown access and read-only aliases are
  read-only. Parent tools are not role-filtered. The exact names `finish`,
  `present_plan`, `AskUserQuestion` are stripped only from specialists;
  `Finish` survives. Parent-tool feature gating does not remove host tools
  from specialists. Generated handoff tools report **read_only=true**.
- The returned tools are the parent's host-tool sequence: no synchronous
  specialist tools or async task tools are appended by this function.

### Exact native comparator boundary

No Rust output is generated, fabricated, tested or claimed here. A future
native comparator may compare the 23 unmarked cases' semantic public fields:
parent/specialist names, static role instructions and descriptions, model
names, ordered fallbacks, mapped ModelSettings fields, ordered tool names and
read-only bits, ordered handoff names/descriptions, and target identity via
specialist keys (or the independent generic target). It must explicitly map
SDK input names/settings and nil/empty representation rather than assuming a
native Config/Agent JSON layout matches this fixture.

The seven `source_only` cases exclude native strict duplicate/blank-role/name
collision validation and explicit-empty-fallback policies; they are **not
expected native errors or passes**. Tool `implementation` labels are harness
provenance, not native type names. The dynamic parent delegation-guide string,
dynamic-function marker and callback/type-presence flags document SDK
composition only unless a separate comparator explicitly supports those
surfaces. `filter_output` uses the earlier filter oracle's representation
boundary. Model execution, provider resolution, scheduling, handoff execution,
approval/policy enforcement and full prompt parity are outside this fixture.

### Catalog files, versions and provenance

Added `catalog.go` (SDK call and projection), `catalog_scenarios.go` (inputs),
`catalog_test.go` (source semantic assertions and two repeat generations), and
`../../fixtures/handoff/sdk-catalog-handoffs.json`. Updated `main.go` only to
dispatch the opt-in argument, `check.py` to verify both fixtures, and the
isolated `go.mod`/`go.sum` for runtime's dependency closure. Minimum Go remains
1.26.2; verification uses Go 1.26.8 linux/amd64, SDK v0.0.115 at the same pin.
The GPL-3.0-only repository/source-derived license policy and upstream GPL-3.0
provenance above apply equally to this fixture. Dependency versions/checksums
are recorded in this module; no vendored or relicensed upstream code is added.
Additional immutable-pin source paths:

- `pkg/agentsdk/runtime/builder.go`: exported composition, handoff sanitizer,
  generic target, model settings and parent tool-surface gates.
- `pkg/agentsdk/runtime/features.go`: explicit/legacy gates.
- `pkg/agentsdk/specialists.go`: role deduplication, access adaptation,
  signal stripping, fallback precedence and delegation instructions.
- `internal/agent/mode_routing.go`: mode/role model routing and settings.
- `internal/agent/handoff.go`: public definition and read-only tool adapter.
- `pkg/agentsdk/tools/shell/bash.go`: real Bash read-only access adapter.

## Managed subagent feature selection

`go run -mod=readonly . subagents` generates
`fixtures/handoff/sdk-subagent-selection.json` by executing the public
`runtime.NewBuilder(config).Build(context)` for all eight Task/Status/Control
combinations. Unlike the catalog projection, this exercises the stage that
attaches managed subagent tools. Each configuration supplies one catalog role,
no ordinary tool features and an explicit temporary workspace. A synthetic test
API-key placeholder and a loopback HTTP trap isolate provider configuration;
no inference, task spawning or external credentials are used, and any provider
request makes generation fail. All bundle closers are called before recording
the next case. The checker regenerates this third fixture twice too.

The output records the actual ordered `bundle.Tools` names and whether the SDK
created a session scheduler. Rust compares the **name sets**, not tool ordering,
across all eight selections. Its registry sorts names, whereas the SDK adapter
order is preserved verbatim in this fixture. Native tests also require an
explicit host-owned scheduler for every nonempty selection and check retained
ownership; these are native lifecycle assertions, not claims of equivalent SDK
automatic allocation. Descriptor schemas, actual child execution and complete
Builder output remain outside this narrow comparison.

`subagents.go` and `subagents_test.go` use the same pinned module and
GPL-3.0-only provenance policy as the other harness files. Inspected upstream
sources are `pkg/agentsdk/runtime/{builder,features}.go` at the pin above,
particularly `attachAsyncSubAgentTools` and `asyncSubAgentToolNames`. The generator
invokes the exported builder instead of reimplementing either selector.

## Final-summary request projection

`go run -mod=readonly . final-summary` executes the pinned public SDK runner in
complete and streaming modes with `ForceFinalSummaryTurn` off/on and one/two
available turns. The eight cases in `fixtures/handoff/sdk-final-summary.json`
record exact instruction bytes, ordered advertised tool names, executed tool
counts and final output. A local scripted model and tool provide deterministic
responses; no network, credentials, worker CLI or evaluation adapter is used.
`check.py` regenerates this fixture twice and checks exact bytes.

Rust compares those fields in
`final_summary_turn_is_opt_in_and_request_only_in_run_and_stream`. Additional
Rust tests cover retries, pending managed-child joins, observer snapshots,
provenance and durable recovery; these are not independently established SDK
parity. In particular, native budgets count retry attempts, and these Go cases
do not test retries. The test deliberately does not project complete SDK
RunResult/events or assert equivalent malformed-response handling.

`final_summary.go` and `final_summary_test.go` follow the same GPL-3.0-only
provenance policy. Inspected sources at the pin above are
`internal/agent/runner.go` (`finalSummaryTurnDirective` and its final-turn
branch) and the public `pkg/agentsdk` runner exports. The oracle invokes that
implementation; it does not recreate its directive or selection logic.
