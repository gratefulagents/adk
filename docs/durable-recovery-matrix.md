# Durable recovery compatibility matrix

Baseline: SDK **v0.0.115**, commit
[`1dc92b73900fac74dc357a938e4b5eee6392b418`](https://github.com/gratefulagents/sdk/tree/1dc92b73900fac74dc357a938e4b5eee6392b418).
This table distinguishes what the baseline code *accepts* from what its persisted
state can safely prove. Acceptance by the old runner is not evidence that an
external effect did not happen.

## Go schema-1 checkpoint migration

All supported nonterminal migrations require the original policy, cumulative
usage/attempt/tool/cost counters, original start/deadline, registered agent, and
complete call/result pairs. `GoRecovery` makes the missing metadata explicit;
counters cannot decrease recorded Go usage. Terminal migration also requires the
verified final output (Go's checkpoint does not store the parsed result).

| Go boundary/state | Pinned Go behavior | Rust migration/recovery |
|---|---|---|
| `run_started` | Restore history and agent; continue | Supported with verified metadata |
| `tool_completed` | Restore; continue | Supported **only with complete call/result pairs**. The Go approval path also emits this boundary with an unresolved call; that form is rejected, not replayed |
| `handoff_completed` | Resume registered target agent | Supported; actual Go handoff history contains ordinary paired tool calls/outputs |
| `paused` after a pause tool | Continue from complete history | Supported; completed tool is not dispatched again |
| `child_changed` | Falls through to continuation | Supported with complete parent history and child-owner rules below |
| `run_completed` | Return last text without model call | Supported; return verified terminal result without another dispatch/write |
| `model_prepared` | Falls through to another model call | Rejected: the old checkpoint has no dispatched marker; a crash may have happened after dispatch. No automatic non-replayable retry |
| `model_completed` | Explicit reconciliation error | Rejected, matching baseline; raw Go does not persist Rust's exact next phase |
| `tool_prepared` | Explicit reconciliation error | Rejected, matching baseline |
| `approval_pending` | Explicit reconciliation error | Rejected for **raw Go** migration, matching baseline; native approval recovery is supported below |
| `run_cancelled` | Falls through if supplied a new active context | Rejected: cancellation does not prove an in-flight effect did not occur |
| Unknown boundary/schema | Go checks schema, but unknown boundary strings fall through | Rejected. Every nonterminal Rust envelope deliberately uses Go-rejected `model_completed`; actual execution boundary is in the Rust extension |
| Approval history markers at a safe boundary | Restored by Go | Retained in Rust journal and Go-compatible history; false `approved` is preserved as pending rather than inventing a denial. Markers never grant execution authority; incomplete call/result history remains rejected |
| Explicit legacy `handoff_call` / `handoff_output` history records | Go restores these payloads | Rejected for executable migration: no call ID is present. These are not emitted by the pinned runner's normal handoff path. Store codecs still preserve them |
| Nonempty interruption payload / effect field in a raw Go import | Raw Go has no Rust effect ledger | Requires explicit reconciliation, not automatic migration |

Source evidence:
- [`runner.go:471–509`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/runner.go#L471-L509): schema checks, terminal return, three explicit refused boundaries.
- [`runner.go:1504–1553`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/runner.go#L1504-L1553): handoff outputs and target checkpoint.
- [`runner.go:1564–1720`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/runner.go#L1564-L1720): tools, approvals, terminal and pause boundaries.
- `internal/agent/durable_checkpoint_test.go`, `durable_forced_crash_test.go`, and `durable_review_regression_test.go`: baseline failure/recovery tests.

`crates/adk-runtime/tests/fixtures/boundaries.go` runs the actual pinned Go runner
with local scripted models/tools and exports `go-boundaries.json` (tool, pause,
handoff and approval paths). Only step IDs and timestamps are normalized; run and
attempt IDs are explicit fixture inputs. Rust tests execute migrations from these
emitted checkpoints, including refusal of approval-path `tool_completed` with an
unresolved call. No external provider is contacted. Fixtures/generators are
SDK-derived GPL-3.0-only; see `fixtures/licenses/SDK-GPL-3.0.txt`.

## Native execution and recovery

| Mode | Status and ownership |
|---|---|
| Completion and genuine provider streaming | Supported via `run_durable` / lazy `stream_durable`, same persisted prepared/dispatched/completed protocol. Drop during streaming leaves dispatch outcome unknown; recovery refuses replay. Complete requires clean provider EOF |
| Native model/tool prepared recovery | Supported with stable step/effect/idempotency key. An exact-call approval grant is restored from the validated journal under the unchanged policy, without another approval callback. Unlike raw Go, native checkpoints prove no acknowledged dispatched transition |
| Native dispatched/outcome-unknown recovery | Refused until explicit destination/operator reconciliation. No retry, fallback or exactly-once claim |
| Model/tool completion and handoff | Supported, including remaining serial siblings and target identity. Go envelope projects native handoff results to the baseline paired `Handing off to …` tool output; runtime extension retains native semantics |
| Tool pause | Supported from the saved next phase without repeating completed tool work |
| Deferred approval pause | Supported: restore pending call and journal, return owned continuation without calling approval again. Caller supplies a decision; merely reopening never approves. A pre-callback approval-intent checkpoint with no saved decision asks the host again |
| Approved/denied journal after completion | Supported with validated anchors and retained Go history markers. Completed effects are not reexecuted |
| Go approval-gate adapter on native continuation | Supported. Durable cumulative turn budget is **not** reset by the legacy per-invocation helper |
| Go lifecycle callbacks | Supported through `GoCallbackAdapter`. Delivery is observational and can be missing/repeated across crash; callbacks must not be used as an exactly-once effect boundary |
| Native observational hooks | Explicit `RunHooks::durable_observer()` opt-in. Default false; opting in asserts replay-tolerant observation, not permission to perform non-replayable effects |
| Local compaction | Supported by existing deterministic engine and approval-journal anchoring |
| Custom compactor, dynamic turn context, stop gate, output parser, legacy durable hook, or non-opted-in hook | Rejected before dispatch. These function-valued native extensions are not represented in the pinned Go checkpoint contract and have no durable effect protocol. Go observers are no longer rejected with this group |
| Old native checkpoint declaring a journal but omitting its entries | Nonterminal recovery rejected: cannot reconstruct lost approval provenance. New checkpoints persist entries; terminal results remain readable |

## Children

The pinned runner's `wireDurableChildren` restores into a scheduler, rejects active
children without one, and permits terminal-only children without one.
[`subagent_registry.go:1433–1490`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/subagent_registry.go#L1433-L1490)
changes pending/waiting/running children to **reconciling**, not restarted work
(the nearby Go comment incorrectly describes failed tombstones).

Rust now follows that ownership boundary through `ChildCheckpointOwner`:

- No owner: absent/null/empty or terminal-only records are accepted and retained;
  active records are rejected before parent dispatch.
- With owner: restore validated unique child IDs, preserving delivery flags,
  security baseline, child checkpoints, and steering payloads. Active statuses
  become `reconciling`; stale waiting-on state is removed. The owner must preserve
  and manage these records without dispatching during restore, including Go's
  in-flight/queued steering semantics. Snapshot/restore errors stop the parent.
- Unknown statuses and malformed/duplicate records fail closed. An owner must
  restore only into its own fresh registry or explicitly validate an already
  restored registry; it must never overwrite live work blindly.
- Subsequent parent boundaries include the owner's current snapshot. The owner,
  not the parent runner, performs explicit durable child-worker/operator
  reconciliation. This is the #4 integration interface, not a new scheduler or
  an automatic retry mechanism.

The lower-level `StoredCheckpointStore` continues to reject a separate stored
cancellation/child-run ledger needing reconciliation; attaching a scheduler does
not erase store-level unresolved state.

## State-backed tools

`adk_project_state::tools::tools(Arc<dyn Store>, actor)` exposes exactly the 15
pinned project-state tools, using the existing `adk_core::Tool` contract. It does
not add unrelated #7 registry families. Both filesystem and SQLite tests invoke
the adapters, verify mutation/read-only metadata, actor and priority defaults,
links/readiness, omitted-vs-empty updates, recall, stats, deletion, model-visible
errors, priming and reopened-store persistence.

`fixtures/project-state/tools.go` executes the actual Go tools and emits
`tools.json`. Rust compares all definitions and the task/memory operation trace
against those Go outputs on both stores (normalizing generated IDs/timestamps
only). Priming is tested through the adapter and separately against the existing
exact Go `prime.txt` fixture. Thus #7's full registry is not a reverse dependency
for state-tool contract verification.
