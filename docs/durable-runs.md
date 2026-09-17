# Durable runs and project state

Issue #9 adds persisted state, not a production worker or a database redesign.
All formats target SDK v0.0.115 (`1dc92b73900fac74dc357a938e4b5eee6392b418`).
The platform migration remains unchanged; no production data was migrated.

## Entry points

| Capability | Public API | Detail |
|---|---|---|
| Run documents/stores | `adk::durable` (`durable` feature) | [Store operations, privacy and lease ownership](../crates/adk-durable/README.md) |
| Runner recovery | `adk::runtime::{DurableRun, StoredCheckpointStore, RunnerCheckpoint, GoRecovery}` (`runtime` feature) | [Execution/recovery contract](../crates/adk-runtime/DURABLE_API.md) |
| Tasks/memories | `adk::project_state` (`project-state` feature) | [Project state and recall](project-state.md) |
| Research | Rust ADK and maintained crate evaluation | [Verified links, licenses and decisions](durable-research.md) |

The facade's default remains contract-only. Enabling `runtime` includes the
filesystem-capable durable primitives needed by its adapter; `durable` additionally
enables the PostgreSQL backend. No feature introduces platform/Kubernetes code.

Create a run snapshot, acquire an expiring lease, open `StoredCheckpointStore`, and
pass it to `Runner::run_durable`. To recover, stop the prior owner, acquire a new
lease, read `checkpoint()` and use a new attempt ID with the same run ID and no
additional input. The host owns lease renewal/release and registration of matching
agents/tools. The current adapter uses synchronous store calls inline to avoid
unowned tasks surviving cancellation; **dedicate an executor thread to these
runs** rather than blocking a shared async executor. Cost estimators used with this
adapter report currency units, converted to micro-units in the persisted budget.

## Safety contract

- Typed IDs, expected revisions, ordered immutable events and expiring fencing
  tokens prevent stale *store* writers. PostgreSQL uses the database clock and
  row locks; filesystem stores use cross-process advisory locks and fsynced
  atomic replacement. Tenant/run keys are checked on reads and writes.
- The runner persists prepared intent, dispatched state and completed state.
  Failure to persist prevents the next operation. Unknown schemas and unknown
  effect outcomes fail closed; cumulative budgets and the original deadline
  survive a restart.
- A dispatched non-replayable effect is **not automatically retried**. Inspect
  the destination and explicitly reconcile it. Idempotency keys are stable,
  but only a destination that honors a key can deduplicate work. A lease cannot
  undo or prevent an already-started remote effect. There is no exactly-once
  external-effect or host-event-delivery guarantee.
- Redaction/encryption are caller-owned fallible hooks. An encryptor-configured
  run store rejects plaintext downgrade; filesystem encryption covers lease and
  document together. Redaction that changes an executable runner continuation
  stops dispatch rather than acknowledging a checkpoint that cannot be resumed.
- Retention/deletion do not erase backups, database WAL, filesystem snapshots or
  remote embedding-provider copies; those remain host responsibilities.

## Persisted compatibility and limits

Run documents use schema **2**, including explicit schema-1 migration. Runner
checkpoints independently use schema **1** plus a versioned Rust continuation.
Project-state logs independently use schema **1**. These version namespaces are
not interchangeable.

Actual Go-generated filesystem records, encrypted envelopes, document migrations,
SQLite databases, embeddings and runner histories are checked in. Optional Go
reader tests continue Rust-written stores, then Rust reads the Go continuation.
PostgreSQL tests apply an unmodified copy of platform migration 042 and use only
uniquely named disposable test schemas.

Go's runner does not reject every unknown boundary name. Nonterminal Rust runner
envelopes therefore use the Go-rejected `model_completed` marker, retaining the
actual boundary in the Rust extension. They remain inspectable by Go without
silently authorizing unsafe replay. Explicit migration supports safe Go start/completed-tool/handoff/pause/child-change/terminal boundaries with host-verified missing policy/budget metadata and complete history. Ambiguous Go prepared checkpoints are not evidence that dispatch never occurred.

Durable streaming, Go observational callbacks, native approval journals/paused
continuations and host-owned child checkpoint restoration are supported. See the
[exact pinned-baseline supported/rejected matrix](durable-recovery-matrix.md) for
all boundaries, conditions, tests, and remaining function-valued native extensions
without a crash-replay protocol. Filesystem durable-run privacy is Unix-only and
assumes trusted private ancestors and a local filesystem with locking/rename/fsync
semantics; use PostgreSQL elsewhere. Project-state lock compatibility is not a
claim of safe simultaneous stale-lock recovery by unmodified Go and Rust workers.

## Verification

```sh
cargo test --locked --workspace --all-features --all-targets
cargo test --locked -p adk-durable --no-default-features
cargo test --locked --workspace --all-features --doc
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
cargo deny --locked check
python3 scripts/check-purity.py

# Actual Go reader/writer and real PostgreSQL; requires the pinned repos/sdk.
ADK_TEST_GO=1 ADK_TEST_POSTGRES_URL='host=127.0.0.1 port=5432 user=postgres dbname=postgres sslmode=disable' \
    cargo test --locked -p adk-durable --all-features --all-targets -- --nocapture
```

CI has a PostgreSQL 17 service job so database semantics are not only mocked.
The default offline suites consume checked-in fixtures; Go regeneration/reader
verification requires the pinned SDK checkout and a Go module cache/network.
See [durable fixture provenance](../fixtures/durable/README.md) and
[project-state provenance](../fixtures/project-state/manifest.json).

The 15 baseline state-backed tool adapters are implemented and tested on both
filesystem and SQLite stores, including differential comparison with actual
Go-generated tool definitions and outputs. #7's unrelated/full registry remains
a downstream consumer, not a blocker for these contracts.
