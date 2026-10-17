# Managed subagents

The opt-in `runtime` feature provides a local, explicitly owned child scheduler.
`adk_runtime::subagent` contains the scheduling/recovery API;
`build_subagent_task_tools`, `AgentAsTool`, `RunnerChildExecutor`, and
`SubagentSession` provide runner adapters. Both delegation surfaces submit through
one `ChildExecutor`; there is no separate unmanaged agent-as-tool execution path.

## Session ownership

A host registers child `Runner`s in `RunnerChildExecutor`, configures permitted
agent names and security baselines in `SchedulerConfig`, then constructs a
`Scheduler` with a session `Context` and optional `SchedulerStore`. Keep that
exclusive owner alive across parent turns. Clone `SchedulerHandle` for tools and
checkpoint integration, and wrap it in an `Arc<SubagentSession>`.

Install the same session in `RunnerConfig.subagents` and pass it to
`build_subagent_task_tools(session, default_agent)`. This binds incremental result
delivery and final joining to the parent runner. Merely attaching tools without
the runner session does not install a final-answer gate.

The session context supplies cancellation and the outer deadline; an individual
`subagent_wait` or synchronous tool timeout does **not** close the scheduler.
The host must call `owner.shutdown().await?` when the session closes. This closes
admission, cancels and joins children, and persists shutdown state. Drop requests
abortion as a safety fallback but cannot acknowledge durable asynchronous cleanup.
Custom child executors must not detach tasks or spawn unowned blocking work.

Nested delegation uses `ChildControl::delegation_handle()`, not a fresh root
scheduler. Native runner tools and handoff graphs are rebound to that scoped
session; foreign session handles never escape into the child. Nested waits yield
the parent's execution permit and reacquire it before continuing, including after
an explicit wait timeout. This allows concurrency-one nesting without exceeding
the global cap. Reacquiring a busy slot can extend the wall-clock wait beyond its
result-wait timeout. Dropping a suspended wait cancels its parent subtree rather
than detaching a background permit-reacquisition task. Concurrent waits from one
child execution are rejected.

## Model-facing tools

`AgentAsTool::new` exposes a named synchronous specialist. A completed successful
call returns the child's final text directly, or `(no output)` for an empty result;
JSON-looking text is not wrapped or re-encoded. Parsed non-string structured
outputs have no final text; use an extractor to return a textual projection.
The parent output policy still
applies untrusted markers and size limits. In contrast, `subagent` retains its
structured task envelope. Native timeout/failed-task envelopes are retained for
managed recovery; timeout never hides the still-active task identity.

Use `RunnerChildExecutor::with_output_extractor(agent_name, Arc::new(|result| ...))`
to transform a successful child's full `RunResult` before its task result is
stored and returned. Empty extraction falls back to the original final output.
The callback is scoped to an executor registration, not a tool wrapper, so both
named and managed delegation to that registration receive the transformed result.
Use separate registrations when only a named tool should transform output.
Failed/cancelled runs never invoke it; scheduler panic containment still applies.

When a failed child returns partial run state, its error includes the latest
trimmed, nonblank message that does not begin with `[SYSTEM]`, under
`Partial progress before the run ended:`. The excerpt uses the SDK's
1,600-character head/tail elision, so summary views retain the finding even
when they omit `result`. Native retained result text remains a separate
4,096-character tail fallback; named-tool recovery envelopes remain as described
above. This error projection does not claim complete Go failure-envelope parity.
Extractors should be deterministic: durable completion recovery may run the
projection again. They do not change child history, usage or checkpoint contents.

- `subagent`: exactly one `message` or nonempty keyed `tasks` DAG. `mode` defaults
  to `sync`; `background` returns IDs immediately. DAG keys are local to the call,
  and `task_ids_by_key` maps them to retained task IDs. Dependencies may refer to
  local keys or earlier IDs. Invalid DAGs are rejected before any node is admitted.
- `subagent_status`: `summary`, `activity`, `results`, or `graph`. Rereading results
  does not consume or delete them.
- `subagent_wait`: event-driven `all` or `any`, with optional task IDs and timeout.
  Omitted IDs select active or not-yet-delivered tasks. A timeout returns current
  state, not a false child failure and not a child cancellation.
- `subagent_control`: `message` or `cancel`. Steering carries stable call identity
  for deduplication; identical text in distinct calls is still distinct steering.

`all_success` is the default dependency policy. Failed/cancelled dependencies
prevent that node from executing. `all_terminal` permits cleanup/aggregation after
any terminal dependency outcome; reconciling remains nonterminal. Result forwarding
is on by default and can be disabled independently of history sharing.

Child conversations start with only their task packet and selected dependency
results. `share_parent_context` explicitly copies completed history; unresolved
parent tool calls are not copied. Tool access narrows the current invocation's
policy, the session baseline, and the registered child's baseline. A task cannot
widen a call-level read-only restriction. A baseline is an enforcement contract,
not a substitute for OS sandboxing.

## Bounds and recovery

`SchedulerConfig` bounds admitted tasks, active child executions, child turns,
and delegation depth. `BudgetLimits` tracks aggregate child tokens, turns, tool
calls and monetary micro-units. Custom executors charge through `ChildControl`
and return cumulative usage; the scheduler avoids double-counting. Token/cost
consumption is observational: provider generation already dispatched cannot be
undone when its usage is reported.

`SchedulerStore::persist` must atomically persist a complete, versioned snapshot
with host-provided fencing/CAS. A failed or cancelled write acknowledgement is
ambiguous and closes admission; do not retry effects on that owner. Committed
snapshot reads may reenter persistence callbacks. No filesystem or distributed
lease policy is invented by this scheduler.

Cancellation has separate live-observation and durable-acknowledgement boundaries.
After a cancellation future is polled, `status`/`list` can show `Cancelled` with
`cancellation requested` and no waiting dependencies while a resume/checkpoint
write is still blocked. This is an intent projection, not a committed result:
`snapshot`, waits, result collection, results-detail tool responses and parent
final joins continue to use committed state. Neither the first nor a repeated
cancel call acknowledges success before terminal persistence. Failed writes keep
the owner poisoned; live cancellation never authorizes replay or early delivery.
Restored steering retains separate in-flight and queued journals, but redelivers
unacknowledged in-flight messages before queued messages. Checkpoint, snapshot
and delivery values are owned copies.

`SchedulerHandle` implements `ChildCheckpointOwner`. A parent `run_durable` with
`RunnerConfig.subagents` binds that owner automatically and requires a durable
scheduler store: occasional parent snapshots alone cannot prove that a child has
not dispatched in the meantime. The store must validate the native scheduler
revision against its latest independently persisted ledger, so an older parent
snapshot cannot overwrite newer child effect evidence. The scheduler snapshot
retains DAG edges, effective security, cumulative usage, first-delivery state,
dispatch evidence and identified queued/in-flight/acknowledged steering.
Restore requires a fresh scheduler, the same session identity and a compatible
security/budget baseline, and never dispatches work by itself.

Nonterminal restored records become `reconciling`. `resume_queued(id)` is only for
work proven never dispatched, and revalidates security before admission.
`resume_checkpoint(id)` requires a native runner continuation at a supported safe
boundary and refuses dispatched/outcome-unknown effects. A durable child runner
checkpoint commits its history and applied steering IDs together, so message
acknowledgements cannot get ahead of recoverable conversation state. Work with
ambiguous dispatched effects requires `reconcile(id, terminal_outcome)`; read-only
does not imply replay-safe. Terminal records are retained without re-execution.
This deliberately does not claim exactly-once external effects.

See [inspected Rust/Go sources and design decisions](research/subagent-orchestration.md)
for ownership tradeoffs and the reference-derived regression checklist. Tests
live in `crates/adk-runtime/tests/subagent*.rs`.

## Durable executor generations

For a durable session that must refresh its child catalog without losing accepted
work, opt into named generations from the beginning:

- `Scheduler::new_with_generations(context, current, historical, store)` receives
  a current `SchedulerGeneration` and any historical reconstruction bindings.
- Each binding contains an immutable, nonempty host-defined `id`, its complete
  `SchedulerConfig`, and an `Arc<dyn ChildExecutor>`.
- `Scheduler::reconfigure_generation(next).await` activates a **new ID**. Accepted
  tasks keep their original executor, security/turn limits and concurrency pool;
  future submissions use the new generation. Graph/depth limits and the shared
  accounting budget cannot change during refresh.

Persist the host's reconstruction recipe **before** activating its ID. An ID
identifies the complete model, instructions, tool catalog, host integration and
executor assembly, not just a role name. The SDK serializes generation identities
and scheduler configuration; it does **not** serialize executable callbacks,
credentials, models, tools or hosts. Treat reconstruction bindings as trusted host
configuration, not instructions to instantiate arbitrary checkpoint contents.

Named refresh persists a fenced checkpoint before exposing its new wiring, even
when no tasks exist yet. Failed or cancelled persistence poisons the owner rather
than retrying an ambiguous activation. A newly constructed owner has no durable
activation until its first persisted operation. The same CAS/lease requirements
as ordinary scheduler persistence apply.

On recovery, construct a fresh owner with the saved active generation and bindings
for **every saved generation**, including generations with only terminal tasks.
Reconstructed configurations must match saved descriptors exactly. Missing or
changed bindings, reused IDs, malformed pool identities and implicit version
migration are rejected. Equal-capacity consecutive refreshes share a concurrency
pool; changing capacity creates a separate pool for the new generation. Recovery
preserves those relationships rather than silently multiplying execution permits.
Nonterminal records still require explicit queued resume, checkpoint resume or
reconciliation; supplying bindings never authorizes replay of ambiguous effects.

The public facade can name its automatically assembled current executor with
`Builder::subagent_generation_id(id)`. A fresh owned recovery session can also
receive `Builder::subagent_historical_generations(bindings)`. Hosts reconstruct
historical runners explicitly (for example with `RunnerChildExecutor::new`) and
must keep their tool/resource owners alive until session shutdown. Rebuilding the
current executor alone does not reconstruct earlier models, instructions or tool
owners. The same session handle is retained across a successful owned refresh;
a failed owned Builder rebuild closes the transferred session rather than
returning a usable old owner.

Unnamed schedulers retain the legacy version-1 recovery contract. They cannot
refresh tracked durable tasks or silently adopt named identities. Named sessions
use version-2 checkpoints; start a new named session when adopting this feature
instead of relabeling existing version-1 records.

Runnable offline recovery examples are integration tests:

```sh
cargo test --locked -p adk --features builder --test catalog_handoffs \
  named_builder_restart_uses_historical_native_binding_without_tool_replay
cargo test --locked -p adk-runtime --test subagent \
  named_activation_committed_before_cancel_recovers_authoritative_generation
```

The facade example includes recovery after the active catalog becomes empty:
no new child tools are advertised, but previously accepted historical work can
still be explicitly recovered. The second example models a store that commits
activation and then loses its acknowledgement; the old owner is poisoned, and a
fresh owner recovers the authoritative committed generation.

## Display estimates versus actual accounting

A completed child's `cost_usd`/`cost_known` follow the SDK's display-estimation
rule: preserve the prices captured for its model responses, then estimate any
remaining token/cache usage once using the child's original model. Requests-only
residuals do not trigger an estimate. This can differ from the cumulative amount
actually charged for compaction under a different model or a nonlinear estimator.
Failed, cancelled and nonterminal outcomes retain cumulative accounting instead.

`RunMetrics.cost_usd` and scheduler budget micro-units remain **actual cumulative
accounting**; the residual display estimate is never charged again. The optional
`RunMetrics.response_cost` freezes the response subtotal and certainty, avoiding
re-estimation of past responses. Native checkpoints retain it. Older/reconciled
checkpoints without that provenance retain their valid cumulative amount but
cannot claim known display pricing merely because a later response is priced.

Custom estimators can override `CostEstimator::estimate_display_cost` to return
an amount with explicit certainty, including a nonzero uncertain estimate. Its
default calls the existing `estimate_cost` once. This extension affects display
projection only, not actual accounting or budget enforcement. Negative,
nonfinite or overflowing display estimates do not enter published totals; valid
available subtotal is retained with unknown certainty. Native context-size
metadata is not included in residual pricing inputs.

See `fixtures/costs/README.md` for the independently generated pinned-Go cases.

## Caller-owned delegation guide

With `runtime`, `adk::specialists::build_delegation_guide(&agent, &specialists)`
formats the SDK task-packet instructions and the supplied specialist/handoff
catalog. Specialists use a `BTreeMap<String, Arc<AgentConfig>>`; map keys are
listed in sorted order, blank keys are omitted, and handoffs retain agent order.
It performs no model/tool execution and does not mutate or automatically inject
instructions. The builder also uses this formatter for its automatic delegation
guide, advertising only policy-permitted handoffs and catalog specialists when
the managed `subagent` task tool is available. Status/control-only configurations
do not advertise task delegation.

The exact reusable text in `crates/adk/src/specialists/delegation.txt` is derived
from GPL-3.0-only SDK `pkg/agentsdk/specialists.go` at the pinned source revision.
Six independently executed Go cases compare complete guide output, including
empty catalogs, Unicode/whitespace labels, description defaults, handoff-only
and combined catalogs. Native strings are UTF-8 and agent handles are nonnull.

## Standalone specialist catalog API

With `runtime`, `adk::specialists` exposes catalog composition independently of
`builder`:

- `SpecialistRoleSpec` and `build_specialists_from_catalog` produce unbound
  `SpecialistAgentSpec` values. Blank names are skipped, but accepted names,
  duplicates, order, instructions, model overrides and fallback strings are
  preserved verbatim. Raw conversion adds no tools, settings or model defaults.
- `prepare_specialist_catalog` accepts `SpecialistCatalogOptions`, trims names,
  keeps the first case-sensitive occurrence, and accepts empty instructions.
  It applies base → catalog → mode-default → mode-role model precedence.
  `routing: None` does not resolve aliases; `Some(Default::default())` does.
  Only OpenAI (including an empty provider label) and Anthropic resolve size
  aliases. Empty OpenAI models remain empty; other provider IDs stay opaque.
- Nonempty catalog fallbacks replace the base verbatim. Nonempty resolved mode
  fallbacks replace those. Empty lists do not clear fallbacks; an all-blank role
  routing list suppresses mode fallbacks but retains catalog/base fallbacks.
  Routing settings merge over base settings. Role reasoning `none` clears the
  *mode* budget, not a nonzero base budget. Access adaptation precedes signal
  stripping; tools retain shared `Arc` identity. This is selection, not authority.
- `bind_specialist_catalog` takes the unmodified **prepared** output and a
  caller-supplied `FnMut(&str) -> Result<ModelBinding, Error>`. Bind exactly those
  names, including raw fallback strings; do not add resolver defaults here.
  The returned `SpecialistCatalog::agents()` exposes registration snapshots.
- Register runners built from those exact snapshots with `RunnerChildExecutor`,
  using matching names in the caller-owned `SchedulerConfig::agents`. Then call
  `catalog.attach_tools(Arc<SubagentSession>)`. The caller supplies host, model
  implementations, runner configuration, scheduler security and lifecycle, and
  must await `Scheduler::shutdown()`. Optional output extractors belong on
  `RunnerChildExecutor::with_output_extractor`; empty extraction falls back to
  final text. No global runtime or implicit registration is created.
- `build_specialist_tools_from_catalog` is a convenience for an already
  registered session. `None` returns empty agents/tools **before** any adapters
  or binding callbacks. The staged API is preferable when registering a new
  session. Registration and attachment must use the same immutable snapshots;
  changing a source spec requires rebinding and re-registration.

Named wrappers retain catalog order and SDK name sanitization, including
collisions. Description is copied exactly, approval defaults to false, and the
wrapper is read-only when all selected tools are read-only (including no tools).
Catalog assembly does not reject collisions, install guardrails, set parallel
execution settings, or impose the builder's authorization ceilings.

**Bounded support:** prepared-agent fields and wrapper metadata are compared
against 54 independently executed pinned-Go cases in
`fixtures/specialist-catalog/observations.json`. Native integration tests cover
caller-owned registration, successful execution, custom/empty extraction and
shutdown. This is not full Go wrapper/runtime parity: native wrappers use the
existing child engine, schema and error/timeout delivery, add native child
instructions, and dispatch by registered name rather than retaining Go live
agent pointers. Recursive Go wrapper pointer-graph mutation is not represented.
Native settings remain a JSON map, not a new Go `ModelSettings` codec; Go
nil-versus-empty slice identity is not represented.

### Mode-template routing bridge

With the `builder` feature, `specialists::mode_routing_from_template_spec` copies
SDK routing labels from an optional `builder::ModeSpec` into standalone
`SpecialistModeRouting`. Missing templates or routing return `None`; present
empty routing remains `Some`. Model, fallback, reasoning, verbosity and role-key
strings are copied verbatim into owned containers, without normalization,
validation, provider binding or session registration. Native arbitrary `settings`
maps are deliberately not part of this SDK-label adapter; use the builder's
routing-resolution path when those native extensions are needed.

Six actual-Go conversion observations and native copy-isolation tests supplement
the 54 catalog cases. The fixture retains Go nulls, but the native comparator
projects nil containers to empty owned containers as documented above. This is
not a mode-template wire codec or full `ModeModelRouting` serialization claim.

### Compatibility mode DTOs and conversion

With `compat`, `adk::codec::dto::{ModeTemplateSpec, ModeConstraints, ModeRouting,
ModeRoleRouting}` preserve the pinned Go data shapes through sealed
`from_go_json` / `to_go_json_bytes`. Unlike native builder types, optional
containers retain nil versus allocated-empty identity. Integer constraints use
the pinned 64-bit Go range; decoding does not enforce runtime constraints.

`adk::codec::mode::mode_routing_from_template_spec` provides the exact pure
SDK conversion, separate from the native bridge above. It copies raw labels
and role keys, returns `None` for absent templates/routing, and converts empty
fallback slices and role maps to nil as the Go helper does. Returned data is
independently owned. Six actual-Go cases compare exact bytes and source-mutation
isolation without normalizing nil/empty containers; 993 independent codec cases
cover the data boundary. Neither API binds models or creates runtime sessions.

`adk::codec::mode::{resolve_mode_routing, resolve_role_mode_routing}` then resolve
provider aliases, ordered fallbacks, reasoning and verbosity without creating
executable models. Role lookup is case- and whitespace-sensitive. An explicit
`none` reasoning override clears the inherited thinking budget; a nonempty
all-blank role fallback list clears inherited fallbacks, while an empty list
inherits them. `ModeRoutingResolution` supports `to_go_json_bytes`, but is not a
Go decoding target. The independent oracle compares 1,764 results and exact
bytes; the standalone consumer exercises decode → conversion → role resolution
→ encoding. Provider request clamping and execution policy remain separate.

### Compatibility role/catalog DTOs

`adk_codec::dto::{RoleSpec, RoleCatalog}` are compatibility-only data shapes,
not the native `adk::specialists::SpecialistRoleSpec` or an executable catalog.
Use the sealed `from_go_json` and `to_go_json_bytes` entry points for Go byte
compatibility; ordinary Serde is not that contract, notably for invalid UTF-8
and lone surrogates. `None` retains nil fallback/catalog slices while
`Some(vec![])` retains allocated empty slices. Names, access labels, model
labels and fallback strings remain unnormalized; duplicate roles are retained.
There is no implicit conversion into native specialist construction or execution.

The pinned Go oracle in `scripts/role-catalog-reference/` records 364 cases in
`fixtures/role-catalog/observations.json`; `crates/adk-codec/tests/roles.rs`
compares accepted decoded fields and exact marshaled bytes, and rejection for
invalid inputs. It does not claim Go partial-error mutation, identical error
text, arbitrary invalid-UTF8 in-memory Go strings, an Agent codec, or native
wrapper/runtime parity. The native catalog's nil/empty and execution limits
above are unchanged by these separate compatibility DTOs.
