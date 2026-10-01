# Project-state persisted-record proof (bounded)

SDK pin: `1dc92b73900fac74dc357a938e4b5eee6392b418` (checkout HEAD verified, untouched).
**46 immutable field IDs mapped: 46 bounded proven, 0 remaining record-codec blockers.**
No inventory/source-lock/claims/overlay/evidence-script/pinned-SDK/archived-ledger edits;
no commits or mass closure. Parent reviews and integrates proof hashes/claims.

The machine-readable companion, `fixtures/project-state/record-proof-map.json`,
was synchronized by the parent after review: 46 bounded proven fields, including
required offset and metadata-null cases. The integrated evidence gate independently
regenerates the Go fixtures before accepting Rust field claims.

## Oracle, audit, and fixes

`fixtures/project-state/records/main.go` imports the pinned Go record types, constructs
inputs, then **Go-unmarshals and Go-marshals each input** to produce `records.json`.
Rust decodes each input into the named record and encodes it for whole-object comparison
against **Go expected**, not Rust self-roundtrip. Key presence and nested values are checked;
object ordering and raw JSON whitespace are not byte-identity claims.
All 46 fields have nondefault keys. **All 47 generated cases pass strict equality**:
Event 8, Task 12, TaskComment 8, Memory 11, SessionSummary 8.

Required cases for every type: `nondefault`, `omitted`, `null`, `empty`, `offset`,
`offset_negative`, `offset_integral`, `offset_zero`. Task and Memory additionally require
`metadata_null`, `metadata_empty`, `metadata_false`; Task also requires `wide_priority`.
`compare<T>` explicitly requires each case, its input, and its Go expected output;
missing cases cannot silently skip proof. The former diagnostic mismatch probe was removed
and its cases now participate in the same strict equality assertions as all other cases.

Covered: seq=i64::MAX; JSON integer=18446744073709551615 and fraction=
0.12345678901234567890123456789; task priority=9007199254740993 on this 64-bit Go host;
nanosecond, trimmed-fraction and integral timestamps; Go year-1 zero time;
optional timestamps present/absent/null; scalar and list omitted/null/empty handling.
Offsets cover +05:30 with nanoseconds and integral seconds, -03:30 with trimmed fractions,
and +00:00 normalized to Go's Z. Task.Comments includes nested offset timestamps.
Metadata covers omitted, explicit null, empty object, false, and nested nondefault JSON.
The generic `null` cases omit metadata; `metadata_null` independently proves present null.

### Earlier fixes retained

- Null scalar decoding yields Go zero values; omitted/null mandatory timestamps yield
  `0001-01-01T00:00:00Z`. Persisted Task.priority remains i64; engine inputs remain clamped i32.
- The earlier baseline generator repair prevented literal `task_ids` keys being normalized
  into SDK IDs. Existing regenerated events/backends/indexes/expected files were not changed
  in this follow-up. `go_baseline_retains_task_ids_json_keys` still checks associations and
  typed event decoding. Original expected state lacked nondefault Memory.last_read_at,
  Memory.metadata, Memory.task_ids and SessionSummary.task_ids; the record fixture covers all.

### Closed gaps and Rust-native API decisions

- The ten timestamp fields on Event, Task, TaskComment, Memory and SessionSummary now use
  `DateTime<FixedOffset>` (optional where appropriate). The shared Go formatter trims only
  fractional trailing zeroes, preserves nonzero offsets, and emits Z for zero offset.
  Project and the separate namespace `memory::Memory` retain their UTC field types.
- Persisted Task.metadata and Memory.metadata now use `Option<Value>`. Missing means None
  and is omitted; present JSON null deserializes to Some(Value::Null) and serializes as null.
  Non-null JSON is Some(value). This is an intentional pre-release Rust-native API change.
- CreateTaskInput and UpsertMemoryInput remain Value-based: their default Null continues to
  mean omission, explicitly converted by the engine. TaskPatch remains Option<Value>:
  native Some(Value::Null) now persists explicit null, while None leaves metadata unchanged.
  Input JSON null-vs-omission parity is not claimed.
- Engine-created timestamps explicitly convert UTC to fixed offset; replay `at` timestamps
  decode directly with their offsets. SQLite's integer-nanosecond event timestamp reconstructs
  a zero-offset value; its storage format has no original offset information (unchanged).
  Recall uses cross-timezone `signed_duration_since` while its clock input remains UTC.
- Local callers/tests use the new field types. Repository search found downstream references
  in `crates/adk-tools/{src/lib.rs,src/memory.rs,tests/state.rs,tests/memory.rs}` and
  `crates/adk/{src/lib.rs,examples/features.rs}`; none requires a source fix for these changes.
  The namespace-memory metadata accesses there use the unchanged `memory::Memory` type.
  Parent should run broader workspace integration; no other crates were edited here.

Scope is fresh JSON record decoding, not in-place Go unmarshal, Rust `Default` construction,
all malformed inputs, complete backends, input options, 32-bit Go int behavior or unsupported OS.
No whole-project or whole-SDK conformance claim follows from these bounded field proofs.

## Executed verification (this follow-up)

Using the supplied Rust 1.88 / Go environment, after implementation:

- `cargo test --locked -p adk-project-state`: **31 passed**, 0 failed/ignored
  (contracts 21, record_codecs 7, tools 3; unit/doc suites 0).
  Includes strict Go record cases, baseline replay on both backends,
  `replay_preserves_fixed_offsets_in_task_transition_payloads`, and
  `persisted_metadata_presence_survives_mutations_and_reopen` (both backends).
- `RUSTC_WORKSPACE_WRAPPER=/workspace/scratch/rust188/bin/clippy-driver CLIPPY_ARGS=-Dwarnings
  cargo check --locked -p adk-project-state --all-targets`: **PASS**, Clippy/typecheck.
  This invokes the driver directly to avoid the environment's `/proc/self/exe` launcher issue.
  Initial cross-timezone subtraction compile failure and an uninlined-format Clippy finding
  were fixed; final runs above are clean, with no warning suppressions.
- Scoped `rustfmt --edition 2024 --check`, `gofmt -l` and `git diff --check`: **PASS**.
- From `repos/sdk`: `go run ../../fixtures/project-state/records/main.go -out
  ../../fixtures/project-state/records/.verification-tmp/records.json`, then `cmp` against
  `fixtures/project-state/records.json`: **identical independent regeneration**.
- From `repos/sdk`: `go test ./pkg/agentsdk/projectstate -run
  'Test(Filesystem|SQLite)Store(TaskLifecycle|Memories)' -count=1`: **PASS**.
- `cargo run --locked -p adk-project-state --example export_roundtrip --
  fixtures/project-state/records/.verification-tmp/rust-export`, then from `repos/sdk`
  `go run ../../fixtures/project-state/verify/main.go
  ../../fixtures/project-state/records/.verification-tmp/rust-export`:
  **PASS Rust -> Go filesystem and SQLite replay, schemas, priming, subsequent Go writes**.
- Go printed nonfatal telemetry-sidecar `/proc/self/exe` warnings and exited 0.
  Temporary regeneration/export artifacts were removed after verification.

Environment: `PATH=/workspace/scratch/rust188/bin:/usr/local/go/bin:/usr/local/bin:/usr/bin:/bin`,
`LD_LIBRARY_PATH=/workspace/scratch/rust188/lib`, `RUSTC=/workspace/scratch/rust188/bin/rustc`,
`RUSTDOC=/workspace/scratch/rust188/bin/rustdoc`, `CARGO_HOME=/workspace/scratch/cargo`,
`CARGO_TARGET_DIR=/workspace/scratch/target`, `GOROOT=/usr/local/go`, `GOTOOLCHAIN=local`,
`GOPATH=/workspace/scratch/go`, `GOCACHE=/workspace/scratch/go-cache`.

## Per-field acceptance map

Every source field is in `pkg/agentsdk/projectstate/types.go`; Rust symbols are exported
under `adk_project_state`, defined in `crates/adk-project-state/src/types.rs`.
Fixture pointers below are within `fixtures/project-state/records.json`.
Assertion **A** is `encode::<Type>(case.input) == case.expected` in `compare<Type>`:
full-object equality includes each named key and absence/presence across all required cases
listed above. The pointers highlight nondefault values or the formerly blocked subcase;
they do not restrict the test to that single case. **P = bounded proven**.

Executed test aliases, all in `crates/adk-project-state/tests/record_codecs.rs`:

- **Event**: `go_event_record_decode_encode_matches_go`.
- **Task**: `go_task_record_decode_encode_matches_go`.
- **TaskComment**: `go_task_comment_record_decode_encode_matches_go`.
- **Memory**: `go_memory_record_decode_encode_matches_go`.
- **SessionSummary**: `go_session_summary_record_decode_encode_matches_go`.

Upstream reference anchors in `docs/verification/issue-11-pinned-reference.json.tests`
(previously checked pass; matching pinned tests rerun above). These are related
lifecycle/memory provenance, **not** per-field assertions or SessionSummary-specific proofs:

- **L**: `github.com/gratefulagents/sdk/pkg/agentsdk/projectstate/TestFilesystemStoreTaskLifecycleAndIndexes` = `pass`.
- **M**: `github.com/gratefulagents/sdk/pkg/agentsdk/projectstate/TestFilesystemStoreMemoriesAndPrime` = `pass`.

| Immutable acceptance ID | Source field | Rust symbol | Exact fixture pointer (A) | Rust test alias | Ref | Result |
|---|---|---|---|---|---|---|
| `SDK-6396BBE7A925A0B4` | `Event.Actor` | `Event::actor` | `/Event/nondefault/expected/actor` | Event | L | P |
| `SDK-BAE515A1F6A18DC4` | `Event.EventID` | `Event::event_id` | `/Event/nondefault/expected/event_id` | Event | L | P |
| `SDK-D8B154F141A66338` | `Event.Payload` | `Event::payload` | `/Event/nondefault/expected/payload` | Event | L | P |
| `SDK-7E53349E5DE47649` | `Event.ProjectID` | `Event::project_id` | `/Event/nondefault/expected/project_id` | Event | L | P |
| `SDK-089A0C518E5206BB` | `Event.RunID` | `Event::run_id` | `/Event/nondefault/expected/run_id` | Event | L | P |
| `SDK-8788937B2076945B` | `Event.Seq` | `Event::seq` | `/Event/nondefault/expected/seq` | Event | L | P |
| `SDK-D20A1492D51821A4` | `Event.Time` | `Event::time` | `/Event/offset/expected/time` | Event | L | P |
| `SDK-42C941783A75E82B` | `Event.Type` | `Event::event_type` | `/Event/nondefault/expected/type` | Event | L | P |
| `SDK-B58CFB713281DDE5` | `Memory.Content` | `Memory::content` | `/Memory/nondefault/expected/content` | Memory | M | P |
| `SDK-7226EFFEFC8AFB5E` | `Memory.CreatedAt` | `Memory::created_at` | `/Memory/offset/expected/created_at` | Memory | M | P |
| `SDK-3DDBABA38139AF93` | `Memory.FilePaths` | `Memory::file_paths` | `/Memory/nondefault/expected/file_paths` | Memory | M | P |
| `SDK-6EE5835AEFB817EE` | `Memory.ID` | `Memory::id` | `/Memory/nondefault/expected/id` | Memory | M | P |
| `SDK-388ACD150693D533` | `Memory.Kind` | `Memory::kind` | `/Memory/nondefault/expected/kind` | Memory | M | P |
| `SDK-4E0355E04C6BB042` | `Memory.LastReadAt` | `Memory::last_read_at` | `/Memory/offset/expected/last_read_at` | Memory | M | P |
| `SDK-4E35F81F30F76ABC` | `Memory.Metadata` | `Memory::metadata` | `/Memory/metadata_null/expected/metadata` | Memory | M | P |
| `SDK-5BD024DC782785C0` | `Memory.Scope` | `Memory::scope` | `/Memory/nondefault/expected/scope` | Memory | M | P |
| `SDK-2643CE3FF456EFE8` | `Memory.SourceRun` | `Memory::source_run` | `/Memory/nondefault/expected/source_run` | Memory | M | P |
| `SDK-FA3954363239567D` | `Memory.Tags` | `Memory::tags` | `/Memory/nondefault/expected/tags` | Memory | M | P |
| `SDK-B10FA43A588D8A39` | `Memory.TaskIDs` | `Memory::task_ids` | `/Memory/nondefault/expected/task_ids` | Memory | M | P |
| `SDK-38638FBF34487A28` | `Memory.UpdatedAt` | `Memory::updated_at` | `/Memory/offset/expected/updated_at` | Memory | M | P |
| `SDK-DCCEDA8D4308F62F` | `SessionSummary.CreatedAt` | `SessionSummary::created_at` | `/SessionSummary/offset/expected/created_at` | SessionSummary | M | P |
| `SDK-93684FE07A6C5235` | `SessionSummary.ID` | `SessionSummary::id` | `/SessionSummary/nondefault/expected/id` | SessionSummary | M | P |
| `SDK-A06F0E283E40C18F` | `SessionSummary.RunID` | `SessionSummary::run_id` | `/SessionSummary/nondefault/expected/run_id` | SessionSummary | M | P |
| `SDK-001153C51DDFE204` | `SessionSummary.Summary` | `SessionSummary::summary` | `/SessionSummary/nondefault/expected/summary` | SessionSummary | M | P |
| `SDK-0C2F5C21D560C288` | `SessionSummary.TaskIDs` | `SessionSummary::task_ids` | `/SessionSummary/nondefault/expected/task_ids` | SessionSummary | M | P |
| `SDK-3598C768C40D7059` | `SessionSummary.UpdatedAt` | `SessionSummary::updated_at` | `/SessionSummary/offset/expected/updated_at` | SessionSummary | M | P |
| `SDK-BBB472350B9D2D99` | `Task.Assignee` | `Task::assignee` | `/Task/nondefault/expected/assignee` | Task | L | P |
| `SDK-3FD1BDFD3D4D096B` | `Task.Blocks` | `Task::blocks` | `/Task/nondefault/expected/blocks` | Task | L | P |
| `SDK-428D87225C63A3A9` | `Task.ClosedAt` | `Task::closed_at` | `/Task/offset/expected/closed_at` | Task | L | P |
| `SDK-563D626584AF00C4` | `Task.Comments` | `Task::comments` | `/Task/nondefault/expected/comments` | Task | L | P |
| `SDK-390F90BBC25090F0` | `Task.CreatedAt` | `Task::created_at` | `/Task/offset/expected/created_at` | Task | L | P |
| `SDK-3AFD0E04F7911246` | `Task.DependsOn` | `Task::depends_on` | `/Task/nondefault/expected/depends_on` | Task | L | P |
| `SDK-C7C5735E663FC993` | `Task.Description` | `Task::description` | `/Task/nondefault/expected/description` | Task | L | P |
| `SDK-14ABA48943E07CF6` | `Task.ID` | `Task::id` | `/Task/nondefault/expected/id` | Task | L | P |
| `SDK-8986E085B25CBE5F` | `Task.Labels` | `Task::labels` | `/Task/nondefault/expected/labels` | Task | L | P |
| `SDK-4F7F4F866961A827` | `Task.Metadata` | `Task::metadata` | `/Task/metadata_null/expected/metadata` | Task | L | P |
| `SDK-C7ED7B94A3AED388` | `Task.Priority` | `Task::priority` | `/Task/nondefault/expected/priority` | Task | L | P |
| `SDK-BEDAABF6C293FFD6` | `Task.SourceRun` | `Task::source_run` | `/Task/nondefault/expected/source_run` | Task | L | P |
| `SDK-9453ACFE0EFD1B65` | `Task.Status` | `Task::status` | `/Task/nondefault/expected/status` | Task | L | P |
| `SDK-2E94F960FBE80DD4` | `Task.Title` | `Task::title` | `/Task/nondefault/expected/title` | Task | L | P |
| `SDK-99E93611C3449F29` | `Task.Type` | `Task::task_type` | `/Task/nondefault/expected/type` | Task | L | P |
| `SDK-B7532526ACB391FF` | `Task.UpdatedAt` | `Task::updated_at` | `/Task/offset/expected/updated_at` | Task | L | P |
| `SDK-36B0F237DF024EF1` | `TaskComment.Actor` | `TaskComment::actor` | `/TaskComment/nondefault/expected/actor` | TaskComment | L | P |
| `SDK-3EF9FD241BEC804F` | `TaskComment.Body` | `TaskComment::body` | `/TaskComment/nondefault/expected/body` | TaskComment | L | P |
| `SDK-98AC086A106A8D00` | `TaskComment.CreatedAt` | `TaskComment::created_at` | `/TaskComment/offset/expected/created_at` | TaskComment | L | P |
| `SDK-C2FBF7ABF92D5AF6` | `TaskComment.ID` | `TaskComment::id` | `/TaskComment/nondefault/expected/id` | TaskComment | L | P |

## Files changed in this follow-up

- `crates/adk-project-state/src/{types,engine,storage,recall}.rs`.
- `crates/adk-project-state/tests/{contracts,record_codecs}.rs`.
- `fixtures/project-state/records/main.go` and `fixtures/project-state/records.json`.
- This report. Earlier baseline/proof-map work and concurrent parent/other-agent changes
  are not edits made in this follow-up. No commits were created.
