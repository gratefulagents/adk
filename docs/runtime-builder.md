# Native runtime builder

Enable `adk`'s **`builder`** Cargo feature. It composes `adk-runtime`,
`adk-providers` (including runtime integration), and `adk-tools`; it does not
require platform services, a worker CLI, or evaluation adapters. Cargo features
make implementations available. `builder::Features` selects what a particular
runtime is allowed to assemble.

## Construction

```rust,no_run
use adk::{builder::{Builder, Config, Features}, core::*, providers::factory::Kind};
use std::sync::Arc;

async fn build(
    context: &Context,
    model: Arc<dyn StreamingModel>,
) -> Result<adk::builder::Bundle, Error> {
    Builder::new(Config {
        model: "openai/small".into(),
        active_mode: Some("plan".into()),
        features: Some(Features {
            tools: ["ReadFile".into(), "Glob".into(), "Grep".into()].into(),
            mode_instructions: true,
            untrusted_tool_outputs: true,
            ..Default::default()
        }),
        ..Default::default()
    })
    .model("openai", Kind::OpenAi, model)?
    .build(context).await
}
```

`Builder::model` registers an injected streaming model, useful for offline tests
and host adapters. `Builder::route` takes the existing provider `RouteSpec`, an
explicit `CredentialStore`, and a `Refresh` implementation. It registers a real
provider without fetching credentials or making a network request. Each route
retains its own credential scope. Alternatively, supply a complete `Routes`
registry with `Builder::routes`. No registration is implicit: selecting an
unregistered default or fallback fails before tool construction.

The default route is inferred from explicit `default_provider`, the initial
model prefix, configured provider kind, then `openai`, matching the existing
provider factory. Supplying a `Routes` registry uses that registry's default.
Mode/role routing can select any registered prefix. First-slash routing preserves
opaque gateway model IDs: `gateway/anthropic/claude` reaches the gateway as
`anthropic/claude`. Aliases and omitted model names use the provider factory's
existing defaults. SDK fallback bindings remain separate from provider-specific
fallback settings.

`Builder::implementations` supplies canonical built-in implementations.
`Builder::extra_tools` supplies extensions, enabled only by `ExtraTools` or the
legacy tools/subagents switches. Built-in contract checking, duplicate rejection,
access adaptation, deny lists, and final dispatch policy all come from
`adk-tools`. Missing selected implementations are errors, not warnings or
model-visible placeholders. Optional host-only ExtraTools entries are not
invented: selecting ExtraTools does not require every possible host extension.

Configure lifecycle-owned shell/LSP/browser resources through
`Builder::shell(adk_sandbox::Config)`, `Builder::lsp(adk_tools::lsp::Config)`,
and `Builder::browser(adk_tools::browser::Config)`. LSP and browser setters are
available on Linux and macOS, matching the underlying tool bundle. No executable
or credential discovery occurs. These typed setters cannot replace the internal
tool builder or change its frozen feature selection and access policy; a resource
input cannot enable an unselected feature. Host-injected implementations
must follow the core no-detached-work contract; custom resources remain the
host's responsibility unless the underlying tool bundle explicitly owns them.
Project-state, MCP, and other trusted adapters can supply tool implementations;
this builder does not discover their stores or connections.

`runner_config` supplies existing native runtime hooks, compaction adapters,
budget limits, spill directories, and other supported runner configuration. The
builder controls work directory, selected subagent session, retry/compaction
activation, mutation approval, output trust wrapping, and model parallel-call
settings. It installs baseline provider cost accounting unless the host supplies
an estimator. Unknown prices retain the existing baseline zero-cost behavior;
they are not a reliable hard budget boundary.

## Strict selection versus legacy defaults

* `Config::features = None` uses `legacy_tools` and the three
  `enable_compaction`, `enable_retry`, `enable_approval` switches. Mode
  instructions/routing, parallel-call model requests, and untrusted-output
  wrapping are on. Tools are off unless selected by legacy switches.
* `Some(Features::default())` is explicitly all off. It overrides legacy flags,
  including legacy tool selection. Mandatory host authorization and mode/role
  access constraints still apply; disabling instructions is not authorization
  to write in plan mode.
* Strict tool feature names are those accepted by `adk-tools::Features::Strict`,
  e.g. `ReadFile`, `Bash`, `Signals.Finish`, `ProjectState.TaskTools`, and
  `ExtraTools`. Unknown names fail construction. Selected shell/LSP features
  without required runtime dependencies fail construction.
* `subagents` requires a session containing an existing owned scheduler. Managed
  tools are registered without enabling unrelated ExtraTools. Other injected
  extras remain gated. Legacy `enable_subagents` in `legacy_tools` retains the
  registry's signal/extra-tool behavior; it does **not** automatically create a
  scheduler.

## Host and file configuration

`ConfigSource::load` returns a native `HostConfig` snapshot of modes and roles.
Hosts can implement it without platform types. `FileConfigSource` loads:

```text
~/.gratefulagents/
  modes/*.yaml    # *.yml and *.json also accepted
  agents/*.md    # optional YAML frontmatter
```

`FileConfigSource::default()` chooses `$HOME/.gratefulagents` only when HOME is
absolute and nonempty. Loading fails if HOME is missing, empty, or relative; it
never falls back to repository `.gratefulagents` files. `new(path)` uses the
supplied trusted host path literally, including relative roots (no environment
interpolation or tilde expansion). Files are trusted host
configuration, never implicitly read from the workspace. Loading uses ordinary
synchronous filesystem reads; use a local configuration directory, not a slow
remote filesystem on an async executor thread.

Built-in `chat` and read-only `plan` modes are always available, even without a
source. A file may replace a built-in of the same case-insensitive name. Missing
directories are allowed; unreadable/malformed files, duplicate file names after
normalization, invalid access values, and unsupported fields fail explicitly.
Active mode lookup accepts names case-insensitively and display labels; active
role lookup is exact. File entries are sorted deterministically. A mode name
falls back to CRD-shaped `metadata.name`, then its filename; version defaults to
`v1`. Both plain specs and envelopes containing `spec` are accepted. Outer
metadata is not interpreted beyond `metadata.name`.

```yaml
# modes/review.yaml
name: review
displayName: Review
toolAccess: read-only
instructions: Review changes without modifying the workspace.
constraints:
  maxTurns: 20
  maxRetries: 2
modelRouting:
  defaultModel: openai/medium
  fallbackModels: [anthropic/small]
  reasoningLevel: medium
  textVerbosity: low
  settings:
    temperature: 0.2
  roleOverrides:
    critic:
      model: anthropic/large
      fallbackModels: []
```

```markdown
---
name: critic
description: Review design and correctness
tool_access: read-only
model_override: openai/small
---
Find concrete defects and explain their impact.
```

Role frontmatter also accepts `toolAccess` and `model`. The Markdown body is the
role instruction text; empty bodies fail. CRLF is supported. Unknown fields are
rejected instead of silently implying an unsupported guarantee. Provider keys,
executable paths, stores, approval adapters, and credentials do not belong in
these documents.

### Override precedence

1. Explicit `Config` supplies base instructions/model/settings/policy.
2. `mode_snapshot`, if present, wins over `active_mode` lookup. Otherwise the
   source overrides built-ins by name.
3. Enabled mode routing replaces nonempty model/reasoning/verbosity fields and
   merges setting keys. An absent fallback list inherits; an explicit empty
   list clears fallbacks. Mode instructions append to base instructions.
4. The active role appends instructions, supplies its nonempty model override,
   and narrows access. Explicit `Config::roles` override source roles by name.
5. Enabled per-role mode routing wins over the role model and general mode
   routing. Its settings override earlier keys. `parallel_tool_calls` is always
   set from the resolved feature selection last.

Access is monotonic: no mode or role can widen a host's read-only or
workspace-write baseline. Go's `full` tool-access spelling maps to
`WorkspaceWrite`, **not** unrestricted native `FullAccess`; explicit
`full-access` is available but still cannot widen the host policy. `maxTurns`
is a positive cap intersected with the host cap, not permission to increase it.
`maxRetries` only has an effect when retries are enabled, and zero disables them.

## Bundle and session lifecycle

`Bundle::run` and `stream` take caller context, input history, and the native
`Host` boundary. They freeze the built policy; callers cannot silently replace
it through a request. `agent()` and `policy()` expose observations, not an
unscoped executable runner. Tool handles are revocable wrappers owned by the
tool bundle.

* No supplied session: the bundle creates and owns a `SessionState`.
* `owned_session(state)`: transfers exclusive session ownership to the bundle.
* `session(state.handle())`: borrows a shared session across rebuilds. Closing a
  turn bundle does **not** close that host-owned session.
* `SessionState::with_scheduler(scheduler)` adopts an existing exclusive
  `adk-runtime::subagent::Scheduler`; handles share its `SubagentSession`.
  The host configures the scheduler's executor, child agents, budgets, security
  baselines, persistence, and concurrency. It must use independently owned
  child runtimes if tasks need to survive parent bundle teardown.
* `Bundle::close().await` closes tools and, only when owned, closes the session.
  Both cleanup paths are attempted. `SessionState::close().await` cancels its
  scope and shuts down/joins its scheduler. Close is idempotent.
* Dropping either owner revokes its handles. Drop cancels/aborts rather than
  awaiting cleanup; explicitly close before shutting down the Tokio runtime.
* Caller cancellation, bundle closure, and session closure are combined without
  giving a child authority to cancel the caller. Streams and approval
  continuations retain these scopes, so keeping an escaped handle cannot keep
  a closed owner executable.
* Failed builds close an adopted owned session. A shared host-owned session is
  not closed on build failure. Runner validation failure closes the already
  constructed tool bundle. Provider bindings are validated before resource
  configuration. Cancelling/dropping a build future follows owner Drop rules,
  not asynchronous join guarantees.

Typical host loop:

```rust,ignore
let mut session = SessionState::with_scheduler(host_scheduler);
for input in turns {
    let mut bundle = make_builder()
        .session(session.handle())
        .build(&context).await?;
    let outcome = bundle.run(context.clone(), input, host.clone()).await;
    bundle.close().await?;
    let outcome = outcome?;
    // Retain outcome.spills as long as its history references those files.
}
session.close().await?;
```

Only scheduler state survives automatically. Conversation history, continuation
ownership, tool spill owners, durable checkpoints, and external host stores are
not secretly copied into session state. A host retains those explicitly.

## History attribution and trace snapshots

`Bundle::run_with_provenance` and `stream_with_provenance` accept a parallel
`Vec<ItemProvenance>` without changing the bundle's configured execution policy.
Use `Unattributed` for known host/user additions and `Agent { name }` for known
agent-authored history. Preserve `RunResult::history_provenance` when reusing
`history`; attribution is independent of message role and the current agent.
An empty sidecar means `Unknown`, not the current agent. Nonempty sidecars must
match the input length and agent names must be nonblank; invalid input fails
before model dispatch. The existing `run` and `stream` helpers use `Unknown`.

Generation observers assemble pinned-SDK request snapshots only when all fields
are representable. Unknown attribution, adapter-only settings and fractional
SDK tool timeouts are explicit snapshot errors; they do not fail the run or
fabricate metadata. TraceWriter records conversion errors in its health state.
Full capture persists request content, while metadata capture retains only the
snapshot's size and digest. Host redaction/capture policy remains necessary.
Native durable payload v2 preserves attribution; v1 payloads resume with unknown
attribution because their historical agent names were not reliable.

## Source mapping and explicit gaps

Reviewed mappings are `repos/sdk/pkg/agentsdk/runtime/{builder,features}.go` and
`host/{host.go,fileconfig/fileconfig.go}`. This is native composition, not a
Go `Config` wire codec or a claim of full Go runtime parity.

| Area | Preserved or deliberately different |
| --- | --- |
| Defaults | Agent `agent`, OpenAI default routing, factory model aliases, 100 turns, workspace-write permission, tools off, retry/approval/compaction off, legacy mode routing/instructions and parallel/untrusted flags on. |
| Model settings | Native base settings are empty except `parallel_tool_calls`; Go's automatic medium reasoning/verbosity are not injected. Explicit routing settings map `reasoningLevel` to `reasoning_effort` and `textVerbosity` to `text_verbosity`. Provider adapters own supported-setting validation; do not apply OpenAI-only verbosity to Anthropic routes. |
| Retry | Enabled default is 3 retries with 250–2000 ms backoff; host runner policy can supply delays, predicate, and count. Native runner owns retry advice/fallback behavior. |
| Compaction | Explicitly gated; otherwise retains native runner policy/custom compactor. No Go provider metadata discovery or synthesized handoff-history policy. |
| Files | YAML/YML/JSON mode specs and CRD-shaped envelopes; Markdown/YAML role frontmatter; built-in chat/plan and deterministic overrides. Unsupported fields and invalid/zero turn limits fail rather than silently falling back. |
| Constraints | `maxTurns` and `maxRetries` supported. Go `subAgentMaxTurns`, `maxConcurrentSubAgents`, and `maxRuntimeMinutes` are rejected in mode files; configure child scheduler limits and context deadlines explicitly. |
| Roles | One selected role per bundle, including mode role routing. No automatic specialist agent graph, generic handoff fallback, or agent-as-tool generation. Existing native runner/scheduler APIs remain available to host code. |
| Runtime adapters | No automatic MCP discovery, project-state priming/store discovery, security guardrail installation, forced final summary, polling, tracing/event writer, or persistent ChatLoop. Supply explicit tools, native runner hooks, and native `Host` callbacks. Unsupported runtime features are not represented as inert booleans. |
| Lifecycle | Explicit ownership improves on Go's warn-and-continue tool setup: construction errors fail closed. Shared state is not accidentally closed by a turn bundle. No detached background cleanup is introduced. |

## Offline verification

```sh
cargo test -p adk --features builder --test builder
cargo check -p adk --features builder
```

Tests cover defaults, strict empty versus legacy selection, unavailable features,
typed resource forwarding without enabling unselected tools, read-only
preparation/dispatch, isolated missing/empty/relative HOME rejection, explicit
host roots, YAML/role loading and rejection, mode/role
precedence, opaque provider routing, fallback validation, cancellation/deadlines,
escaped stream revocation, shared-session rebuilds, and adopted scheduler cleanup.
No provider credentials, external service, or platform adapter is needed.
