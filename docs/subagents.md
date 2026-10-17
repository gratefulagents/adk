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
