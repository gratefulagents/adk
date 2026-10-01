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
| `agent_runtime` | Two model turns execute a lookup once; the next request contains its result; final text, last agent, response count and accumulated usage match. One-turn exhaustion retains partial responses and reports `MaxTurns`. | Uses injected models, not a live backend or dynamic instruction callback. |
| `model_abstraction` | Custom `Model`/`StreamingModel` implementations behind `Routes`; default route resolution, unknown-prefix rejection, first-prefix stripping while preserving `vendor/model`; unused backend is not invoked. | Models are deterministic fixtures. |
| `providers` | Real request encoders for Chat Completions, Responses and Anthropic preserve model, instructions and user content. Chat/Anthropic response decoders agree on content and token usage. Provider protocol defaults and incompatible auth modes are checked. | No HTTP/auth refresh/live calls; the six dedicated live-provider Go tests remain unverified. |
| `tools` | A real runner pauses on exact-call approval without executing the tool, then resumes once, executes once and returns the final answer. | Does not claim all timeout/wrapper combinations. |
| `tools_registry` | Build an explicitly selected `ReadFile` bundle, read a real temporary file, close the bundle and prove the retained tool handle is cancelled. | Tests a filesystem capability, not every canonical registry tool. |
| `mcp` | Adapt a host `ToolManager` descriptor into a native tool; invoke it, preserve text output, reject non-object input before manager dispatch. | In-process manager fixture, like descriptor-adapter examples; no stdio/HTTP/TLS transport assertion. |
| `sandbox` | Run bounded shell output in a temporary workspace; assert successful exit, exact capture limit/truncation, fixed PATH and absence of inherited provider/cloud secrets. | Explicit `Backend::Local` is **not OS isolation**. This does not claim bubblewrap/Seatbelt enforcement. |
| `chatloop` | `run_go_chat` resolves a deferred approval through a gate exactly once; a second invocation receives stored first-turn history without replaying the tool. | In-memory history is host-owned, not a new public ChatLoop/session persistence adapter. |
| `handoffs_subagents` | Transfer to a specialist changes `last_agent`; a real scheduler plus `RunnerChildExecutor` and `AgentAsTool` executes a nested runner, delivers child evidence to the parent, then shuts down. | No distributed workers or external task queues. |
| `guardrails` | Safe text passes secret detection; synthetic credential text returns `Guardrail` without leaking the secret; authorized subprocess output is blocked by the actual execution facade's output guardrail. | Generic Go input/output/tool callback guardrails and typed tripwire results are unavailable; see below. |
| `structured_output` | Valid JSON is returned as a parsed object; wrong-type JSON emits exactly one `OutputValidationFailed` observation, whereas valid output emits none. | Validation is observational, **not fail-closed**; see below. |
| `streaming` | A real pull-stream runner delivers two ordered text deltas, exactly one terminal event, and the same complete final answer. | Scripted stream, not an SSE/HTTP integration test. |
| `context_compaction` | Real deterministic local compaction shrinks history, generates a summary, preserves the first goal and latest question; disabled compaction leaves history unchanged. | No paid LLM summary or native provider compaction request. |
| `settings_routing` | Public `Builder` applies mode model/settings/instructions and read-only access; a role override selects a different model and verbosity. Assert actual dispatched requests and owned-session closure. | Uses host-supplied mode snapshots rather than file loading. |
| `observability` | Real `Observability` hooks plus `ObservedHost` count model attempts, tools and tokens; JSONL survives seven-byte chunk decoding with consecutive sequence numbers; terminal record is `done`; metadata capture hides answer content. | Native event schema, not Go trace/event schema parity or a remote OTel collector. |
| `errors_retries` | Transient provider failure retries once and succeeds without charging a failed response; pre-cancelled context prevents any model call. | No network backoff integration. |
| `costs` | Host-selected pricing produces an exact usage observation; an exhausted monetary budget reports `Guardrail` before tool execution. | Example monetary units, not live prices; reported usage limits cannot preempt provider spend. |
| `policy` | Read-only denies mutation; writable access still requires tool approval; denylist and exact-name allowlist win; access clamping cannot escalate authority. | Policy decisions do not themselves provide process isolation. |
| `memory` | Custom auditing `Store` wrapper, namespace store/search/delete and tag/phrase ranking; wrong-namespace deletion fails. Inject the custom store into the real `Memory` tool and assert its operation counts; exercise store/search/delete, source/repo metadata and host-bound namespace despite model-supplied namespace. | This uses `project_state::memory::Store`, the namespace SDK-memory contract, **not project-state task/memory APIs**. No PostgreSQL/pgvector service. |
| `tracestore` | Real private filesystem event store persists two ordered, decodable JSONL records, hides raw content, creates mode-0600 files, and rejects traversal and reopening an existing run. | Native event store lacks Go metadata/score/list-run APIs; see below. |

## Genuine gaps and semantic differences

- **Guardrail callbacks:** native secret detection/execution guardrails are real and
  exercised, but there is no equivalent public API for the Go example's arbitrary
  agent-input, agent-output, tool-input and tool-output callbacks with typed tripwire
  results. Secret scanning is not a substitute for those four interfaces.
- **Structured output:** `Runner::validate_output` reports schema/parser violations
  through `RunHooks`, then returns parsed JSON or raw text. The executable asserts
  this current contract. Applications needing fail-closed validation must not assume
  the schema alone blocks the answer.
- **Trace persistence:** `FilesystemTraceStore` writes the native observation schema,
  not Go `llm_calls.jsonl`/`metadata.json`/`score.json`. Go's `WriteScore`,
  `UpdateMetadataFinishedAt`, filtered `ListRuns`, and metadata round trips have no
  counterpart demonstrated here. No fake score/eval adapter is introduced.
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
