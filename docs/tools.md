# Built-in tool registry port — incomplete parity

`adk-tools` is opt-in through `adk`'s `tools` feature. It currently provides a source-backed contract catalog, deterministic selection/composition, signal tools, host-backed plan tools, and integration with the existing project-state implementation. **This is not the complete tool implementation requested by issue #7. The issue must remain open.** No production runner automatically enables this registry.

## Canonical catalog and construction

`crates/adk-tools/src/manifest.json` contains 51 access-specific entries covering all 46 distinct names in the pinned SDK's `pkg/agentsdk/tools` tree. Definitions derive from the v0.0.115 ledger (GPL-3.0-only); they preserve static schemas, descriptions, read-only and approval flags. Source type and acceptance ID accompany each entry. This includes the 15 registry families plus independently constructed skills and project-state tools. Dynamic MCP/subagent tools and platform-specific Git/GitHub tools belong to their separate integrations.

Regenerate/check the catalog:

```sh
python3 scripts/tool-manifest.py
python3 scripts/tool-manifest.py --check
```

`capabilities()` and `select(&Config)` use that catalog. `Features::Strict` accepts exact tool-producing feature paths (`Grep`, `Signals.Finish`, `ProjectState.TaskTools`, etc.); an empty set enables nothing. `Features::Legacy` models the legacy tool/subagent enable flags and default/web/signal/async/state toggles. Features which configure analyzers rather than select tools, such as `VisionAnalyzer`, are not accepted as tool-producing switches.

Selection applies access variants, exact-name filtering, mutating-tool grants, the Browser private-network gate, Terminal full-access/remote-write gate and async-shell write gate. Browser's read-only contract excludes screenshot. WebFetch is not gated behind Browser's unrestricted-network opt-in. A selection is only a list of eligible contracts, **not evidence of implementation or execution permission**.

`Registry::build` receives real `Arc<dyn Tool>` implementations. It rejects duplicate names, unknown built-in names, static schema/description/access/control/timeout drift, and missing selected runtime implementations. Host-only entries describe optional supplied implementations and do not fabricate stores or identity. Dynamic Bash schemas are explicitly absent in the catalog and construction remains unavailable until their environment-dependent schema generator is ported, even if a caller supplies a tool of that name. There are no registered “not implemented” tools. This built-in registry does not yet provide the SDK's arbitrary extra-tool extension path.

Every selected call still needs the existing executor's policy and approval checks. In particular, cataloguing a lifecycle/control tool is not an authorization bypass: a host executing `save_plan` in read-only mode must permit that name in its execution policy. Injected adapters and their external resources remain host-owned; this implementation does not claim bundle teardown for unported shell/LSP/browser resources.

## Implemented behavior

- `read_file`, `list_files`, `glob`, `grep`: bounded reads and lines, deterministic text/JSON results, query-bound cursor pagination, include/exclude/default-directory and gitignore filtering, context and files/count modes. Linux descriptor-relative `openat2` reads reject symlinks and hard links; unsupported platforms fail closed. Go glob/regex compatibility is translated explicitly rather than using Rust defaults. Differential fixtures cover 56 cases and 80 paginated calls against the pinned SDK. This does not yet establish exhaustive regex or cross-platform parity.

- `Write`, `Edit` (Linux): workspace writes stage and sync a new file before descriptor-relative replacement, preserving permissions; edits require byte-exact unique matches unless `replace_all` is set. Edit reads are bounded to 5 MiB and result diffs to the baseline 256-match/8 KiB limits. Workspace Edit rejects hard links; Workspace Write replaces a hard-linked destination without changing its aliases. Full-access variants preserve direct-write inode semantics and allow paths outside the workspace, but reject special files rather than blocking on FIFOs/devices.
- `Move`, `Delete` (Linux): quarantine entries before inode validation, reject symlinks/hard links/special files, never overwrite move destinations, and restore quarantined entries on failure. Delete accepts only regular files or empty directories. Restoration conflicts are reported, not hidden or overwritten.
- `think`: validates a nonblank thought and returns the SDK acknowledgement.
- `AskUserQuestion`: plain-text or structured-choice result, including default freeform behavior. The SDK tool itself does not set `should_pause`; the port preserves this.
- `present_plan`: validates summary/actions, emits the baseline structured result, ignores unsupported action fields rather than granting a mode transition.
- `finish`: emits the baseline summary with `should_pause`; `signal::finish` optionally invokes a host `FinishSink`. A failed sink reports an error and does not claim completion.
- `plan::tools`: supplies `save_plan`/`get_plan` using a trusted host artifact-store/session identity. Includes missing-plan handling, error reporting, byte counts and the baseline 200-byte default summary rule. Store implementations own durable persistence, timestamp metadata and cancellation-aware I/O; tests use a recording store, not a new production artifact backend.
- `memory::tool`: supplies the separate host-only `Memory` tool using #9's namespace-store contract. Namespace, source run and repository metadata are host-owned. Store/search/list/delete actions preserve the SDK's empty-result messages and mutating classification; no store or identity is invented.
- `skills::tools` (Linux): supplies `skill_search`, `skill_install`, and `skill_list_installed` from a host-owned catalog, fallback workspace, and environment-name availability snapshot. Search preserves query precedence, ordering, and Unicode simple-case matching. Installation writes `.mcp.json` without starting servers, merges the baseline environment/tool/enabled restrictions, preserves other configured servers, and tightens permissions. Config reads reject hard links as an additional confinement restriction. The pinned SDK's default catalog is empty; passing an empty catalog preserves that behavior.
- `project_state_tools`: supplies all 15 existing #9 tools without changing their behavior. Registry integration tests exercise each name, restart persistence and read-only filtering against filesystem and SQLite backends.

## Verification and mapping

| Evidence | Scope |
| --- | --- |
| `crates/adk-tools/tests/registry.rs` | Strict-family isolation across all three access modes, unique names, legacy defaults, Browser/WebFetch policy distinction, Terminal/async/remote-write gates, exact-name filtering, duplicate/drift/missing implementation errors and all 15 state schemas. These are selection tests, not behavior tests for missing families. |
| `fixtures/tools/signals.json`, `crates/adk-tools/tests/signals.rs` | Deterministic signal results, malformed input, cancellation before execution, JSON escaping, no action-based mode transitions, host callback failures and plan artifact behavior. |
| `fixtures/tools/verify-signals.go` | Executes the JSON fixtures against actual SDK signal implementations. Run from `repos/sdk` at the pinned commit with `go run ../../fixtures/tools/verify-signals.go`. |
| `crates/adk-tools/tests/write.rs`, `edit.rs`, `lifecycle.rs` | Mutation success/errors, read-only selection, cancellation, modes/inode semantics, byte preservation, symlink/hard-link/special-file rejection, source restoration and concurrent parent swaps. |
| `fixtures/tools/lifecycle.json`, `skills.json`, `verify-lifecycle.go` | 95 pinned SDK comparisons of results and resulting filesystem trees: 77 filesystem cases plus 18 skills cases, including Unicode matching and exact installed config bytes. |
| `crates/adk-tools/tests/skills.rs` | Catalog search, install/reinstall hardening merges, environment availability, host fallback workspace, list ordering, malformed config, confinement, cancellation and read-only selection. |
| `crates/adk-tools/tests/state.rs` | All 15 tools through registry composition, both durable backends, trusted actor, restart and read-only surfaces. |
| Existing `adk-project-state` tests | Full underlying task/memory/error/security and Go result fixtures remain authoritative; they are not duplicated or bypassed here. |

Commands:

```sh
cargo test -p adk-tools -p adk-project-state
cargo clippy -p adk-tools --all-targets -- -D warnings
cargo check -p adk --features tools
python3 scripts/tool-manifest.py --check
```

## Remaining acceptance work (not waived)

Exhaustive search/regex and cross-platform parity; ApplyPatch parsing/prevalidation/rollback; exhaustive filesystem security-corpus coverage and platform-specific error diagnostics; foreground/background shell; Terminal; LSP; bounded SSRF-safe WebFetch; Browser and vision; repository attachment and GitHub; dynamic Bash schemas; arbitrary extra-tool composition; runtime auto-wiring; resource-owning bundle teardown; full feature/configuration-mode comparisons; per-tool happy/error/policy mappings and the full security corpus.

The current sandbox has no bidirectional stdio/PTY session API, preventing a safe LSP/Terminal adapter using its public executor surface. See [research](tools-research.md). That specific dependency gap does **not** explain away the other unfinished families, which remain implementation work. Do not close issue #7 or advertise SDK behavioral parity based on this catalog.
