# Standalone host sessions

Enable `adk`'s `host` feature for caller-driven orchestration. It includes
`builder` and `observability`, but no platform client, service discovery,
background polling, worker command, or evaluation adapter. Application code
owns the stores, credentials, input source, and schedule.

Run the offline embedding example:

```sh
cargo run --locked -p adk --features host --example host_session
```

The example consumes a host message, calls an injected model, verifies the
persisted batch and cursor, and explicitly closes the owned session. It uses no
network, credentials, subprocess, or platform service.

## Composition and ownership

Construct `host::ChatLoopOptions::new(agent)` with a native `AgentConfig`.
Supply optional `SessionStore`, `ConfigSource`, `PlatformToolFactory`,
`ApprovalGate`, `TraceStore`, `RunStatusSink`, and core `Host` implementations.
`RunnerConfig` and `RunPolicy` remain explicit native values; custom runtime
hooks are composed rather than replaced. Tool-factory results replace the
agent's tool list, so return any base tools you want to retain.

`ChatLoop::run` performs one invocation, not an infinite conversation loop.
The default session is owned. `Session::Borrowed(handle)` binds an existing
owner without closing it. Call `close().await` to drain an owned session;
closing a borrowed loop does not close its owner. Caller cancellation and the
session cancellation signal are both respected, and caller deadlines remain
in force during collaborator calls.

The loop does not create a store session, implement a transaction, or persist
its cursor automatically. `cursor()` exposes the signed message ID **and**
opaque token. Save both if the application needs restartable ingestion.
The cursor advances as each page is loaded, before model execution. A later
page/model/store failure therefore does not roll back already consumed pages.
The default page size is 50; a short page or unchanged cursor ends pagination.

Each call reloads handoff history from `ConfigSource`; previous call results
are not silently retained as the next call's prompt. A store must supply its
own replay history through that collaborator. `RunBatch` keeps native items,
authorship provenance and approval-boundary sidecars together. Missing
provenance means unknown authorship, never the active agent.

## Ordering and failure boundaries

Preparation loads permissions, instructions, guardrail rules and mode snapshot,
then working state, then invokes the tool factory. Handoff history and paged
messages are loaded afterward. `role_catalog` is available to application/tool
assembly but is not called by the loop.

Successful runner batches are persisted, including an empty batch. Approval
resolution is sequential; resolved items and markers are persisted **before**
another model call. If a later gate fails, already resolved records are still
persisted. Persistence failure takes precedence over the earlier resolution
failure. The default maximum is 12 approval resumes, checked after persisting
the interrupted runner batch. An approved tool's pause resolves the entire
pending approval batch without making another model call.

Without a gate, the loop synthesizes denied approval/error-output pairs,
persists them and finalizes without executing tools. The result retains the
source SDK's interrupted state and prior final history; no executable
continuation is returned for those denied calls.

The awaited approval-persistence boundary is not a durable recovery protocol.
Native durable continuations are rejected before approval resolution when this
boundary is supplied: a checkpoint alone cannot prove that a separate host
store append committed. Durable reconciliation of that external append remains
an explicit integration limit, not an exactly-once guarantee.

Model failures discard the returned partial result, matching the pinned host
contract; this does not undo effects or records already persisted. Append,
approval and finalization failures retain their combined partial result. Trace
finalization precedes final status publication. Optional progress, trace-ID,
category and file methods are application hooks, not promises that the loop
calls them automatically. Use the separate tracing integration for runtime
spans and provider snapshots.

## Limits and compaction

A mode can supply missing main/child turn limits and narrow tool access.
Child limits are attached to each invocation's tool policy, composed by minimum
with existing restrictions, and bounded again by the scheduler owner. Reusing
a scheduler does not mutate a session-global turn limit.

Working state is carried forward after compaction, not added automatically to
the initial prompt. Nonblank dynamic `CompactionCarryForward` context supersedes
static `working_state_context`; blank dynamic context falls back to static.
Existing carry-forward messages are replaced, preserving provenance and
approval boundaries. Input guardrails run before the new context is inserted.
Durable runs require a nonempty stable callback key; changed host settings
participate in the durable configuration fingerprint.

## Compatibility and validation limits

Reference: SDK v0.0.115, commit
`1dc92b73900fac74dc357a938e4b5eee6392b418`,
[`pkg/agentsdk/chatloop.go`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/chatloop.go).
`fixtures/host-loop/sdk-chatloop.json` contains 46 independently executed Go
scenarios. Regenerate and check them with `python3 scripts/host-reference/check.py`
after obtaining the clean pinned SDK and a Go toolchain. Source fixture coverage
is not a claim that every scenario or SDK capability is verified in Rust.

Dynamic guardrail regular expressions use Rust's regex engine; unsupported
syntax is rejected explicitly. Native tool arguments are structured JSON, so
raw Go input-byte spelling is not preserved in regex checks. Store durability,
transactionality, provider credentials, and live services remain application
responsibilities. The full SDK capability ledger remains authoritative for
unresolved parity; this feature is not blanket ledger closure.
