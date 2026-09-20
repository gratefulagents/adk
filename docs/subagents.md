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
