# Durable execution and memory research (#9)

Verified 2026-09-17. These are design references, not copied implementations or
claims that an external ADK satisfies our persisted contracts. Exact resolved
versions/checksums are in `Cargo.lock`; all project code remains GPL-3.0-only.

## Rust ADK patterns

| Reference | Verified version/license | Decision |
|---|---|---|
| [ADK graph Checkpointer](https://docs.rs/adk-graph/2.2.0/adk_graph/checkpoint/trait.Checkpointer.html) | 2.2.0, Apache-2.0 | Adapt checkpoint/backend separation. The trait does not expose expected revision and fencing token parameters; do not adopt it as our concurrency contract. |
| [ADK memory](https://docs.rs/adk-memory/2.2.0/adk_memory/) | 2.2.0, Apache-2.0 | Adapt memory-service/backend separation and explicit deletion. Do not copy global/project search semantics into tenant-confined stores. |
| [SQLx](https://crates.io/api/v1/crates/sqlx), [transaction ownership](https://docs.rs/sqlx/0.9.0/sqlx/struct.Transaction.html) | 0.9.0, MIT OR Apache-2.0, Rust 1.94 | Reject latest for this Rust 1.88 workspace. Explicit transaction owners/rollback-on-drop are useful regardless of driver. ADK 2.2 declares SQLx ^0.8, not 0.9. |

## Store and filesystem candidates

| Reference | Verified version/license | Decision |
|---|---|---|
| [rust-postgres metadata](https://crates.io/api/v1/crates/postgres/0.19.12), [repository](https://github.com/rust-postgres/rust-postgres) | 0.19.12, MIT OR Apache-2.0, Rust 1.81 | Use the native PostgreSQL driver family with owned transactions and existing BYTEA/relational schema. A synchronous store boundary must not be invoked directly on an async runtime worker. |
| [rusqlite metadata](https://crates.io/api/v1/crates/rusqlite/0.37.0), [current docs](https://docs.rs/rusqlite/0.40.2/rusqlite/) | inspected 0.37.0 and current 0.40.2, MIT | Use 0.37 for this workspace's compiler/dependency constraints and a single bundled SQLite driver. Preserve the Go tables, project scoping and little-endian vector format. |
| [fs4](https://docs.rs/fs4/1.1.0/fs4/) | 1.1.0, MIT OR Apache-2.0; sync MSRV 1.75 | Prefer maintained advisory-lock wrapper; file locks only serialize cooperating local writers, not distributed external effects. |
| [fs2 metadata](https://crates.io/api/v1/crates/fs2/0.4.3) | 0.4.3, MIT/Apache-2.0; last release 2018 | Evaluated, not preferred over fs4. |
| [tempfile persist](https://docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html#method.persist) | 3.27.0, MIT OR Apache-2.0 | Use same-directory temporary replacement, explicitly sync file and parent directory. `persist` alone is not a crash-durability guarantee. |

Recent publication/docs are maintenance signals, not a security audit. Dependency
licenses and version duplication are checked separately by the repository's
`cargo deny` policy. The resolved PostgreSQL driver is 0.19.14 (same
MIT OR Apache-2.0 license; [version metadata](https://crates.io/api/v1/crates/postgres/0.19.14)).

The policy retains exact-version exceptions rather than weakening global bans:
PostgreSQL's current RustCrypto stack coexists with the existing SDK's SHA-1/SHA-2
0.10 stack; PostgreSQL and SQLite use distinct `fallible-iterator` APIs; `whoami`
and `mio` use distinct WASI generations. Existing schema-validation/proc-macro
dependencies also resolve two RNG/`syn` generations. These are API incompatibilities,
not interchangeable duplicate versions. `foldhash` 0.1.5 (via SQLite's hashlink)
has a reviewed Zlib license: retain its copyright/notice, do not misrepresent
origin, and identify altered source. Only that exact version is allowlisted.

## Decisions from baseline contracts

The input is SDK `1dc92b73900fac74dc357a938e4b5eee6392b418` (v0.0.115),
not a newly designed database. Sources:

- [durable](https://github.com/gratefulagents/sdk/tree/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/durable): schema 2 document, v1 migration, snapshot/event JSON, SHA-256 idempotency key, lease token and CAS semantics.
- [runner checkpoint](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/durable_checkpoint.go): independent schema 1 and pointer-free history.
- [projectstate](https://github.com/gratefulagents/sdk/tree/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/projectstate): event-sourced tasks/memories, filesystem log and SQLite tables.
- [platform migration 042](https://github.com/gratefulagents/gratefulagents/blob/08e65c970830f05042c251bcbb46ec6a9e3719b9/internal/store/postgres/migrations/042_sdk_durable_runs.up.sql): unchanged durable_runs/durable_events schema and retention index.

Use Rust newtypes for identities, exhaustive effect states, owned transaction and
lease values rather than reproducing Go mutex ownership. All state-changing
writes must check tenant, revision and a current fence atomically. A database
commit error is an error even when its outcome cannot be determined. Unknown
schema versions and unresolved effects stop execution.

[PostgreSQL locking](https://www.postgresql.org/docs/18/explicit-locking.html)
supports row-level serialization; maintain short transactions and consistent lock
ordering. [SQLite transaction documentation](https://www.sqlite.org/lang_transaction.html)
explains why `BEGIN IMMEDIATE` may return busy and why commit failures cannot be
treated as success. Bound contention and propagate errors.

Persist prepared intent before dispatch, mark dispatch before external work, and
persist completion before continuing. A crash between dispatch and completion is
outcome-unknown. A fence prevents stale database writes, **not** an external
service from performing an already-started request. Stable idempotency keys only
help when the destination honors them. Never claim exactly-once external effects
or automatically retry a non-replayable uncertain effect.

## Integration boundary

The current checkout has no built-in state-tool registry from #7. Its
[integration clarification](https://github.com/gratefulagents/adk/issues/7#issuecomment-5698139570)
requires testing tools against #9 before #7 closes. Store/engine tests here are
not substitutes for those future model-facing tool-contract tests. Platform
reader compatibility is a persisted-format obligation, not authorization to
change production data or run production migrations.
