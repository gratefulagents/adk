# State / namespace Memory / plan gap oracle

Pinned source: SDK v0.0.115, commit
`1dc92b73900fac74dc357a938e4b5eee6392b418` (GPL-3.0-only).

Regenerate from `repos/sdk`:

```sh
go run ../../fixtures/tools/state-memory-plan-generate.go > ../../fixtures/tools/state-memory-plan-expected.json
```

The generator executes actual SDK tools, not reimplemented expected behavior.
`state-memory-plan-cases.json` supplies 51 state calls. The generator also supplies
six namespace Memory calls, eight plan cases, and three missing-host-store cases.
Rust tests replay all state calls against both #9 filesystem and SQLite stores;
Memory uses #9 InMemoryStore. Plans use a recording ArtifactStore. Existing
15-tool schema comparisons are deliberately not duplicated. UUIDs, timestamps
and comment IDs are normalized; Memory similarity compares JSON numbers
numerically (`1` and `1.0`). No diagnostic text or null/empty arrays are normalized.

## Covered contracts

- task_ready: blocked/unblocked, host/other/unassigned, labels, assignee,
  include_assigned and binding limits with exact ordered task IDs.
- task_show/update/close missing IDs; claim/comment explicit trimmed actor;
  competing claim; empty comment; missing endpoint, self and cyclic links.
- memory_list/recall: competing kinds/tags and binding limits; mixed filtered
  memory_stats counts; memory_update kind/scope/file_paths/task_ids/source_run;
  missing/nonexistent memory_delete; prime memory/ready limits and unknown active ID.
- Namespace Memory: nullable optional fields and null tag elements, padded UUID
  delete, host identity; additional Rust recording/delegating store checks query,
  tags, namespace and limits. A 55-matching-record population binds search=10
  and list=50 defaults, positive limits, zero/negative/null defaults, real tag
  exclusion and cross-namespace isolation.
- Plan: absent and empty artifact, default summaries below/exactly/above 200
  bytes, 200-byte UTF-8 and a split UTF-8 code point at the 200-byte boundary.

## Construction boundary

Go permits nil stores and returns the three captured `missing_store` tool errors.
Rust APIs require `Arc<dyn Store>` / `Arc<dyn ArtifactStore>`: a nil store cannot
be passed. Host-only tools are absent without explicit injection; separate tests
assert that boundary. This is deliberately not claimed as runtime-error parity.

## State parity regressions found and fixed

The initial strict replay found nine differences on each backend. Shared engine
and tool serialization fixes now preserve the following Go results without
normalizing diagnostic text or null/empty arrays in the test:

- Steps 15/16/17/22: Go `task "missing" not found`; Rust `task missing not found`.
- Step 23: Go `dependency task "missing" not found`; Rust `task missing not found`.
- Steps 33/37: Go empty filtered memory results are `null`; Rust returns `[]`.
- Step 49: Go `memory "" not found`; Rust `memory  not found`.
- Step 50: Go `memory "missing" not found`; Rust `memory missing not found`.

The audit's suggestion that a competing claim and a cyclic task_link should
produce errors does not match the pinned SDK: claim reassigns the in-progress
task to the explicit competitor; adding the cycle succeeds. Tests preserve these
actual baseline results rather than inventing stricter semantics. An unknown
active task also succeeds, producing ordinary bounded project context.

No production Memory/plan bug was found; their existing null/UUID/empty artifact
and summary implementation already satisfies the added cases. State fixes are in
`adk-project-state/src/engine.rs` and `tools.rs`, with additional validation-order
and no-mutation-on-failure regressions in that crate. This corpus does not establish tool-level
cancellation, store failure injection for every operation, or persisted host
restart for namespace Memory/plan.

## Fresh verification

- Actual Go regeneration exactly matched all 68 recorded cases (51 state,
  6 Memory, 8 plan, 3 nil-store).
- Final focused tests: Memory 5 passed; signals 6 passed; state 2 passed.
  The strict replay checks all 51 steps on both backends: all 102 outputs match.
  The underlying state crate's 21 tests also pass. No failed test was ignored,
  weakened, or hidden with extra normalization.
- Focused cargo check, rustfmt checks, and git diff whitespace checks passed.
- Clippy passed with warnings denied and no dependency linting, invoking
  clippy-driver as RUSTC_WORKSPACE_WRAPPER. The cargo-clippy launcher itself
  failed because this host lacks /proc/self/exe.
- Rust verification used installed toolchain 1.97.1 directly, its library path
  in LD_LIBRARY_PATH, and the BFD linker instead of the /proc-dependent lld
  wrapper. Go used explicit GOROOT=/usr/local/go and emitted a nonfatal telemetry
  warning. No repository build configuration was changed.
