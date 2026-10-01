# SDK migration baseline ledger — schema v1

Upstream: [`gratefulagents/sdk`](https://github.com/gratefulagents/sdk), tag **v0.0.115**, commit **`1dc92b73900fac74dc357a938e4b5eee6392b418`**. The tag and checkout must both resolve to this commit; generation rejects a dirty checkout. This is a source inventory and proposed Rust ownership baseline, **not a claim of Rust parity or of passing SDK tests**.

## Reproduce and verify

Run from the repository root, with Git and a Go compiler supporting the source syntax:

```sh
go run scripts/inventory/main.go
go test -v scripts/inventory/main.go scripts/inventory/main_test.go
go vet scripts/inventory/main.go scripts/inventory/main_test.go
go run scripts/inventory/main.go -check
python3 scripts/inventory/validate.py
```

The generator uses only the Go standard library, never imports/builds the SDK, never downloads its dependencies, and never changes `repos/sdk`. No runtime tools, provider requests, commands from source files, or example/eval payloads are executed. On the inventory host the default Go launcher has a broken GOROOT; prepend `env GOROOT=/usr/local/go GOTOOLCHAIN=local` to Go commands. Go 1.26.8 was used for verification. Its telemetry launcher prints a warning about `/proc/self/exe`, but the compiler/test commands succeed with that environment.

All generated files are deterministic and versioned under `sdk-v0.0.115/`. `-check` regenerates in memory and compares bytes, including the manifest. The generator and routing-file digests in the manifest prevent silently reusing stale output after changing extraction or ownership rules. No timestamps or absolute workspace paths enter the artifacts. Acceptance IDs are stable for the same canonical record key at this pin; byte-offset-based occurrence IDs may change on a later SDK version.

## Artifacts and counts

| Artifact / inventory category | Coverage |
|---|---:|
| `sources.json` | All **460** tracked files, exact UTF-8 content and SHA-256 |
| Parsed Go files | **398**, including all build-constrained variants and tests |
| `packages` | **74** directory/package-name pairs, including examples and test helpers |
| `apis` | **5,859** declarations/members, public and internal |
| Public-package exported declarations/members | **2,965** (see breakdown below) |
| Type aliases resolved | **205** |
| Exported variables linked to local target declarations | **101** |
| `tools` | **62** concrete definitions, dynamic adapters and promoted permission variants |
| Distinct literal tool names | **53**, not a count of simultaneously registered tools |
| `tool_parameters` | **191** top-level property occurrences from literal schema candidates |
| `capabilities` | **81**: 15 registry-family descriptors and 66 feature-structure fields |
| `registrations` | **128** registration/option calls and tool composite occurrences |
| `cli_flags` | **49** explicit flag registrations |
| `environment` | **133** Go environment/helper call sites, including test-only sites |
| `configuration` / `defaults` | **253** relevant function definitions / **118** configuration literals |
| `tests` / `subtests` | **1,182** named entry points / **261** static `.Run(...)` sites |
| `examples` | **57** files (also included in other applicable categories) |
| `evals_and_automation` | **23** eval, script, workflow and Makefile artifacts |
| `artifacts` | **62** non-Go files |
| `os_constraints` / `os_runtime_checks` | **31** build-comment occurrences / **30** runtime GOOS comparison expressions |
| `verification_groups` | **88** concrete proposed Rust module groups |
| Total individually mapped records | **9,142** |
| Verification groups without directly assigned Go tests | **17**, each with an explicit coverage gap |
| Unmapped records / parse failures | **0 / 0** |

Public exported breakdown: 475 types, 1,157 fields, 644 methods, 292 functions, 191 constants, 111 variables, and 95 interface members. These are **syntactic declarations**, not a deduplicated, type-checked effective Go API surface. For example, methods on private concrete types in public packages can be exposed through constructors, and fields on a public type alias are found by following its target rather than being duplicated.

`manifest.json` is the authoritative generated count/hash summary. `inventory.schema.json` describes the common record contract; category-specific fields are documented here and retained without lossy normalization. `acceptance-contracts.json` defines what a future migration acceptance entails.

## How to consume the ledger

`inventory.json.records` contains named arrays. Every record has:

- `id`, `acceptance_id`, `rust_module`, and proposed `role_owner`;
- `implementation_status: not_implemented`, `verification_status: not_run`;
- `acceptance_contract: source-parity-v1` and `verification_group_id`.

**Acceptance IDs are future obligations, not executable tests.** A hash-shaped `SDK-…` ID uniquely names a source-backed migration obligation. It does not mean a Rust test exists, an existing Go test passed, or every symbol is covered. Baseline statuses deliberately do not promote inventory extraction checks into implementation verification. Role owners are responsibilities to assign, not assertions that a particular person has accepted work.

Follow `verification_group_id` to `verification_groups.source_test_ids`, then to `tests` for the **actual existing Go function name, source file, line range and signature**. Associations use proposed module ownership, not measured code coverage. Groups with no assigned Go tests have an explicit `coverage_gap`; they still have a concrete module and owner, never a “remaining” bucket. The memory-tool family links its actual memory feature-example tests because that package has no direct test file. Runtime feature switches link to the builder suite. Registry families route to the relevant implementation suite, rather than merely pointing everything to registry tests.

Examples of concrete existing regressions (all indexed; **not run here**):

| Capability | Proposed Rust module | Existing Go regression |
|---|---|---|
| Runtime feature selection | `sdk::runtime` | `pkg/agentsdk/runtime/builder_test.go: TestBuildAgentStrictModesDisabledByDefault` |
| Agent runner / handoff filtering | `sdk::agent` | `internal/agent/audit_fixes_test.go: TestHandoffInputFilterSeesCurrentTurnItems` |
| Durable fail-closed checkpoints | `sdk::durable` | `internal/agent/durable_checkpoint_test.go: TestRunnerDurableCheckpointFailureIsFailClosed` |
| Search and pagination | `sdk::tools::search` | `pkg/agentsdk/tools/search/pagination_test.go: TestGitignoreNegationCannotReincludeUnderExcludedParent` |
| Shell permission/network policy | `sdk::tools::shell` | `pkg/agentsdk/tools/shell/bash_test.go: TestBashNetworkPolicyMatchesPermissionMode` |
| GitHub issue tool | `sdk::tools::git` | `pkg/agentsdk/tools/git/github_test.go: TestCreateIssueToolCreatesMissingLabels` |
| Browser network confinement | `sdk::tools::browser` | `pkg/agentsdk/tools/browser/browser_test.go: TestExecutePublicOnlyFailsClosedBeforeBrowserLaunch` |
| Filesystem confinement | `sdk::tools::fs::confinement` | `pkg/agentsdk/tools/internal/pathutil/lifecycle_test.go: TestLifecycleOperationsRejectEscapeAndHardlink` |
| MCP tools and approval boundary | `sdk::mcp` | `pkg/agentsdk/mcp/breakglass_test.go: TestBreakGlassQuestionAndActions` |
| Custom memory stores/tools | `sdk::tools::memory` | `examples/features/memory/custom_store_test.go: TestCustomStoreCanBackMemoryToolExample` |
| Host config traversal rejection | `sdk::host::file_config` | `pkg/agentsdk/host/fileconfig/fileconfig_test.go: TestGetModeRejectsPathTraversalName` |
| OAuth refresh fallback | `sdk::providers::oauth` | `pkg/agentsdk/providers/oauth/copilot_token_source_test.go: TestCopilotTokenSourceFallsBackToUnexpiredTokenOnFailure` |
| Trace append concurrency | `sdk::trace_store` | `pkg/agentsdk/tracestore/trace_store_append_linux_test.go: TestFilesystemTraceStoreConcurrentAppendsLoseNothing` |

### API details

`signature` retains formatted Go declarations, receiver signatures, generic parameters, named/variadic parameters, return types, struct field types/tags, interface declarations and comments. All type declarations and their members are retained, even private supporting types, so public aliases and private returned implementations remain traceable. Exported functions, methods, constants and variables are indexed. Function bodies and all private implementation declarations remain in the exact source archive.

`member_ids` links directly declared type members; aliases expose `resolved_type_id` / `resolved_member_ids`. `target_id` / `target_signature` links local alias/variable/constant targets, including function-valued public façade exports. `effective_spec` and `const_group_index` preserve implicit const/iota declarations without pretending to evaluate them. Byte ranges are zero-based, half-open positions in the source archive; line numbers are one-based. Documentation comments are also preserved in the whole source file.

Explicit module/role routes live in `scripts/inventory/routes.json`. Public facade and internal core files have more specific routes for model, session, events, compaction, durable state, policy, tools, subagents, usage and telemetry. Aliases follow target ownership. Types and field names are not mechanically translated to Rust: the proposal is module-level, with Rust structs/enums/traits, `Result`, futures/streams and ownership decisions to be settled during implementation. No fake Rust signature is asserted to be equivalent to Go.

### Tools, schemas and registration

Each `tools` record contains concrete or inherited method declarations, including **complete Name, Description, InputSchema, Execute, approval, enablement and timeout source**, where declared. This captures defaults enforced in Execute as well as schema prose. `literal_schemas` are decoded **candidates**, not necessarily the schema returned in every branch. `tool_parameters` retains each top-level property's complete nested schema plus requiredness; nested properties remain inside that schema, not separate flattened records.

Definitions and registration occurrences are different things. Permission modes, runtime features, host injection, callbacks, MCP discovery and store availability gate registration. Source-backed `registrations`, `configuration`, feature fields and the complete registry/builder source preserve those conditions. A registration occurrence is **not** proof that a tool is enabled by default. `Features == nil` preserves the legacy surface; non-nil Features strictly opts into families. Memory is classified host-only in `RegistryCapabilities`. Project-state, skills, MCP, handoff and subagent tools also have construction paths outside the default registry.

Six tool types have no complete literal schema: `FunctionTool`, `policyToolWrapper`, `BashStartTool`, `BashTool`, `ReadOnlyBashTool`, and `WorkspaceWriteBashTool`. Their full schema expressions and forwarding paths are preserved. Bash interpolates environment-sensitive timeout descriptions. Dynamic MCP schemas and agent/handoff schemas may include a literal fallback **without being static overall**. Runtime names supplied by a host/MCP server cannot be enumerated from this checkout; those dynamic adapters have owned acceptance obligations, not an unassigned bucket. We do not synthesize a misleading universal “default registry” or invent schema defaults from prose.

### CLI, configuration, examples, evals and OS

CLI records include binding, Go flag type, help, registration default expression and the `cliConfig` initializer (including environment fallback expressions). Absent explicit fields use their actual Go zero values. Standard-library parser behavior, including one/two-hyphen flag spellings and implicit `-h`/`-help`, is not an additional SDK registration. Provider/model fallback, validation and post-parse normalization remain in the archived CLI source and configuration function records; registration defaults are not necessarily final effective runtime values.

Environment records cover Go `os.Getenv`, `os.LookupEnv`, and the CLI's `envOr`/`envInt`/`envBool` calls. Dynamic key expressions are retained; no environment values or credentials are read. Struct tags preserve JSON/YAML config fields. Configuration/default indexes are syntactic navigation aids, not an interpreter for all assignments. Host `~/.gratefulagents/{modes/*.yaml,agents/*.md}` loading, built-in chat/plan modes, sandbox/PTY processes, OAuth auth files, LSP executables, browser prerequisites and all other nonliteral behavior remain in full source.

All examples, integration tests, promptfoo smoke assertions, Terminal-Bench Python/shell adapters, adversarial audit fixtures, workflow live-test gates, module dependencies and Makefile commands are archived and assigned. Non-Go files are file-level obligations: their YAML/Python/shell flags, environment references, fixture cases and script defaults are **not separately AST-normalized**. Test counts include functions with Test/Benchmark/Fuzz/Example prefixes (including helpers such as TestMain), and `.Run` sites can be dynamic/table-driven; counts are not expanded test cases. Live examples can skip without credentials (`GRATEFUL_LIVE_TESTS=skip` versus `required`). No live or non-live SDK suite or eval was executed for this inventory.

Build constraints are inventoried across Linux/Unix/non-Unix variants without applying the host's build selection. Runtime GOOS comparisons are separately indexed. Linux confinement/openat behavior, macOS Seatbelt, Unix PTY/process lifecycle and non-Unix unsupported/fallback paths are not equivalent merely because declarations share names. File naming constraints and exact fallback implementations are retained in the source archive. No cross-compilation or OS runtime parity claim is made.

## Verified boundaries and remaining work

The generator's tests check grouped fields/tags, generic interfaces, aliases and member links, function-valued exports, iota inheritance, promoted tool methods, literal JSON validity, zero unmapped records, duplicate IDs, source fidelity and pinned breadth. Independent validation checks source hashes/ranges, record links, common schema shape, capability test associations and the manifest. Deterministic regeneration, tests, vet and build are the inventory acceptance evidence, **not SDK behavioral evidence**.

Limits requiring future owned acceptance work: type-checked external/dependency method sets; ambiguous promotion and embedding semantics; inferred Go constant values and generic instantiation; dynamic tool schemas/names and registration combinations; defaults evaluated under actual environments; symbol-level test coverage; migration-specific executable assertions; provider/live evals and supported-OS runtime matrices. Exact tracked sources prevent loss of evidence, but do not make these analyses complete. A future SDK version requires a new pin/version directory, route review and explicit count updates—no silent roll-forward.

## Attribution and licensing

Declarations, comments, schemas, docs, tests and the full-source archive derive from **Grateful Agents SDK v0.0.115** at the commit above. Upstream ships the GNU GPL version 3 license in `LICENSE`; that exact text is itself included in `sources.json`. These are attributed source-derived inventory artifacts, not a relicensing of upstream work. The repository's root license/notice copies are maintained by the coordinating migration work. Generator files carry `SPDX-License-Identifier: GPL-3.0-only`. Preserve source attribution and applicable GPL obligations when redistributing the excerpts or generated archive.

## Issue #11 audit overlay (append-only note)

[`issue-11-overlay.json`](issue-11-overlay.json) is an acceptance-ID-keyed,
post-generation audit overlay for the public-facade issue. It is **not** a
replacement for the generated `sdk-v0.0.115/` baseline and must never be used to
edit its `implementation_status` or `verification_status` fields. The overlay
records a pre-implementation snapshot: all 9,142 baseline IDs are either
`excluded` (251) or `unresolved` (8,891), with `verification_status: not_run`.
It contains no auto-verified record.

The overlay excludes `sdk_cli`, `sdk_evals`/`sdk_evals::*`, and the three
Terminal-Bench workflow records routed to `sdk::ci`. Its module API/test
reference map is a navigation aid only; it is explicitly separate from
per-record semantic closure. A future completion claim needs the acceptance
ID's implementation symbol, executable Rust test or reviewed artifact,
command/log, target/features, source-regression reference, and approved
divergence where applicable.

Use the deterministic helper without regenerating or mutating snapshots:

```sh
python3 scripts/issue11-ledger.py generate
python3 scripts/issue11-ledger.py check
```

The helper writes only `issue-11-overlay.json` and refuses paths under the
generated snapshot directory. The audit also observed source-pin drift: the
local `repos/sdk` checkout was v0.0.116 (`63afe2ed8cc5f13ca7469054f2c1cb812fcac801`),
whereas this ledger remains pinned to v0.0.115
(`1dc92b73900fac74dc357a938e4b5eee6392b418`). See
[`docs/verification/issue-11.md`](../../verification/issue-11.md) for evidence
limits and the parent-owned final-validation section.
