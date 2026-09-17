# Durable project state and memory

`adk-project-state` ports the version-1 contracts in the Go SDK's
`pkg/agentsdk/projectstate` and the separate namespace-memory contract in
`pkg/agentsdk/memory`. Source baseline:
`1dc92b73900fac74dc357a938e4b5eee6392b418`. This is SDK-derived GPL-3.0-only code;
fixture provenance is in `fixtures/project-state/NOTICE.md`.

## API and integration

```rust
use adk_project_state::{CreateTaskInput, FilesystemOptions, MemoryFilter,
    PrimeOptions, ProjectStore, StoreOptions, TaskFilter, UpsertMemoryInput};

# fn example() -> adk_project_state::Result<()> {
let state = ProjectStore::filesystem(FilesystemOptions {
    state_dir: ".agent-state".into(),
    store: StoreOptions {
        project_id: "my-project".into(),
        actor: "agent-alice".into(),
        run_id: "run-123".into(),
        ..Default::default()
    },
    ..Default::default()
})?;
let task = state.create_task(CreateTaskInput {
    title: "Ship the fix".into(), priority: 1, ..Default::default()
})?;
state.claim_task(&task.id, "agent-alice")?;
state.upsert_memory(UpsertMemoryInput {
    kind: "pinned".into(), content: "Run cargo test before shipping".into(),
    task_ids: vec![task.id], ..Default::default()
})?;
let ready = state.ready_tasks(TaskFilter::default())?;
let recalled = state.search_memories(MemoryFilter {
    query: "cargo test".into(), ..Default::default()
})?;
let briefing = state.prime_context(PrimeOptions::default())?;
# let _ = (ready, recalled, briefing);
# Ok(()) }
```

`ProjectStore::sqlite(SQLiteOptions)` uses the same engine and can share a database
between projects. `sqlite_connection` takes ownership of a `rusqlite::Connection`
and supports custom, validated table prefixes. Dropping the store releases its
connection; it does not delete persisted state. Connections are mutex protected.
All operations are blocking: asynchronous hosts should execute them on a bounded
blocking executor. Lock waits have a configurable filesystem timeout (30 seconds
by default); SQLite uses a five-second busy timeout. Provider implementations must
bound their own embedding I/O. Dropping an async wrapper does not cancel an already
running filesystem transaction.

Object-safe `TaskStore`, `MemoryStore`, `SessionStore`, `PrimeStore`, and combined
`Store` traits allow hosts to inject state without depending on a backend.
`State::replay`, `events()`, `state()`, `memory_stats()`, and retention methods are
also exposed. There is no global singleton or implicit network provider.

**Issue #7 tool-registry integration remains external/pending.** Inspection found
only `adk_core::Tool`, not an existing Rust task/memory-specific store interface.
No registry, authorization bypass, or unrelated tool catalog is added here. A #7
adapter should deserialize the input DTOs, call these traits on its blocking
executor, and preserve the host's `ToolContext` authorization/cancellation rules.
Host policy, not a model-selected project ID or filesystem path, must select the
store. The serde DTO names/fields match the Go tool input contracts.

## Behavior compatibility

- Tasks: create, partial patch, claim, close/reopen, comments, dependency add/remove,
  reverse `blocks`, list/get, and ready-work filtering. Missing blockers block work;
  priorities clamp to 0–4; Go status/type aliases are accepted. Labels require all
  requested labels, case-insensitively. Task ordering is status, priority, descending
  update time, then ID. Labels change only with `replace_labels=true`.
- Claim is serialized but **not compare-and-set ownership**: like Go, a subsequent
  claim can reassign an already claimed task. Dependencies may form cycles (which
  remain blocked); adding a self-dependency is rejected. Creation permits unresolved
  dependency IDs, matching Go.
- Typed project memories: pinned/semantic/episodic/procedural, project/user/task/file
  scope, content, tags, task/file references, source run, timestamps, opaque JSON
  metadata. Upsert replaces fields rather than patching them, preserving creation
  time. A host implementing `memory_update` must merge omitted fields first.
- Session summaries preserve creation time on replacement. Priming includes active,
  ready and blocked work and pinned/recent memories with Go-compatible truncation.
- Unknown event types are retained and ignored on replay for forward compatibility.
  Unsupported project schema versions fail rather than being overwritten. Metadata
  integers retain precision beyond IEEE-754, and timestamps serialize as Go-style
  RFC3339Nano. Null Go slices are accepted.
- `memory::InMemoryStore` is intentionally a different contract: UUID IDs, namespaces,
  any-tag matching (case-sensitive), phrase-first substring/term scores, 10-result
  search and 50-result list defaults. Results are owned copies. Namespace isolation
  applies to list, search and deletion. `memory::Embedder`, `NoopEmbedder`, and
  `vector_literal` preserve the separate single-text embedding interface.

## On-disk formats and concurrency

Filesystem layout:

```text
state/
  events.jsonl                 authoritative event stream
  indexes/project.json         derived project snapshot
  indexes/tasks.json           {schema_version, updated_at, tasks}
  indexes/memories.json        {schema_version, updated_at, memories}
  indexes/sessions.json        {schema_version, updated_at, sessions}
  indexes/embeddings.json      {schema_version, model, updated_at, vectors}
  snapshots/                  reserved by Go layout
  locks/state.lock            Go-compatible pid/token/time lock
  locks/rust-state.lock       stable Rust OS-lock inode; never delete while in use
```

Events contain `seq`, `event_id`, `project_id`, optional `run_id`/`actor`, `time`,
`type`, and `payload`. Both directions of interoperability are verified against
actual Go stores, not just hand-written JSON. Snapshots are regenerated from events
on open; they never override the event stream. A malformed final record is truncated
as a torn write; malformed records followed by more content fail. A valid final
record lacking a newline is preserved and separated before the next append.

A stable OS advisory lock (`fs4`) covers Rust recovery, replay, mutation and snapshot
replacement. Rust also acquires the Go token lock, so normal mixed-client operations
honor Go's lock. Rust only reclaims token locks whose Unix owner PID is demonstrably
dead; old live/unknown-owner locks time out instead of being stolen. An incomplete
lock with no PID needs operator recovery while all clients are stopped. PID reuse
can also require operator recovery. **Go itself has a stale-lock rename race and
age-based live-lock stealing; concurrent mixed-language stale recovery is not made
safe by a Rust-only lock.** For multiwriter guarantees use only Rust writers or
externally serialize/upgrade Go writers. Advisory locking requires a filesystem
with reliable lock/rename/fsync semantics; distributed/network filesystems are not
claimed safe.

SQLite tables match Go exactly, default prefix `projectstate_`:

- `events(project_id TEXT, seq INTEGER, event_id TEXT, run_id TEXT, actor TEXT,
  ts INTEGER, type TEXT, payload BLOB)`, primary key `(project_id, seq)`.
- `embeddings(project_id TEXT, memory_id TEXT, hash TEXT, model TEXT, dims INTEGER,
  vector BLOB)`, primary key `(project_id, memory_id)`.

`ts` is signed Unix nanoseconds; vectors are little-endian float32 blobs. Rust uses
WAL, FULL synchronous durability, foreign keys and secure deletion. An IMMEDIATE
transaction begins **before replay**, preventing lost read-modify-write updates
between Rust handles/processes. Go clients retain their own weaker read-before-write
semantics; sharing schemas does not strengthen another implementation's transactions.
There is one connection per store; table-prefix names are restricted to SQL identifier
characters. Custom/shared connections are owned and configured by this crate.

## Recall, privacy, and retention

`LexicalRecall` performs local case-insensitive substring/any-term matching over
content, kind, scope, tags and task/file references. Pinned matches precede other
matches, then newest first. Filters require all tags. Empty queries are rejected;
`list_memories` allows no query and returns all matches.

`EmbeddingRecall::search_with_embeddings` is a separate explicit capability with an
injected batch `Embedder`. It lazily caches vectors by SHA-256 content hash and model
ID, using Go's cache formats. Changing content or model invalidates the cache. Hybrid
ranking combines normalized term frequency/coverage, cosine similarity, pinned boost
and recency decay using Go defaults. No-relevance candidates cannot gain relevance
from boosts. Provider failure falls back to lexical search; malformed/nonfinite
vectors are never persisted. A memory changed/deleted during embedding is neither
resurrected in the cache nor returned stale. Cache persistence errors are reported.

Unlike Go's constructor-configured eager embeddings, ordinary Rust writes and
lexical reads **never** invoke an embedder. There is no built-in OpenAI HTTP client;
provider adapters implement one of the two explicitly separate embedding traits.
This keeps network disclosure opt-in and avoids coupling project state to a provider.

`StateHooks` supports:

- `prepare_memory`: redact/transform or reject input before persistence;
- `before_write`: authorize the final event before durable mutation;
- `after_write`: notify after a successful operation, outside the lock;
- `before_embed`: authorize text disclosure before calling an embedder. Denial is
  propagated rather than treated as a provider failure.

Pre-write hooks run under the storage lock and must not re-enter the same store.
Hooks are attached with `with_hooks`; initial project creation precedes their
installation. Hooks do not replace host-level authorization. Returned priming and
memory text are untrusted content, not instructions to the host. No secrets are
logged by this crate.

`delete_memory` appends a Go-compatible tombstone: historical content remains in the
log. `retain_memories` supports an update-time cutoff and maximum non-pinned count;
pinned memories are protected by default. `purge_history=true` uses the purge path.
`purge_memories(ids)` also handles already-tombstoned IDs, removes all corresponding
upsert/deletion events, renumbers sequences, prunes vectors and rewrites derived
snapshots. IDs are preserved for unrelated events. Purge emits a hook-only
`memory.purged` notice, not a new durable event carrying deleted content. The return
value is the number of removed events. Purging is an explicit destructive operation
and invalidates sequence-based cursors.

Cache cleanup precedes the authoritative deletion/rewrite; failure cannot commit a
tombstone while stranding its cache. Reopening reconciles orphan cached IDs under
lock, repairing older crash states. Filesystem operations are not a multi-file
transaction: an event append can succeed before a later snapshot fails. The error
must be surfaced; reopen repairs derived snapshots. SQLite operations are atomic.

New and existing managed directories/files are tightened to 0700/0600 on Unix.
Symlinks (including path ancestors) and nonregular state files are rejected; Unix
file opens use `O_NOFOLLOW`. Directory entries are synced for new directories,
initial event-log creation, and atomic replacements. Do not place state in a directory
that an adversarial same-UID process can rename concurrently; this is not an
openat-based adversarial filesystem sandbox. Existing SQLite parent directories
are also tightened, so use a dedicated private state directory.

**Purge is logical erasure, not guaranteed physical erasure.** SQLite WAL/free pages,
filesystem journal/COW snapshots, backups, other event types containing copied text,
and external embedding providers may retain bytes. Configure encrypted storage,
backup/provider retention and operational erasure separately. No automatic background
retention, third-party telemetry, or external disclosure is enabled.

## Verification and fixture regeneration

```sh
cargo test -p adk-project-state
cargo clippy -p adk-project-state --all-targets -- -D warnings
cargo fmt -p adk-project-state -- --check

# Uses only the checked-out baseline SDK; no provider requests.
(cd repos/sdk && GOROOT=/usr/local/go GOTOOLCHAIN=local \
  go run ../../fixtures/project-state/generate.go -out ../../fixtures/project-state)

# Use a fresh directory for each reverse roundtrip.
cargo run -p adk-project-state --example export_roundtrip -- /tmp/project-state-roundtrip
(cd repos/sdk && GOROOT=/usr/local/go GOTOOLCHAIN=local \
  go run ../../fixtures/project-state/verify/main.go /tmp/project-state-roundtrip)
```

Tests cover Go event/snapshot/namespace schemas, both backend replays, exact priming,
Go vector-cache reuse, large metadata integers, lifecycle/filter semantics, separate
handles and independent processes, abandoned-lock recovery, permissions/symlink
rejection, torn tails and interior corruption, privacy hooks, provider fallback,
retention/purge and failure-before-deletion/reopen cache reconciliation. Reverse
roundtrip verification opens Rust output in Go, compares tasks/memories/summaries/
ready work/priming, then appends a Go event. Tests require no API keys or live provider.

## State-backed tool contracts

`adk_project_state::tools::tools(Arc<dyn Store>, actor)` returns the 15 baseline
project-state tools as `adk_core::Tool` objects. This is independent of the full
#7 registry. Synchronous store I/O runs on the caller's executor thread, just as
with direct store calls; hosts should dedicate a blocking-capable execution
thread. Tool errors are model-visible; authorization/approval belongs to the
runner before dispatch. No store mutation is presumed replayable merely because
its tool name or read-only metadata is known.

`tests/tools.rs` tests both filesystem and SQLite adapters, including reopening,
actor/priority defaults, links, task updates, memory updates/deletion/recall/stats,
priming, schema metadata and error results. Go definitions and output traces are
compared against `fixtures/project-state/tools.json`. Regenerate using:

```sh
(cd repos/sdk && go run ../../fixtures/project-state/tools.go > ../../fixtures/project-state/tools.json)
```

The fixture covers all 15 tools, nullable/default inputs, malformed roots,
store errors and plain-text priming. It normalizes generated task/memory/comment
IDs and timestamps. Tests check `is_error` and compare business errors/priming
text exactly; language-specific JSON decoding errors retain the `Invalid input:`
classification instead of requiring identical Go/serde diagnostic wording.
