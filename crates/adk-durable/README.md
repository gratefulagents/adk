# adk-durable

Runtime-neutral durable execution primitives compatible with SDK Go `pkg/agentsdk/durable` schema v2. This crate does not execute tools or promise exactly-once effects. See [API.md](API.md) for the runner integration contract.

```rust,no_run
use adk_durable::*;
use chrono::Utc;
use std::time::Duration;

let store = FilesystemStore::new("private-runs", StoreOptions::default())?;
let tenant = TenantId::new();
let run = RunId::new();
store.create(RunSnapshot::new(tenant.clone(), run.clone(), Utc::now()))?;
let lease = store.acquire_lease(&tenant, &run, "worker-1", Duration::from_secs(60))?;
let (mut snapshot, _) = store.load(&tenant, &run)?;
let expected = snapshot.revision;
snapshot.revision += 1;
snapshot.status = RunStatus::Running;
store.append(&lease, expected, vec![Event {
    event_type: "model.prepared".into(),
    ..Event::default()
}], snapshot)?;
store.release_lease(&lease)?;
# Ok::<(), adk_durable::Error>(())
```

## Persistence and safety

- Object-safe synchronous `RunStore: Send + Sync`; async applications should execute store calls in blocking tasks. `create` starts history, `append` checks a live fencing token and revision, then atomically replaces the snapshot and appends ordered immutable events. Both return errors rather than acknowledging failed persistence. Snapshots/events are owned, detached values.
- Typed IDs serialize as Go strings. Payloads retain arbitrary-precision JSON numbers and explicit `null`. Year-one zero times, nanosecond RFC3339 timestamps, omitted fields and Go `null` slices are supported. UTC normalization preserves instants; JSON object ordering and timestamp offset spellings are not byte-identical promises.
- `decode_document` migrates v1 cancellation, budgets and missing event keys/sequences. Unknown schemas and effect classifications/states fail closed. Future embedded snapshots are rejected rather than relabeled as v2; PostgreSQL readers and lease mutations also validate snapshot schema, unlike the permissive baseline reader. `encode_document` accepts schema 0/current, not unknown versions.
- `prepared → dispatched → succeeded|failed|outcome_unknown` is enforced. Interrupted dispatched effects become outcome-unknown. Non-replayable dispatched work requires reconciliation; non-replayable outcome-unknown work requires operator resolution. No automatic retry is authorized for either. Idempotency keys are Go's `ga_` plus SHA-256 of `run:effect`.
- Cumulative budget counters and typed attempt, step, tool, approval, child-run and cancellation boundaries are preserved without runner policy or recomputation.

## Filesystem

Files are Go-compatible `tenants/<tenant>/<run>.json` records with `{document, lease}`. Per-tenant `locks/<tenant>.lock` files use maintained `fs4`'s Unix `flock`, interoperable with Go and effective across processes. Lock files intentionally survive deletion to avoid split lock identities. Writes create a private temporary file, sync it, atomically rename it, and sync the directory. Deletion syncs the directory as well. Directories are created `0700`, records/locks `0600`; permissions are tightened on existing store directories. Symlink tenant/record/lock paths, hard-linked files, unsafe IDs and mismatched persisted tenant/run keys are rejected.

The store requires Unix for private file semantics, a trusted private root (and trusted ancestors), and a local filesystem supporting `flock`, atomic rename and fsync. It does not defend against malicious same-UID processes replacing ancestors, nor claim network-filesystem locking guarantees. Non-Unix hosts should use PostgreSQL.

## PostgreSQL

Enabled by default; `--no-default-features` builds filesystem/codecs only. `PostgresStore::connect(url, options)` uses `NoTls`; for production TLS use `PostgresStore::new(caller_configured_postgres_client, options)`. The owned connection is serialized internally; independent stores/connections/processes coordinate through row locks. `init()` transactionally creates the existing Go `durable_runs` / `durable_events` tables and retention index; it does not redesign or migrate production tables. They also match platform migration `042_sdk_durable_runs.up.sql` (the unrelated conversation-message index is not owned here).

Append holds a row lock through all event inserts and the snapshot update, with rollback on any failure. Load uses a read-only repeatable-read transaction so concurrent deletion/recreation cannot mix histories. Lease times come from database `clock_timestamp()` **after** locking, avoiding client clock skew and stale transaction-start time. PostgreSQL BIGINT overflow is rejected explicitly. All run operations use both tenant and run keys; tenant deletion/retention rely on the existing cascade foreign key.

## Privacy and lifecycle

`StoreOptions` accepts caller-owned, fallible `Redactor` and `Encryptor` hooks. Redaction covers snapshot state, step data, tool inputs/outputs, effect outcomes and event payloads, using Go's classification inheritance. Hooks can fail closed. These are payload transformations, not automatic scrubbing of arbitrary strings such as owner names or cancellation reasons.

Filesystem encryption covers the **entire record including the lease**, using Go's `{ "encrypted": true, "data": "<base64>" }` envelope. Every mutation, including acquire/renew/release, preserves encryption. PostgreSQL encrypts each snapshot/event body in the same envelope; indexed keys, revisions, deadlines and lease columns remain cleartext by schema contract. An encryptor-configured store rejects plaintext envelopes to prevent downgrade attacks; removing encryption or importing plaintext into such a store requires an explicit, offline trusted migration. Encrypted input without a decryptor is always rejected. The caller must supply authenticated encryption and key management; the test-only XOR hook is **not security**.

`delete_run`, `delete_tenant`, and `apply_retention` explicitly remove persisted runs/events. Missing retention deadlines mean retain indefinitely. Retention has administrative whole-store scope, can delete leased runs, and may partially delete before returning a later IO error. Backups, filesystem snapshots, database WAL and keys remain the application's deletion responsibility.

## Verification

```sh
cargo test -p adk-durable
cargo test -p adk-durable --no-default-features
ADK_TEST_GO=1 ADK_TEST_POSTGRES_URL='host=127.0.0.1 port=5432 user=postgres dbname=postgres sslmode=disable' \
    cargo test -p adk-durable -- --nocapture
cargo clippy -p adk-durable --all-targets -- -D warnings
```

`ADK_TEST_POSTGRES_URL` enables real database tests; each creates and drops only a uniquely named test schema. `ADK_TEST_GO=1` enables actual baseline Go filesystem and PostgreSQL reader/writer roundtrips, requiring Go 1.26.2+ and the vendored `repos/sdk` checkout. Without these variables the optional integrations report their skip. Tests include live process contention/expiry/fencing, concurrent CAS, the platform SQL schema, Go-generated fixtures, encryption/redaction/privacy, tenant isolation, injected effect-boundary failures, and transaction rollback after event insertion. See [`fixtures/durable/README.md`](../../fixtures/durable/README.md) for fixture provenance.
