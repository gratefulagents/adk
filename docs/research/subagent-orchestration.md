# Subagent orchestration: inspected sources and Rust decisions

Research for issue #10. These are source-inspection pins, not proposed dependency
upgrades. No external ADK implementation is copied or added as a dependency.

## Sources inspected

| Source/version | Inspected implementation | Relevant evidence |
| --- | --- | --- |
| gratefulagents/sdk **v0.0.115**, commit `1dc92b73900fac74dc357a938e4b5eee6392b418` (verified local checkout) | `internal/agent/subagent_run.go`, `subagent_registry.go`, `subagent_durable_test.go`, `runner_durable_children_test.go`; `pkg/agentsdk/subagent_tools.go`; `pkg/agentsdk/runtime/builder.go` (`SessionState`) | Shared child engine; DAG/dependency semantics; fail-closed persistence; interrupted-child reconciliation; session rather than turn ownership |
| zavora-ai **ADK-Rust 2.2.0** (`adk-agent`; not Google ADK) | [ParallelAgent source](https://docs.rs/adk-agent/2.2.0/src/adk_agent/workflow/parallel_agent.rs.html#192-281) | Concurrently polled owned branch streams; parent context/history propagation; consumer-driven lifetime |
| zavora-ai **ADK-Rust 2.2.0** (`adk-graph`) | [Pregel executor source](https://docs.rs/adk-graph/2.2.0/src/adk_graph/executor.rs.html#909-1036) | Bounded ready-node dispatch and whole-superstep barriers; special nested concurrency handling |
| **Tokio 1.53.1** (workspace lockfile) | [JoinSet](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinSet.html), [JoinHandle](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinHandle.html) | A dropped join handle detaches; an owned set aborts on drop; cancellation must be followed by joining for confirmed teardown |
| **tokio-util 0.7.19** (workspace lockfile) | [CancellationToken](https://docs.rs/tokio-util/0.7.19/tokio_util/sync/struct.CancellationToken.html) | Latched cancellation, parent-to-child propagation without reverse cancellation authority |

Go permalinks use the commit above, for example the
[durable regressions](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/subagent_durable_test.go)
and [session ownership](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/runtime/builder.go).
The pre-change local runtime suite was executed on repository-pinned Rust 1.88.0.
The downloaded, lockfile-exact crate sources were also inspected directly:
Tokio `src/task/join_set.rs:373–385,591–595` implements shutdown as abort plus
join-draining, while Drop only aborts; tokio-util
`src/sync/cancellation_token.rs:204–222` derives child tree nodes and cancels the
subtree. This distinction is why explicit scheduler shutdown joins its children
rather than merely aborting the actor that owns their set.

## Ownership decisions

- **Adopt Tokio structured task ownership**, not a literal goroutine translation.
  An exclusive session owner retains the scheduler actor and its owned child
  task set. Cloneable tool handles are capabilities, not owners that detach work.
  Explicit async shutdown cancels and joins. Drop provides abort-on-drop fallback,
  not proof that asynchronous cleanup or persistence finished.
- **Adapt ParallelAgent's owned futures and backpressure**, but reject its stream
  lifetime as the managed-task lifetime. A tool timeout or parent turn ending must
  not drop background children; the persistent session closes them explicitly.
  Reject automatic parent-history sharing: isolated child conversations are the
  default, with an explicit copy of completed history only when requested.
- **Adapt stable ready ordering and bounded dispatch from graph executors**.
  Reject whole-superstep barriers: unrelated task completions must become visible
  incrementally. Dependency waiting is not executing and must not occupy an
  execution permit. `all_success` differs from `all_terminal`; reconciling is not
  a terminal dependency outcome.
- **Reject nested concurrency bypass**. Pregel's nested-call exemption avoids a
  permit deadlock, but is incompatible with a shared hard limit. Nested delegation
  must use an inherited, bounded scope and cannot acquire a fresh unrestricted
  scheduler. In particular, a concurrency-one synchronous nested call must not
  silently deadlock or exceed the cap.
- **Use broadcast state-change observation**, not polling sleeps or a
  single-consumer completion channel. Subscribe before reading state and recheck
  on every wake. Wait-any is about an undelivered terminal result, not arbitrary
  activity; an all-terminal watched set also returns immediately even when its
  results were already delivered. Status result rereads must not erase stored results.
- **Reject `JoinSet::join_all` as an error policy**: it panics on a join error.
  Drain explicitly and record individual child outcomes. Neither cancellation nor
  abort can stop non-yielding code or already-running `spawn_blocking` work;
  executors must own subprocesses and must not detach asynchronous work.

## Persistence and security decisions

1. Persist submission intent before dispatch. A failed store acknowledgement must
   launch nothing and must not expose provisional records to checkpoint readers.
   Snapshot callbacks may reenter committed-state reads.
2. Do not hold a state mutex while awaiting external persistence. Cancellation
   needs an independent latched path: it must win even while a resume checkpoint
   write is blocked. Check again immediately before actual dispatch.
3. A saved task name or generic checkpoint does not establish effect replay
   safety. Restored active tasks become reconciling. Only evidence of a never-
   dispatched/safe continuation allows automatic replay; unknown effects need an
   explicit operator outcome. Unsupported schemas/security weakening are resume
   rejection, not evidence that an effect should be retried.
4. Preserve stable steering identities, queued/in-flight/applied state and order.
   In-flight messages precede later queued messages on recovery. Exactly-once
   external effects cannot be inferred from queue acknowledgement; child history
   and its applied frontier must establish whether application is replay-safe.
5. Security narrowing is an intersection, not a replacement: access mode,
   allow/deny lists, approval, guardrails, output treatment and limits all matter.
   The current tool-call read-only clamp applies even if the session was created
   with write access. Child depth and shared budgets cannot be reset by delegation.
6. Terminal results are committed before notification. First delivery is tracked
   separately from retained results, so incremental parent injection and explicit
   reread coexist. Final parent acceptance must be blocked while children remain
   nonterminal; this is not merely a prompt recommendation.
7. Session closure is explicit. Go's `SessionState.Close` cancels active children
   and flushes scheduler state; Rust additionally joins owned tasks. A cloneable
   scheduler handle surviving a parent turn must not become a detached owner.

## Reference-derived verification targets

The Go durable tests explicitly cover failed submission persistence and hidden
provisional snapshots, reentrant checkpoint reads, active-child reconciliation,
terminal-child recovery without dispatch, in-flight-before-queued steering,
rollback on failed reconciliation writes, cancellation during blocked resume
persistence, and weaker-security/unknown-schema refusal. Rust tests should cover
these invariants alongside DAG cycles/unknown dependencies, dependency failure
and cancellation, shared concurrency, event-driven broadcast wakeups, incremental
result delivery, isolation and explicit shutdown.

The Rust implementation remains a local session scheduler. This research does
not justify a distributed queue, worker lease redesign, rollout change, or an
exactly-once claim for external model/tool effects.
