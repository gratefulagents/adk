# Runtime durable execution

## API

`Runner::run_durable(context, request, host, DurableRun)` is distinct from the legacy observational `DurableHook` API. `DurableRun::new(Arc<dyn CheckpointStore>)` creates an attempt ID; set `resume` to a decoded checkpoint for recovery. The context must carry the original stable run ID, the attempt ID must be new, and recovery input must be empty.

`CheckpointStore::persist(context, &RunnerCheckpoint)` returns an awaited future. A successful acknowledgement means the entire checkpoint is durable. Implementations must fence writers and CAS the prior revision. Persistence failures stop execution; an ambiguous store error is never permission to dispatch.

`StoredCheckpointStore::open(Arc<dyn adk_durable::RunStore>, lease)` joins the sibling durable store API. The caller creates the run/acquires the lease; `checkpoint()` supplies the current continuation. Each write atomically appends an event, updates snapshot state and cumulative budgets, and merges the effect into the effect ledger. Identity/sequence/revision mismatches and decreasing counters fail closed. Any failed persistence permanently closes that adapter. Lease lifecycle remains host-owned. This synchronous store adapter does I/O inline, without detached blocking tasks: dedicate an executor thread to these runs. For a renewed lease, open a new adapter after stopping the prior owner.

## Boundaries and recovery

Model and tool effects have prepared/dispatched/completed records. All effects are conservatively classified non-replayable. Prepared native checkpoints may resume, preserving the effect ID, idempotency key and step ID. Dispatched or outcome-unknown checkpoints require `operator_resolution`; model/provider errors and tool errors/timeouts are never automatically retried in durable mode. Read-only tools are not presumed idempotent. Tools receive the stable key through `ToolContext.idempotency_key`.

ModelCompleted includes the accepted response and exact next phase. ToolCompleted includes remaining queued calls, so completed siblings are never replayed. Handoff checkpoints identify the next registered agent. Terminal recovery returns the saved result without dispatch or another store write. Run usage, model attempts, tool dispatch count, monetary cost, original wall-clock start and absolute deadline survive process restarts. Recovery rejects agent/configuration or policy changes; function implementation identity remains the host's responsibility.

Durable tool batches execute serially. Custom compaction, dynamic turn context, stop gates, custom parsers and non-opted-in observation hooks are rejected because they have no crash-replay protocol. Durable output truncation does not create ephemeral spill-file references. Event delivery is observational, not exactly-once.

## Go compatibility

`RunnerCheckpoint::decode` accepts the baseline Go schema-1 envelope, including null history, and rejects future versions. `Runner::migrate_go_checkpoint(checkpoint, GoRecovery)` explicitly migrates **run_started**, **tool_completed**, **handoff_completed**, **paused**, **child_changed** and **run_completed** checkpoints. The host supplies verified original policy, cumulative usage/attempt/tool/cost counters, original start/deadline and (for terminal checkpoints) verified final output. Counters cannot decrease Go Usage counters; Usage.Requests is a lower bound for attempts. Histories must have complete call/result pairs. Message, tool-call/output, reasoning and compaction snapshots are translated through the existing codec.

Go prepared checkpoints **do not prove undispatched** and are refused. Raw Go model-completed/tool-prepared/approval-pending checkpoints and unresolved histories require reconciliation, matching the baseline's explicit refusal gates. Complete approval history markers are retained without granting authority. Native approval journals and tool-paused recovery are supported; deferred approvals return an owned continuation requiring an explicit decision. Go approval-gate resumption retains cumulative durable budgets.

`Runner::stream_durable` is a lazy owned pull stream using the same persistence protocol and genuine `StreamingModel` provider capability. Dropping it after dispatch leaves an unknown effect; recovery never retries it. `GoCallbackAdapter` and explicitly opted-in `RunHooks::durable_observer()` callbacks are observational, not exactly-once effect delivery.

`DurableRun::children` optionally supplies a `ChildCheckpointOwner`. Active restored records are converted to reconciling and passed to that owner before parent execution; they are never relaunched automatically. Terminal-only records can be retained without an owner. All subsequent parent checkpoints carry child snapshots; any snapshot/restore failure stops the parent. Owner semantics, all baseline boundaries, and explicit unsupported states are listed in the [recovery matrix](../../docs/durable-recovery-matrix.md).

Rust writes retain Go-readable schema-1 envelopes. Critically, the baseline Go runner accepts unknown boundary strings: consequently every **nonterminal Rust envelope uses `boundary=model_completed`**, which the baseline explicitly rejects. The exact Rust boundary is inside the versioned runtime extension and exposed by `execution_boundary()`. Terminal envelopes use `run_completed`. This prevents a baseline reader from blindly replaying `*_dispatched` or partially completed tool batches.

## Verification

`tests/durable.rs` covers fault injection before/after committing prepared, dispatched and completed records; unknown outcomes; stable keys/steps; cumulative budget/deadline exhaustion; partial batches; terminal replay prevention; cancellation during persistence; approval callback ordering; unknown versions/security policy changes; and the real fenced filesystem RunStore adapter.

`tests/fixtures/checkpoint.go` generates `go-checkpoint.json` using the baseline SDK's public DurableCheckpoint aliases. Run from `repos/sdk` with `go run ../../crates/adk-runtime/tests/fixtures/checkpoint.go`. No production data migrations or exactly-once external-effect claims are made.
