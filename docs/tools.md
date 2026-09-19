# Built-in tool registry port — implementation status: draft

`adk-tools` is opt-in through `adk`'s `tools` feature. Its 51 access-specific manifest entries cover all 46 distinct names in the pinned SDK's `pkg/agentsdk/tools` tree. Every name now has either a built-in implementation or a trusted host adapter; the owning host supplies the resources and authority for adapters. This is an implementation-status document, **not acceptance evidence**. Full CI and the security review are still pending, so issue #7 remains **draft** and must not be treated as accepted or closed.

## Catalog, selection, and construction

Definitions derive from the v0.0.115 ledger (GPL-3.0-only) and retain their static schemas, descriptions, read-only and approval flags, source type, and acceptance ID. `capabilities()` and `select(&Config)` select from that manifest deterministically.

`Features::Strict` accepts exact tool-producing feature paths (`Grep`, `Signals.Finish`, `ProjectState.TaskTools`, and `ExtraTools`); an empty set enables nothing. `Features::Legacy` preserves the legacy tool/subagent and default/web/signal/async/state switches. Analyzer configuration such as `VisionAnalyzer` is not a tool-producing feature.

Selection applies access variants, explicit-name filtering, mutable-tool grants, remote-write policy, the Browser private-network gate, the Terminal full-access gate, and async-shell write gating. Browser's read-only contract excludes screenshots. WebFetch does not inherit Browser's unrestricted-network gate. Selection is an eligibility matrix, not execution authorization.

`Registry::build` matches supplied implementations to selected SDK contracts and rejects duplicate names, unknown built-ins, contract drift, and missing selected implementations. Built-ins cover the in-process families. Trusted host adapters provide plan and memory stores, skills catalogs, Git/GitHub command and repository hosts, and configured shell, LSP, and browser resources. A selected dependency that an owned `ToolBundle` lacks fails closed with the deterministic missing-name list; the bundle does not discover an executable, credential, store, or identity.

`Registry::build_with_extra_tools` composes host-supplied extensions explicitly after canonical built-ins. It enables them only through `ExtraTools` (or the legacy tool/subagent switch), filters them through `allowed_names`, rejects duplicate names, and does not allow extensions to bypass preparation or dispatch authorization.

`Registry::prepare` returns `PreparedTools`: the model-visible tools after access adaptation and policy filtering, together with the matching `ToolPolicy`. `ToolBundle::prepared()` wraps those handles with owner-lifetime cancellation and freezes `allowed_tools` to the prepared names. Registration and preparation never grant policy or approval permission.

Preparing a full-access Write/Edit under workspace-write policy narrows its filesystem implementation; a mutation allowlist never waives that confinement. Exact-name exceptions under read-only policy instead retain the host-configured implementation, as in the SDK. Bash honors those explicit exceptions without raising its configured access ceiling. These are dispatch-level guarantees, not just changes to model-visible metadata.

## Implemented tool families

| Family | Implementation and boundary |
| --- | --- |
| Workspace search and files | `read_file`, `list_files`, `glob`, `grep`, `Write`, `Edit`, `Move`, `Delete`, and `ApplyPatch` use the secure workspace layer. Search has deterministic text/JSON output, query-bound pagination, include/exclude/default-directory and gitignore controls. Mutation uses prevalidation and scoped rollback/quarantine where applicable. |
| Shell and terminal | `Bash`, `BashStart`, `BashPoll`, `BashKill`, and `Terminal` run through `adk_sandbox::Executor::start_session`; the tools do not spawn commands directly. The host owns the sandbox configuration and must keep the bundle alive, then call `close().await` before stopping Tokio. |
| LSP | `LSP` uses configured, host-trusted server definitions and the confined bidirectional session API. It implements the read-only operation set, aliases, bounded framing, UTF-16 result adaptation, workspace-scoped routing, cancellation, and explicit close. It does not guess a server executable or use PATH discovery. |
| Browser and vision | `Browser` and `AnalyzeImage` are implemented through configured adapters. Browser runs only the configured absolute executable and uses the sandbox/scratch path for captures; it is not a claim that native Chrome is installed, that a live browser environment has been certified, or that arbitrary browser networking is safe. |
| Web and signals | `WebFetch`, `think`, `AskUserQuestion`, `present_plan`, and `finish` implement their catalogued contracts. WebFetch retains its pinned-address SSRF controls, redirect revalidation, bounded body/pagination, HTML extraction, and cancellation cleanup. |
| Durable state and plans | The 15 project-state tools, `Memory`, `get_plan`, `save_plan`, and `prime_context` use trusted host stores and identity. The adapters do not invent persistence, tenant/run identity, or authorization. |
| Skills | `skill_search`, `skill_install`, and `skill_list_installed` use a host-owned catalog, fallback workspace, and environment-availability snapshot. Installation updates `.mcp.json` but does not start servers. |
| Git and GitHub | `attach_repository`, `create_github_issue`, and `create_pull_request` use host-supplied confined command, repository, credential, attribution, and optional artifact adapters. They do not run a default Git/GH command path, discover credentials, or establish live GitHub behavior. |

All external effects still pass the caller's executor policy and approval checks. A catalogued lifecycle/control tool is not an authorization bypass.

## Confinement, shell, and lifecycle guarantees

### Filesystem policy

On Linux, workspace operations use descriptor-relative `openat2` resolution with `BENEATH | NO_SYMLINKS`. On macOS, the confined filesystem pins the workspace descriptor and walks one component at a time with `openat` and `NOFOLLOW`; an ancestor swap cannot redirect a later lookup through a symlink. Unsupported platforms fail closed rather than use an unconstrained fallback.

The workspace reader accepts regular, single-link files only. It rejects symlinks and hard-linked reads. Mutating tools reject symlinks, special files, traversal, and operation-specific unsafe links; `Move` never overwrites its destination, and `Delete` accepts only regular files or empty directories. Write replacement can intentionally sever an existing destination hard link rather than mutate its aliases; operations that require an existing single-link file reject a hard link. These policies are stricter confinement behavior, not a claim of exhaustive SDK diagnostic parity.

### Hardened shell behavior

Shell tools invoke `/bin/bash --noprofile --norc` through the sandbox. Full access requires an explicitly configured Local backend. Restricted variants do not fall back to Local; when remote Git writes are disabled, the tool fails closed unless the command sandbox enforces filesystem confinement.

Restricted-mode classification deliberately tightens the SDK oracle. It rejects dynamic or compound shell syntax, indirect Git execution, configuration aliases, and pushes with implicit or protected destinations even when filesystem enforcement exists. In enforced restricted mode it also treats newline statements as separators and inspects literal `printf`/`echo | shell` bodies. This classifier is defense in depth, not a shell interpreter, program allowlist, or OS containment boundary: commands such as build tools, interpreters, hooks, and package managers still require sandbox policy.

Host-selected executable dependencies outside standard system directories require explicit `adk_sandbox::Config::runtime_roots`. These are canonical, read-only grants, never model arguments, and may not overlap the workspace or name broad filesystem roots. Linux binds only those directories; Seatbelt uses parameterized read rules. For Apple Git/Python launchers, the host can add `macos_developer_toolchain_root()`; no broad `/Applications` or home-directory grant is inferred. Disabled-remote shell execution additionally masks discovered repository credential configuration, and subprocess output is screened for recognizable secrets.

### Asynchronous ownership

`Executor::start_session` supplies a bidirectional `ProcessSession` for pipes and PTYs. Input is acknowledged in bounded chunks; cancelling an input write can leave a prefix written. Output is bounded between polls; loss is reported as `truncated`, and protocol consumers must treat truncation as fatal.

`ProcessSession::ready()` waits for backend setup and child spawn, not exit. `wait()` closes any remaining input and awaits cleanup; `cancel_and_wait()` requests cancellation and awaits it. Dropping a session requests cancellation. `ToolBundle::close()` cancels owner-scoped tool calls and awaits owned shell/LSP/browser cleanup, including abandoned calls. Browser supervisors and LSP driver completions remain owned independently of request futures. Dropping the bundle requests cancellation but cannot itself await cleanup. In all cases, keep the Tokio runtime alive for TERM/grace/KILL, process-group cleanup, and direct-child reaping. Cancellation cannot undo effects that completed before it was observed.

Injected Git dependencies remain host-owned. Retain the typed `git_host::ExecutorCommandRunner`, stop tool dispatch, then call its `close().await` before shutting down Tokio. It joins active and abandoned commands through reaping; clone operations retain their failure cleanup in the same owned task, so close also awaits removal of an incomplete clone. Custom `CommandRunner` implementations must uphold that cancellation/cleanup contract. A completed external commit, push, issue or PR cannot be undone by cancellation.

## Verification evidence

| Evidence | Scope |
| --- | --- |
| `crates/adk-tools/src/manifest.json` | 51 access-specific entries and 46 distinct tool names. |
| `fixtures/tools/registry-matrix.json`, `crates/adk-tools/tests/registry_matrix.rs` | 3,768 raw pinned-SDK feature/access matrix cases. |
| `crates/adk-tools/tests/registry.rs`, `bundle.rs`, `crates/adk/tests/tool_runtime.rs` | Contract matching, explicit ExtraTools composition, `PreparedTools` adaptation, missing host dependencies, owner cancellation, and close/drop invalidation. |
| `fixtures/tools/{search,lifecycle,skills,signals,web,patch,browser,vision,lsp-cases,git-cases}.json` and their tool tests | Differential fixtures and per-family behavior, malformed input, policy, cancellation, and lifecycle cases. Fixture breadth is evidence, not full acceptance. |
| `crates/adk-tools/tests/{shell,shell_security,terminal,lsp,browser,git,git_host,macos_filesystem}.rs` | Sandboxed session behavior, hardened shell policy, terminal/LSP lifecycle, configured-adapter behavior, Git host boundaries, and macOS filesystem confinement. No test establishes native Chrome or live GitHub operation. |
| `crates/adk-sandbox/src/scratch_tests.rs`, `crates/adk-sandbox/tests/lifecycle.rs` | Scratch-directory ownership and session/process cleanup behavior. |

Verify that the generated catalog still matches its pinned ledger from the repository root:

```sh
python3 scripts/tool-manifest.py --check
```

On 2026-09-19, implementation head `728f169aca907547005279048a0cb226d94b9136` passed all 20 push/PR check records on pinned Rust 1.88.0. The workspace all-feature/all-target run passed **582 tests, zero failures, two ignored helpers across 77 targets**. Required enforcing Linux and macOS runs each passed **250 tests**, plus three public execution-feature tests. Windows passed **90 tests**, including the complete project-state suite and unsupported-backend checks. Formatting, strict Clippy, rustdoc, doctests, dependency checks, purity, manifest and replay checks passed. See [the coverage audit](tool-coverage.md#native-acceptance-verification-2026-09-19) for run links, fixture counts and evidence boundaries.

Local native-enforcement probes still skip because this worker cannot provide Bubblewrap's required `/proc` files. Those local results are not containment evidence; the successful native CI jobs set `ADK_REQUIRE_SANDBOX=1`, which makes an unavailable backend a failure.

## Acceptance status

**Implemented and regression-verified for all 46 pinned SDK names / 51 access variants; ready for maintainer review.** This includes runtime composition, state integration, independent schema comparisons, deterministic results, security and lifecycle regressions. Platform-specific tools outside the SDK registry remain outside this issue. Native Chrome rendering, live GitHub service calls and real third-party analyzer operation are not claimed: those are explicit host dependencies, tested through their adapter contracts. Windows has fail-closed behavior for unsupported confinement, not a new enforcing process/filesystem backend.
