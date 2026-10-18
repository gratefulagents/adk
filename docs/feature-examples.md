# Offline Rust feature examples

[`crates/adk/examples/features.rs`](../crates/adk/examples/features.rs) is one executable
with **20 named scenarios**, matching the directory names under
`repos/sdk/examples/features`. Each scenario invokes Rust implementations and checks
observable results with assertions. Scripted models replace paid/network model calls;
these are not provider conformance or live-service tests.

## Run

Requires Rust 1.88+, a Unix host (Linux/macOS), `/bin/sh`, and a populated Cargo cache.
Fetch build dependencies once if necessary; **execution never reads provider credentials
or contacts a provider, MCP server, database service, or telemetry collector**.

```sh
cargo fetch --locked
sh scripts/check-feature-examples.sh
sh scripts/check-feature-examples.sh memory settings_routing
cargo run --offline --locked -p adk \
  --features builder,providers-runtime,execution,tools,mcp,project-state,observability \
  --example features -- --list
```

The POSIX shell script checks formatting, runs Clippy with warnings denied, builds the
executable, and runs all scenarios unless names are supplied. `all` also selects all.
Unknown names fail with exit status 2. An assertion failure fails the process; there
are no skip-as-success branches. A `PASS <name>` line is printed only after that
scenario's assertions finish. Temporary filesystem, subprocess, bundle, and child
scheduler resources are cleaned up by their owners. No worker CLI or eval adapter is
implemented here.

## Coverage map

This is **directory-level coverage**, not a claim that all 44 Go test entrypoints or
all APIs in their READMEs have exact Rust equivalents. The assertions and remaining
boundaries are deliberately explicit.

| SDK feature directory / scenario | Executed Rust behavior and assertions | Boundary |
| --- | --- | --- |
| `agent_runtime` | Two model turns execute a lookup once; the next request contains its result; final text, last agent, response count and accumulated usage match. One-turn exhaustion retains partial responses and reports `MaxTurns`. Host-owned autonomous progress tracking consumes actual new items and reports one tool call without a breaker. | Uses injected models; no automatic continuation loop, live backend or dynamic instruction callback. |
| `model_abstraction` | Custom `Model`/`StreamingModel` implementations behind `Routes`; default route resolution, unknown-prefix rejection, first-prefix stripping while preserving `vendor/model`; unused backend is not invoked. | Models are deterministic fixtures. |
| `providers` | Real request encoders for Chat Completions, Responses and Anthropic preserve model, instructions and user content. Chat/Anthropic response decoders agree on content and token usage. Provider protocol defaults and incompatible auth modes are checked. | No HTTP/auth refresh/live calls; the six dedicated live-provider Go tests remain unverified. |
| `tools` | A real runner pauses on exact-call approval without executing the tool, then resumes once, executes once and returns the final answer. | Does not claim all timeout/wrapper combinations. |
| `tools_registry` | Build an explicitly selected `ReadFile` bundle, read a real temporary file, close the bundle and prove the retained tool handle is cancelled. | Tests a filesystem capability, not every canonical registry tool. |
| `mcp` | Adapt a host `ToolManager` descriptor into a native tool; invoke it, preserve text output, reject non-object input before manager dispatch. | In-process manager fixture, like descriptor-adapter examples; no stdio/HTTP/TLS transport assertion. |
| `sandbox` | Run bounded shell output in a temporary workspace; assert successful exit, exact capture limit/truncation, fixed PATH and absence of inherited provider/cloud secrets. | Explicit `Backend::Local` is **not OS isolation**. This does not claim bubblewrap/Seatbelt enforcement. |
| `chatloop` | `run_go_chat` resolves a deferred approval through a gate exactly once; a second invocation receives stored first-turn history without replaying the tool. | In-memory history is host-owned, not a new public ChatLoop/session persistence adapter. |
| `handoffs_subagents` | Shared host state gates transfer exposure, an awaited callback receives arguments, and a custom filter retains message provenance while leaving emitted handoff items intact. Disabling the same gate hides the tool on a subsequent run. Transfer changes `last_agent`; explicit child-host injection automatically builds a generic child/scheduler and closes the owned session. A separate host-owned automatic session survives two independently closed/rebuilt turn bundles with the same scheduler and retained task history. A manual scheduler plus `RunnerChildExecutor` and `AgentAsTool` also executes a nested runner and delivers evidence before shutdown. | No distributed workers or external task queues. |
| `guardrails` | Safe text passes secret detection; synthetic credential text returns `Guardrail` without leaking the secret; authorized subprocess output is blocked by the execution facade. Public built-in tool guards redact a synthetic output and trip on destructive shell input without executing it. | The scenario covers built-in helpers, not every custom callback combination or shell language construct; see below. |
| `structured_output` | Facade builder supplies the named schema and owns bundle closure. Valid JSON is returned parsed, including schema-invalid JSON without a warning. Invalid JSON remains raw text and emits exactly one `OutputValidationFailed` observation. | Validation is observational, **not fail-closed**; see below. |
| `streaming` | A real pull-stream runner delivers two ordered text deltas, exactly one terminal event, and the same complete final answer. | Scripted stream, not an SSE/HTTP integration test. |
| `context_compaction` | Real deterministic local compaction shrinks history, generates a summary, preserves the first goal and latest question; disabled compaction leaves history unchanged. | No paid LLM summary or native provider compaction request. |
| `settings_routing` | Public `Builder` applies mode model/settings/instructions and read-only access; a role override selects a different model and verbosity. Assert actual dispatched requests and owned-session closure. | Uses host-supplied mode snapshots rather than file loading. |
| `observability` | Real `Observability` hooks plus `ObservedHost` count model attempts, tools and tokens; JSONL survives seven-byte chunk decoding with consecutive sequence numbers; terminal record is `done`; metadata capture hides answer content. | Native event schema, not Go trace/event schema parity or a remote OTel collector. |
| `errors_retries` | Transient provider failure retries once and succeeds without charging a failed response; pre-cancelled context prevents any model call. | No network backoff integration. |
| `costs` | Host-selected pricing produces an exact usage observation; an exhausted monetary budget reports `Guardrail` before tool execution. | Example monetary units, not live prices; reported usage limits cannot preempt provider spend. |
| `policy` | Read-only denies mutation; writable access still requires tool approval; denylist and exact-name allowlist win; access clamping cannot escalate authority. | Policy decisions do not themselves provide process isolation. |
| `memory` | Custom auditing `Store` wrapper, namespace store/search/delete and tag/phrase ranking; wrong-namespace deletion fails. Inject the custom store into the real `Memory` tool and assert its operation counts; exercise store/search/delete, source/repo metadata and host-bound namespace despite model-supplied namespace. | This uses `project_state::memory::Store`, the namespace SDK-memory contract, **not project-state task/memory APIs**. No PostgreSQL/pgvector service. |
| `tracestore` | Native private event store persists two ordered, decodable JSONL records, hides raw content, and rejects traversal/reopening. On Linux, the distinct SDK-compatible category store round-trips metadata and a score document, appends ordered LLM records, updates finish time, filters runs by candidate and start time, and persists owned runner spans with private files and explicit closure. | Category-store mutation requires Linux `openat2`; it is not executed on macOS. Score persistence is not an evaluation adapter. |

## Genuine gaps and semantic differences

- **Guardrails:** public runtime `Guardrail` implementations support agent-input,
  agent-output, tool-input and tool-output checks with typed results; the example
  focuses on secret detection/execution and built-in tool guards. Builder opt-in,
  custom ordering, durable identities and independent SDK fixture comparisons are
  tested separately. Built-in shell checks are a bounded, fail-closed syntax
  policy, not an OS sandbox or full shell interpreter. See
  [runtime builder](runtime-builder.md) for defaults and semantic limits.
- **Structured output:** `Runner::validate_output` reports JSON syntax/custom-parser
  failures through `RunHooks`, then returns parsed JSON or raw text. The executable asserts
  this current contract. Applications needing fail-closed validation must not assume
  the schema alone blocks the answer.
- **Trace persistence:** `observability::FilesystemTraceStore` writes the native
  observation schema. On Linux, the same scenario separately exercises
  `tracestore::FilesystemTraceStore` and its `llm_calls.jsonl`, `metadata.json`,
  `score.json`, finish-time update and filtered run listing, matching the pinned
  SDK trace-store example. The owned runner tracing path asserts trace start/end,
  completed session status and zero writer errors. Unknown history provenance is
  not inferred; normalized provider snapshots have separate, bounded fixture
  coverage rather than a full parity claim. See [trace storage](trace-store.md).
  No evaluation or Terminal-Bench execution is introduced.
- **Memory custom backends:** the host-injected store and namespace-bound tool are
  exercised; database persistence and vector retrieval are not. A production custom
  store must implement the namespace `Store` contract, not project-state recall.
- **Provider and transport coverage:** offline wire conversion does not prove
  credential refresh, service behavior, provider SSE transport, remote MCP security,
  or external observability export. Live examples remain separate work.

## Reused implementation patterns

The runner/approval/usage fixtures follow `adk-runtime/tests/runner.rs`; nested agents
follow `adk-runtime/tests/subagent_integration.rs`; local compaction follows
`adk-runtime/tests/local_compaction.rs`. Other patterns come from
`adk/tests/execution.rs`, `adk-tools/tests/bundle.rs`,
`adk-mcp/tests/tool_adapter.rs`, and `adk-sandbox/tests/lifecycle.rs`.
The example deliberately imports public facade modules and owns all host fixtures.

## Context-compaction helper example

The `context_compaction` scenario includes the original SDK example's24 repeated
assistant messages and explicit100-token trigger/60-token target, four recent
items, one initial user message and three summary bullets. It asserts compaction
runs, both history length and token estimate shrink, the generated summary contains
`[COMPACTED HISTORY SUMMARY]`, and the `gpt-5.6-sol` default trigger exceeds its
target. It also retains the richer first-goal/latest-question and disabled-policy
checks above. Run only this example with:

```sh
cargo run --offline --locked -p adk \
  --features builder,execution,mcp,project-state,observability \
  --example features -- context_compaction
```

The source README's helper usage maps to these native APIs:

- `DefaultCompactionConfig()` → `LocalCompactionPolicy::default()`.
- `RunConfig.CompactionConfig` → `RunnerConfig.local_compaction`; use
  `LocalCompactionPolicy { enabled: true, ..Default::default() }` for normal runs.
- `CompactionRecorder` / `CompactionFailureReporter` → `RunnerConfig.hooks` with
  `RunHooks::observe` handling `Observation::CompactionStarted`, `Compacted` and
  `CompactionFailed`. Hooks are awaited and may fail closed; they must redact
  sensitive summary/history data before forwarding it.
- `MaybeCompactRunItems` → `compaction::compact_for_request` for owned stored history.
- `CompactionDefaultsForModel(model)` → `LocalCompactionPolicy::for_model(model)`.
- `ExtractCompactionSummary(items)` → `compaction::extract_summary(items)`.

These types/functions are available through `adk::runtime` and its `compaction`
module. This is a reviewed runnable-documentation counterpart, not byte-identical
Go Markdown or a claim of live LLM/provider compaction execution. Native runner
regressions separately verify proactive compaction and failure observations.

## Settings-routing and canonical tool-registry examples

Run either original-example counterpart independently:

```sh
cargo run --offline --locked -p adk \
  --features builder,execution,mcp,project-state,observability \
  --example features -- settings_routing
cargo run --offline --locked -p adk \
  --features builder,execution,mcp,project-state,observability \
  --example features -- tools_registry
```

`settings_routing` uses `adk::codec::dto::ModeRouting` and `ModeRoleRouting` with
`adk::codec::mode::{resolve_mode_routing, resolve_role_mode_routing}`. It reproduces
all six original assertions: Anthropic medium resolves to claude-sonnet-4-6 with
high reasoning/low verbosity, while the writer's small override resolves to
claude-haiku-4-5 with low reasoning/high verbosity. No network call is made.
The resolution's `model_settings` supplies provider settings from mode labels,
replacing the README's separate ModeReasoningSettings/ModeTextVerbositySettings/
ModeRoutingSettings usage. Set just the desired reasoning/verbosity fields on a
ModeRouting to obtain those individual settings. Native model settings merging is
also shown with `adk::codec::snapshots::ModelSettings`; the same scenario retains
builder-driven mocked requests and explicit session shutdown.

`tools_registry` constructs `adk::tools::Registry::build` with the canonical Write
feature and `AccessMode::WorkspaceWrite`, retrieves `get("Write")`, executes the
original file_path/content arguments, and asserts notes.txt contains exactly hello.
Selection under the configured access mode replaces the source IsEnabled check;
the host supplies the working directory and execution policy in ToolContext.
The registry does not grant permission merely because a tool is registered.
The existing ReadFile bundle and post-close rejection checks remain. These are
runnable documentation/example counterparts, not closure of every registry tool,
all routing algorithms, live provider behavior or byte-identical Go prose.

## Trace-store and memory example assertions

Run `tracestore` or `memory` using the same Cargo command above and the desired
scenario name. On Linux, `tracestore` exercises the original category-store example
through `adk::tracestore::FilesystemTraceStore`: create metadata, append two
llm_calls NDJSON records, round-trip Score, update the finish timestamp, and list
all runs or filter by candidate. Deterministic IDs/timestamps replace the original
wall clock; the token counts42/58 and all score metrics are unchanged. It additionally
verifies schema-2 writer/runtime integration, private permissions, redaction and
invalid-path rejection. `adk::observability::FilesystemTraceStore` is a separate
native event store; do not confuse it with the compatible category-store API.
The compatible category-store portion is Linux-only; non-Linux execution of the
scenario does not verify that portion. The explicit original-example test is
therefore Linux-gated too.

`memory` reproduces the original SSL-reminder/deploy-guide fixture with
`adk::project_state::memory::InMemoryStore`: nonnil UUID, namespace isolation,
tag filtering and deletion. `NoopEmbedder::default()` via the `Embedder` trait
returns the same nonempty vector on repeated input. The application-owned
`AuditedMemory` implements `Store`; a canonical Memory registry tool supplied by
`adk::tools::memory::tool` stores the original smoke-test reminder and searches it
by tag/limit. Assertions verify tool-configured namespace/source run/repo metadata,
the returned memory ID and exactly one backend store/search call. This is explicit
host injection, not an automatically provisioned database. These tests do not
claim the separate Postgres/pgvector documentation sketch or a live vector service.

The `policy` scenario now also reproduces the original offline TestPolicyExample
using `adk::codec::policy` projections before exercising native fail-closed tool
policy. This does not claim the separate live TestRunnerToolPolicyExample; see
[execution security](execution-security.md#legacy-sdk-policy-projections) for the
legacy-default/native-authorization distinction.

## Original offline retry examples

`errors_retries` now runs the original three assertions: provider advice retries
one temporary failure and returns recovered after exactly two calls; an explicit
one-retry policy with zero delay returns policy recovered; the standalone legacy
default policy has three retries and attempt2 produces four seconds. Both model
runs use a two-turn limit. These original examples use fakes too: live providers
cannot deterministically inject their required failures.

`adk::codec::retry::default_retry_policy()` and `RetryPolicy::delay_for_attempt`
expose the standalone legacy arithmetic. The result is signed Go-duration
nanoseconds, including pinned64-bit overflow behavior, not a safe sleep Duration.
Rust Default constructs the source zero value; the explicit function constructs
the SDK's suggested3/1000ms/30000ms/2x settings. Partial backoff normalization,
uncapped first delay and fractional multiplier behavior are tested independently.
These are value helpers, not a Go JSON codec or a native runner configuration
adapter. In particular native RunnerConfig still defaults to zero policy retries;
provider advice is a separate bounded retry mechanism. Native retry delays remain
host-bounded and the native policy does not expose the legacy custom multiplier.

Implement `Model::retry_advice` for provider-directed recovery. Set
`RunnerConfig::retry` for policy retries, or `RunnerConfig::error_handler` with a
`ModelErrorHandler` for Retry/Continue/Abort decisions. Native errors use
`RunError.error.info.category` (including MaxTurns, Guardrail and DeadlineExceeded)
and preserve partial run state, rather than Go errors.As concrete-type matching.
The scenario additionally verifies that a cancelled run never invokes the model.

## Original provider configuration and MCP assertions

`providers` constructs OpenAI chat-completions, Anthropic and OpenRouter model
bindings with the original endpoints/model names via `providers::factory::RouteSpec`.
It checks provider identities, runner construction, chat support for gpt-4.1-mini
and rejection of DEFAULT_CHAT_MINI_MODEL. Host-owned credential/refresh doubles
panic if accessed: no key, network request or live service is used. This does not
verify the separate live provider examples.

`mcp` now uses the original mcp__notes__lookup name, query=sdk and note found result.
On Unix it also writes a private .mcp.json, loads a ConfigSnapshot with nonempty
hash/absolute path/echo server, verifies unchanged bytes, rejects a trailing edit,
and restores/reloads the original command. No MCP process is spawned. Native
ConfigSnapshot preserves bounded/no-follow validation and returns ConfigChanged
rather than matching Go error prose. Non-Unix safe snapshots remain unsupported;
the original snapshot example regression is explicitly Unix-gated.

`adk::codec::text::sanitize_mcp_display` provides the explicit legacy approval-UI
byte projection. It strips nonprintable Go Unicode scalars, flattens control
whitespace and adds the original provenance tag. The original hostile description
and hostile server-name examples verify preserved legitimate words and removed
controls. The512-byte body cut can split UTF-8, so the result is Vec<u8>, not String;
UI adapters must choose their own decoding policy. Input strings are valid UTF-8.
The source server tag is not length-bounded and is not an authenticated identity.
Native `mcp::tools::sanitize_description` remains a separate bounded, quoted,
untrusted-description formatter; it is not replaced or weakened by this projection.
Remote MCP client/server policy features and their README are not closed by these
local examples.

The separate `codec::text::sanitize_mcp_tool_description` retains the SDK's
1024-byte body cutoff, quoted names, unnamed/empty-body fallbacks and explicit
untrusted-text framing as bytes. Names remain unbounded in this legacy projection.
It too is separate from the stricter native MCP formatter. Both pure helpers are
checked against160 independently executed cases and every Unicode scalar.

## Original local HTTP streaming and read-only registry checks

`streaming` now additionally runs a real provider SSE stream against an owned
loopback TCP server. It sends the original gpt-4.1-mini/Say hi./hi/32-token request,
consumes the original hello from /test server/usage/DONE sequence, checks the
assembled text and output usage4, and verifies the outgoing model, stream flag
and /chat/completions path. A dummy host credential is used only for that local
endpoint. Client and server futures are joined under one five-second bound; no
server task is detached. This is the original offline httptest counterpart, not
evidence for the separate credential-dependent streaming examples. Existing
native runner event/finish assertions remain in the scenario.

`tools` also selects the original five canonical registry tools. ReadOnly exposes
read_file/glob/grep and excludes Write/Edit; WorkspaceWrite exposes both mutating
tools as a sanity check. Native feature selection remains explicit rather than
implicitly enabling every legacy registry tool. The existing mocked approval/
resume assertions remain; this does not claim the original live FunctionTool or
live approval-wrapper test has run.

### Sandbox application environment

The sandbox example executes the original local `printf hello` case and checks
`CUSTOM=value` alongside absent ambient OpenAI/AWS/database credentials. Rust uses
explicit host-owned `sandbox::Config::application_env` rather than copying the
host environment or permitting arbitrary model-request overrides. Grants cannot
replace policy-owned variables or Git settings; values are omitted from Config
Debug but are visible to children (and potentially backend argv), so this is not
a credential-delivery API. Local execution still requires FullAccess/network Allow.
The lifecycle subprocess regression injects dummy ambient secrets into a separate
parent process and checks the actual child's environment without global mutation.
