# Native runtime builder

Enable `adk`'s **`builder`** Cargo feature. It composes `adk-runtime`,
`adk-providers` (including runtime integration), and `adk-tools`; it does not
require platform services, a worker CLI, or evaluation adapters. Cargo features
make implementations available. `builder::Features` selects what a particular
runtime is allowed to assemble.

For caller-driven message ingestion and approval persistence on top of a native
agent, enable `host` and use the [standalone host-session API](host-sessions.md).
Its stores and lifecycle remain application-owned.

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
With Cargo features `builder,mcp`, `Builder::mcp(McpInput)` supplies validated
configuration and explicit per-server host authority. `Features::mcp` selects
servers, raw/final tool names, and resource adapters independently; input alone
does not enable connections. The bundle owns preflight, bounded acquisition,
discovery, rollback and shutdown. It registers prepared MCP tools without
enabling unrelated `ExtraTools`. See [MCP builder composition](mcp.md#runtime-builder-composition)
for selection defaults, limits and runnable offline cases. Project-state and
other trusted adapters can still supply tool implementations; this builder
does not discover their stores.

`runner_config` supplies existing native runtime hooks, compaction adapters,
budget limits, spill directories, and other supported runner configuration. The
builder controls work directory, selected subagent session, retry/compaction
activation, mutation approval, output trust wrapping, and model parallel-call
settings. It installs baseline provider cost accounting unless the host supplies
an estimator. Unknown prices retain the existing baseline zero-cost behavior;
they are not a reliable hard budget boundary.

## Model-routing helpers

The `runtime` feature alone exposes `adk::runtime::settings::{reasoning_settings,
verbosity_settings, routing_settings}`. They return native settings maps, normalize
labels with the SDK's whitespace/simple-case rules, and leave unknown labels out.
Reasoning budgets are minimal=1024, low=2048, medium=4096, high=8192,
xhigh=16384 and max=24576; provider adapters apply model-specific clamping.
`none` emits only `reasoning_effort=none`, not a zero-budget override. Extending an
existing map with it preserves a prior budget, matching the SDK's nonzero merge.
Use `Config::reasoning = "none"` to construct base settings with no thinking budget.
The builder's free-form `settings` map remains an explicit native override, not a
Go `ModelSettings::Merge` codec.

## Strict selection versus legacy defaults

* `Config::features = None` uses `legacy_tools` and the
  `enable_compaction`, `enable_retry`, `enable_approval`, `enable_guardrails`,
  and `enable_mcp` switches. Mode
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
* `subagents: SubagentFeatures` independently selects `task` (`subagent` and
  `subagent_wait`), `status` (`subagent_status`), and `control`
  (`subagent_control`). All three default off. Any enabled group requires a
  session containing an existing owned scheduler, including status-only and
  control-only builds. Managed tools do not enable unrelated ExtraTools, bypass
  host allow/deny lists, or enter handoff specialist tool views. Legacy
  `enable_subagents` in `legacy_tools` retains the registry's signal/extra-tool
  behavior; it does **not** automatically create a scheduler. To migrate from
  the former `subagents: true`, set all three fields to true; replace false with
  `SubagentFeatures::default()`. The grouping follows pinned SDK
  `runtime/builder.go::asyncSubAgentToolNames`; native scheduler ownership stays
  explicit rather than allocating a hidden scheduler.
* `handoffs` and `handoff_generic_fallback` are separate opt-ins, both off in
  strict and legacy defaults. They neither require nor enable `subagents`, a
  scheduler, or `ExtraTools`.
* `mcp: McpFeatures` defaults off. An enabled selection needs both a server
  selection and a tool/resource selection, plus explicit `McpInput`; requesting
  it without the `mcp` Cargo feature is an error, not a hidden disabled capability.
  Legacy `enable_mcp` selects all configured servers/tools and resource adapters,
  but still requires explicit host-authorized input.

## Builtin tool guardrails

`Features::builtin_guardrails` explicitly enables runtime tool guardrails.
Legacy configuration uses `Config::enable_guardrails`, default `false`; an
explicit feature selection overrides that legacy field. Neither option disables
the native shell's existing enforcement, authorization, or secret checks.

The facade also exposes `adk::guardrails::builtin_tool_input_guardrails()` and
`builtin_tool_output_guardrails()` under the `builder` Cargo feature. These
return ordinary runtime guardrails for direct `RunnerConfig` composition:

1. `block-destructive-commands` checks shell-like custom tools, not only the
   bundled shell implementation.
2. `detect-secret-leak` rejects secret-shaped tool arguments.
3. `detect-secret-in-output` blocks partial credential markers that may accompany
   undetectable credentials; ordinary secret-shaped text is redacted with a
   notice while retaining surrounding text.

Builder-installed builtins precede caller-supplied runtime tool guards. They are
not model/agent input/output guards. Each has a stable, versioned durable key;
custom guards still need their own replay-safe identities. Secret signatures,
normalization and destructive-command classification remain security-owned;
facade assembly does not maintain a second credential matcher.

Native hardening is intentionally broader than the SDK's raw-string guards:
decoded JSON argument strings are inspected, and unsupported dynamic shell
syntax fails closed, including `env` split-string options. Reasoning text is
scanned too, before the runtime can convert it to ordinary text. Multipart text
is checked individually, newline-rendered, and concatenated to detect credentials
both at and across part boundaries; detection blocks the multipart output rather
than guessing how to distribute a replacement. Reasoning/mixed-media output
passes unchanged when safe, but is blocked if it requires redaction because the
runtime's string replacement contract would otherwise discard non-text content.
Scalar output keeps its error/pause flags. These are explicit native limits,
not full SDK guardrail parity.

The independent Go fixture has 87 cases. Rust compares guard order, tripwire and
replacement decisions plus exact redacted output bytes for 75 cases. Eleven
malformed raw-JSON cases cannot be represented by the native `serde_json::Value`
arguments; one escaped credential-marker case deliberately receives stronger
native protection. Go parser-error wording and guardrail prose are not claimed
byte-identical. The offline `guardrails` feature example exercises the public
builtin constructors as well as existing execution-layer protection.

## Final available turn

`Features::force_final_summary_turn` opts the builder into a no-tool summary on
the last available model attempt. Direct runner consumers use
`RunnerConfig::force_final_summary_turn`. Both default off; builder selection is
authoritative over the supplied runner configuration. The request receives the
pinned SDK `<final_turn>` directive without adding it to persisted conversation
history. Tool and handoff descriptors are removed, declared timeout sidecars stay
aligned, and hallucinated tool/handoff calls are denied without approval or
execution. This requests a summary; it cannot guarantee that a model obeys it.

Managed children that still require final joining keep the tool surface
available. Once their results have joined, the existing final-join turn extension
can produce the summary. Native continuation checkpoints preserve whether the
last response was from a no-tool summary attempt; changing the configuration
invalidates durable recovery. The opt-out keeps previous graph fingerprints.

Native budgets count model attempts, including retries/fallbacks. The directive
therefore also applies when a retry consumes the last available attempt. This is
not a claim that native and SDK retry budget accounting are identical. Eight
independently executed pinned Go normal/streamed cases compare exact instruction
bytes, ordered tool names, tool-call counts and final outputs at one/two turns.
Pending-child joins, retries, denied hallucinated calls, generation snapshots and
native durable recovery have separate Rust tests, not independent Go parity
claims. See `scripts/handoff-reference/README.md`.

## Immediate input at run boundaries

Hosts can supply `ImmediateInputPoller` and `ImmediateInputFinalizer` through
`Builder::runner_config` (or directly through `RunnerConfig`). Strict explicit
selection requires `Features::immediate_input_polling`; when it is off, neither
callback is installed. Legacy selection retains callbacks supplied by the host.
The callbacks are optional independently, but only a finalizer can atomically
close admission with a last queue check. A poller alone cannot protect input
racing with run completion.

Callbacks return `ImmediateInputBatch { items, provenance }` with exactly one
authorship entry per item. The runner does not infer authorship from the active
agent. Polling appends accepted input before the next model turn; a finalizer
can return late input instead of completing, extending an exhausted model
budget by one attempt. Normal and streaming execution share this behavior.
Poll errors are best-effort and observable through
`Observation::ImmediateInputPollFailed`; finalizer and malformed-batch errors
abort. Callback futures must not detach work and must be safe to drop when the
parent is cancelled or its deadline expires.

Durable callbacks require nonempty stable `durable_key()` identities. The runner
writes an `immediate_input_dispatched` boundary before invoking the host. A crash
at that boundary is ambiguous: the external queue may already have drained or
closed. Recovery fails closed rather than replaying the callback automatically;
the host must reconcile its queue and checkpoint. This is **not** exactly-once
queue admission. Hosts own the external queue and its retention policy.

The pinned SDK oracle covers fourteen complete/streaming boundary cases and
compares model inputs, returned history/new items, actual authorship labels,
callback/tool counts and output. Native cancellation, malformed batches and
durable recovery have separate regressions. The SDK's optional
`ImmediateInputSignal` is an optional companion described below; boundary
callbacks alone do not interrupt an in-flight model attempt.

### Waking a pending attempt

`RunnerConfig::immediate_input_signal` accepts a host-owned
`ImmediateInputSignal` alongside a poller. A signal without a poller is rejected.
Strict `Features::immediate_input_polling` selection gates the signal too;
legacy mode retains supplied callbacks. Implement `wait` as a drop-safe future
that consumes one notification, not a permanently ready future; it must not
drain the input queue or detach tasks.

A wake before visible output cancels the pending attempt and returns to the
normal polling boundary, rebuilding the request with newly admitted input. A
wake after visible text or reasoning leaves the committed stream running;
native publication of other model events (tool arguments, completed items
or a complete response) also commits the attempt. This conservative native
boundary avoids replaying events already delivered to the host; the SDK oracle
only establishes equivalence for its text/reasoning event variants.
Queued input is consumed at the next boundary or finalizer. The native run's
ordinary root-run turn allowance is extended for superseded attempts, allowing a
replacement even at a one-turn limit. Managed-child turn caps are not extended by
signals. Physical attempt metrics and independent shared/security budget charges
remain intact. Supersession does not
consume provider retry allowance. Native generation observers see interrupted
attempt cleanup rather than a fabricated successful response.

The host still owns queue admission and finalization. Parent cancellation,
deadlines and stream drop stop owned work; a signal is not permission to widen
security budgets. Durable configuration binds the signal's stable key, and
ambiguous dispatched model/admission checkpoints remain fail-closed. No usage
or cost is invented for a dropped provider call whose response was not obtained.
The SDK may account for a completed response returned despite cancellation;
that transport-specific accounting is outside the bounded signal comparison.

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
interpolation or tilde expansion). `from_config_root(text)` trims a CLI/config
string, uses the default for blank text, and expands `~` or `~/…` only with an
absolute HOME. Files are trusted host
configuration, never implicitly read from the workspace. Loading uses ordinary
synchronous filesystem reads; use a local configuration directory, not a slow
remote filesystem on an async executor thread.

With the `host` feature, the same source implements `host::ConfigSource` and is
reexported as `host::fileconfig::FileConfigSource`. Set an immutable active mode
with `with_active_mode("plan")`; no active mode yields no snapshot/directive and
workspace-write permission. Independent `list_modes`, `get_mode`, and `load_roles`
allow callers to inspect one catalog without loading the other. Direct mode
lookup prefers `.yaml` over `.yml`, before catalog fallback; the runtime builder
still validates the complete snapshot before assembly. `build_mode_directive`
is also available as a pure formatter. For a standalone role directory, call
`builder::load_role_catalog(&context, directory)` (also reexported through
`host::fileconfig`). This does not append `agents/`, consult HOME, load modes or
read another source's role directory. It uses the same strict role parser and
checks cancellation before directory access and between files; unlike Go's
context-free helper, cancelled native contexts return a cancellation error.
Missing directories yield an empty catalog. Duplicate names and unknown fields
remain errors rather than inheriting the SDK's permissive policies. See the
pinned Go differential coverage and explicit strict-parser exclusions in
`scripts/fileconfig-reference/README.md`.

Mode `maxConcurrentSubAgents` is checked against the injected scheduler's
immutable concurrency ceiling. A wider scheduler fails construction rather than
being silently resized or affecting another bundle. `maxRuntimeMinutes` is
preserved as metadata, matching the pinned SDK; it is not a timeout. Hosts must
set an execution deadline when they require a runtime limit.

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
   list clears fallbacks. Mode instructions append to base instructions. Enabled
   mode guidance begins with `Active mode: <label>`, preferring the resolved
   snapshot's display name, then name, configured active mode, and finally `chat`.
   This default label is present even without a selected mode in legacy builds;
   explicit `mode_instructions: false` suppresses it. Snapshot label whitespace
   is preserved, matching the SDK formatter.
4. The active role appends instructions, supplies its nonempty model override
   and optional programmatic fallback override, and narrows access. Explicit `Config::roles` override source roles by name.
5. Enabled per-role mode routing wins over the role model and general mode
   routing. Its settings override earlier keys. `parallel_tool_calls` is always
   set from the resolved feature selection last.

Access is monotonic: no mode or role can widen a host's read-only or
workspace-write baseline. Go's `full` tool-access spelling maps to
`WorkspaceWrite`, **not** unrestricted native `FullAccess`; explicit
`full-access` is available but still cannot widen the host policy. `maxTurns`
is a positive cap intersected with the host cap, not permission to increase it.
`maxRetries` only has an effect when retries are enabled, and zero disables them.

## Catalog handoff composition

Enable `Features { handoffs: true, ..Default::default() }` and supply catalog
roles through `Config::roles` or `ConfigSource`. `Bundle::specialists()` exposes
an immutable map of role names to `Arc<AgentConfig>` targets. Parent handoffs
retain source order: host entries first, explicit config replaces entries in
place by trimmed name, and new config entries append. File catalogs already
arrive sorted by name.

Each target starts from the **configured base**, not the active parent role's
mutated model/settings/instructions. Its routing order is base → role model and
fallbacks → mode defaults → mode role override. The last two steps require
`mode_model_routing`; role overrides do not. `RoleSpec::fallback_models` is a
programmatic `Option<Vec<String>>`, default `None`. Like native mode routing,
`Some(vec![])` explicitly clears inherited fallbacks, unlike pinned Go's
nonempty-only fallback replacement. The shared Markdown/frontmatter parser is
unchanged and rejects new fallback fields. Every enabled target model/fallback
resolves against the same shared `Routes` before tool resources are created;
there is no credential fetch or provider discovery.

Targets preserve their role instruction text verbatim (no parent or mode prompt blocks),
cloned input/output guardrails, and owner-bound role tool views. Their explicit
`tool_access_ceiling` intersects host, mode, active-parent-role, and target-role
access and removes mutation exceptions. `finish`, `present_plan`,
`AskUserQuestion`, and all generated managed scheduler tool names are removed.
Parent tools remain unchanged. Role views share the existing resource owner,
not new tool bundles. Saved handles are revoked on bundle close/drop.

Only the parent receives `transfer_to_*` handoffs with `RemoveTools`. Transfer
descriptions use the trimmed role description or the SDK default. Names use the
source sanitization: lowercase ASCII letters/digits survive; spaces, hyphens,
underscores and dots become underscores; other characters are dropped; leading
and trailing underscores are removed; an empty result becomes `specialist`.
Blank names/instructions, duplicates within either catalog, sanitized collisions,
and collisions with enabled parent tools fail construction rather than silently
creating a generic fallback. The graph allowlist includes ordinary tools and
transfers but intersects the **original** host allowlist/denylist; an explicit
empty allowlist stays empty. Denied transfers are not advertised or described as
available delegation options.

For an empty catalog only, additionally enabling `handoff_generic_fallback`
creates a tool-less `specialist` using the SDK handoff prompt and transfer
description, the configured base model/settings, and no fallback bindings.
Otherwise an empty catalog produces no handoffs. The generic target also carries
the host guardrails and access ceiling.

**Intentional gate divergence:** pinned SDK `BuildAgentWithSpecialists` builds
catalog targets behind its subagent gate, then attaches handoffs. Native catalog
handoffs are independent of scheduler/subagent selection. No agents-as-tools,
nested/return handoffs, or scheduler role registration are created. The parent
delegation guidance describes a transfer of conversation ownership, not an
unavailable nested task API. See [handoff composition contracts](handoff-composition.md)
for runtime filtering and access-ceiling details.

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
| Model settings | Default (and blank) `Config::reasoning`/`verbosity` are medium: effort `medium`, thinking budget 4096, verbosity `medium`, plus selected parallel-call behavior. Explicit settings override base settings. Mode/role labels use `runtime::settings` helpers with the SDK's normalized efforts/budgets; unknown labels contribute no override. Providers clamp thinking budgets to model/output limits. Anthropic accepts but does not transmit string verbosity, matching the SDK. |
| Retry | Enabled default is 3 retries with 250–2000 ms backoff; host runner policy can supply delays, predicate, and count. Native runner owns retry advice/fallback behavior. |
| Compaction | Explicitly gated; otherwise retains native runner policy/custom compactor. No Go provider metadata discovery or synthesized handoff-history policy. |
| Files | YAML/YML/JSON mode specs and CRD-shaped envelopes; Markdown/YAML role frontmatter; built-in chat/plan and deterministic overrides. Unsupported fields and invalid/zero turn limits fail rather than silently falling back. |
| Constraints | `maxTurns` caps parent turns; `subAgentMaxTurns` narrows child turns; `maxRetries` sets retries. `maxConcurrentSubAgents` validates the injected scheduler's immutable concurrency ceiling without mutating a shared owner. `maxRuntimeMinutes` is retained as metadata, matching the pinned SDK's lack of enforcement; hosts supply actual deadlines through `Context`. |
| Roles | Active-parent role routing plus opt-in parent-to-catalog handoffs and tool-less empty-catalog fallback. Native handoff gating is independent of subagents; no agent-as-tool generation, nested graph, or scheduler registration. |
| Runtime adapters | Host-authorized MCP and project-state composition, opt-in built-in guardrails, final-summary selection and host-supplied immediate-input callbacks are available. MCP has no implicit repository/credential discovery. Tracing/event sinks still require explicit native observer/host composition; the persistent host-session API is separate from the runtime bundle. |
| Lifecycle | Construction errors fail closed rather than returning partially configured tool resources. Borrowed state is not closed by a turn bundle. MCP cleanup is retained across cancellation; admitted synchronous project-state writes drain on explicit close. Keep the executor alive until close completes. |

## Offline verification

```sh
cargo test -p adk --features builder --test builder --test catalog_handoffs
cargo check -p adk --features builder
```

Tests cover defaults, strict empty versus legacy selection, unavailable features,
typed resource forwarding without enabling unselected tools, read-only
preparation/dispatch, isolated missing/empty/relative HOME rejection, explicit
host roots, YAML/role loading and rejection, mode/role
precedence, opaque provider routing, fallback validation, cancellation/deadlines,
escaped stream revocation, shared-session rebuilds, and adopted scheduler cleanup.
Catalog tests also cover actual run/stream transfers, role routing independence,
preflight validation, feature/generic gates, read-only mutation denial, signal/task
stripping, graph allowlists, name collisions, and saved-handle revocation.
No provider credentials, external service, or platform adapter is needed.

The eight managed-subagent feature combinations compare tool-name sets with
`fixtures/handoff/sdk-subagent-selection.json`, independently generated through
the pinned public SDK `runtime.NewBuilder(...).Build(...)`. The fixture records
original SDK ordering; Rust's registry ordering is intentionally not compared.
Explicit native scheduler ownership and host-policy/exclusion checks remain
separate lifecycle assertions rather than claims of automatic SDK allocation.

### Bounded catalog oracle comparison

`bounded_catalog_handoff_projection_matches_independent_pinned_go_oracle` reads
`fixtures/handoff/sdk-catalog-handoffs.json` generated independently from the
pinned SDK. It compares ten explicitly selected cases: ordered handoff names,
descriptions/read-only flags, filter presence, target names/instructions/models,
ordered fallback lists, model settings and catalog target pointer identity.
SDK nil and empty fallback slices both map to native empty vectors. SDK scalar
MaxTokens maps to the native `max_tokens` settings entry. Tool surfaces and their
ordering, parent prompts/routing, MCP fields, full Agent serialization and actual
filter outputs are **not** part of this comparator; separate native run/stream,
policy and lifecycle tests cover execution. No excluded case counts as a pass.
The selected cases are named in the test; seven source-only cases and the other
thirteen unselected cases remain outside this comparison.

Additional native differences are deliberate: `Config::settings` maps are
honored whereas this SDK builder uses scalar settings only; unknown role access
strings fail instead of becoming read-only; disabled ExtraTools cannot supply
target tools; the immutable specialist getter includes the generic target while
the SDK specialist map does not. Native tools retain the registry's deterministic
name ordering rather than injected host order. Native strict catalog validation,
independent handoff gating, explicit-empty fallback clearing and handoff-only
parent guidance are not normalized into SDK parity claims. Exact-case signal
stripping leaves differently cased host names such as `Finish` intact.

## Owned project-state composition

Enable both `builder` and `project-state` Cargo features. The four
`Features.project_state` switches (`prime_context`, `task_tools`, `memory_tools`,
`prime_tool`) are independent; any one requires a store. Legacy
`Config.enable_project_state` or `legacy_tools.enable_project_state` selects all
four. An explicit `Features` value overrides legacy selection. Missing Cargo
support is an error, not a silently disabled capability.

Supply `Builder::project_state_store(Arc<dyn Store>)` to borrow an application
store, or `Builder::project_state_host(FilesystemResolutionHost { cwd, home })`
for automatic filesystem construction. An injected store takes precedence.
Host paths must be absolute; configuration paths resolve lexically relative to
that explicit cwd without reading process environment. `Config.project_state`
supplies the state directory, project ID, actor, run ID and active-task ID;
`Config.work_dir` supplies the workspace. A home is required only for the default
`.gratefulagents/projects/<id>/state` directory. Resolution does not bypass
private-file or symlink checks.

Full builds prime once with ready/memory limits of eight. Actor selection uses
the first nonblank configured actor, agent name, or `agent`; the active-task ID
is forwarded unchanged and never causes a claim. Nonblank prime text is appended
with two newlines to `RunnerConfig.working_state_context`, preserving existing
nonblank content. This is compaction carry-forward state, **not initial prompt
injection**. Priming failure is nonfatal and appears in `Bundle::warnings()` as a
sanitized diagnostic; initialization failure fails construction. Raw store
errors and private paths are not included in these diagnostics.

After priming, a full builder replaces blank working-state text with
`Runtime state is maintained by the host adapter.` (the SDK default).
Explicit nonblank host text and successful priming take precedence. The default
is also injected only after compaction; direct `RunnerConfig::default()` remains
empty, and tool-only construction does not prime or add working-state context.

With compaction enabled and an unchanged default local policy, full composition
uses the SDK builder's ten recent items, two initial user messages and five
summary bullets. `RunnerConfig.compaction_model_defaults` resolves token
thresholds for each active model without resetting those history rules; this
choice is part of durable resume identity. Explicit nondefault retention and
threshold settings are preserved; feature selection still controls enablement.
Direct runtime defaults remain twelve recent items/four bullets.

`Config.local_compaction: Some(policy)` is the explicit host override: its
enabled flag takes precedence over feature selection, including an explicitly
disabled policy when the compaction feature is enabled. With that feature on,
thresholds still follow the active model, as in the SDK; with it off, the
explicit policy's thresholds remain fixed, even when equal to runtime defaults.
Absent an explicit `Config` policy, existing `runner_config` policy handling is
unchanged. `RunnerConfig.compaction_model_defaults` distinguishes `Some(true)`
(resolve per model), `Some(false)` (fixed), and `None` (direct-runtime default
selection). This choice is fingerprinted for durable recovery.

`RunnerConfig.compaction_model_resolver` accepts a host-owned async
`CompactionModelResolver`. A positive trigger overrides token thresholds for
the active model; `None` or zero trigger retains the configured policy, without
falling through to model defaults. Zero/oversized targets normalize normally.
The same resolution is shared by local and custom compaction within an attempt;
new attempts, continuing turns and fallback models resolve again. The builder
retains this callback only when the compaction feature is selected, as in the SDK.
Resolution honors cancellation/deadlines. Native callback errors abort before
provider dispatch; unavailable metadata should instead return `Ok(None)`.
Durable use requires a nonempty key covering deterministic, replay-safe behavior
and configuration; network discovery must not opt into that contract.

Provider catalog fetching/caching and automatic metadata resolver assembly are
still host-owned, not automatically wired by the builder. The existing
`adk::providers::metadata::ModelMetadata::compaction_defaults` helper can supply
thresholds for an explicitly loaded catalog. This does not claim the SDK's
automatic authenticated `/models` lookup/default cache behavior.

```rust
let config = adk::builder::Config {
    local_compaction: Some(adk::runtime::compaction::LocalCompactionPolicy {
        enabled: false,
        ..Default::default()
    }),
    ..Default::default()
};
```

Unless the host supplies `compaction_carry_forward`, full builds install the SDK
default `Runtime state: provider=<configured provider>, mode=<mode label>`.
Provider defaults to `openai`; the label uses snapshot display name, snapshot
name, configured active mode, then `chat`, independently of mode instructions.
This snapshot callback is deterministic and has a durable identity. As in the
SDK, its nonblank result takes precedence over static/primed working-state text.
Supply a callback returning blank text to use the static fallback instead, or a
host callback for current state. Tool-only construction installs no callback.

The eight task tools, six memory tools and `prime_context` use owned composition,
not `ExtraTools`. Duplicate detection, role views and read-only execution policy
still apply. Synchronous storage runs on Tokio's blocking pool. Closing or
dropping the bundle revokes retained tools; explicit `close().await` also waits
for admitted blocking operations. Cancelling a caller or close waiter cannot
terminate an in-progress synchronous write or replay it. Keep the executor alive
until close finishes. Caller-owned references to an injected store are not
revoked or closed by the bundle.

`FilesystemOptions::resolve(&host)` also supports standalone store construction.
It preserves the original configured project ID while resolving paths: passing
a derived ID through explicit-ID sanitization again can change punctuation-only
workspace identities. Resolver/store-open regressions cover this boundary and
Go's simple Unicode lowercase behavior.

Verification: `scripts/project-state-runtime-reference/run.py --check` executes
the pinned SDK full builder independently for sixteen feature combinations;
Rust compares store creation, tool names and working-state text. This bounded
fixture does not establish whole-builder parity, provider-network behavior or
target-wide support. Its tool-only observations also exercise `BuildToolBundle`.

## Provider-free tool assembly

`Builder::build_tools(&context).await` returns a non-cloneable `ToolRuntime`.
It uses the same registry, typed shell/LSP/browser resources, project-state and
MCP assembly paths as a full build, but does not construct providers or runners,
load a runtime `ConfigSource`, resolve modes/roles, prime working state, generate
handoffs or create a scheduler. Runtime-only feature switches are not tool
selection. Pass the intended tool policy directly through `Config.policy.tools`.
A priming-only project-state selection still opens a store; it does not prime it.

Use `prepared()` for owner-bound tools and their matching dispatch policy,
`context()` for a cancellation scope, and `close().await` before executor shutdown.
An explicitly supplied owned session remains alive until tool-runtime teardown;
a borrowed session remains caller-owned. Build failures join owned sessions and
roll back acquired MCP processes. Retained handles are revoked on close/drop.
`mcp_catalog()` and `mcp_servers()` expose selected discovery metadata when MCP
is compiled in. Offline comparisons cover the sixteen project-state selections
and twelve MCP selections independently against the pinned SDK tool builder.

## Workspace instruction context

Full builds append the pinned SDK workspace block after parent instructions and
delegation guidance. With explicit `Features`, the block uses the actual
policy-prepared tool names (in native registry order), the narrowed access level,
and the configured working directory verbatim. A blank/whitespace-only directory
omits that strict block. An explicit empty feature selection still supplies the
environment block with `Available tools include: none.` when the directory is
nonblank: disabling capabilities does not disable workspace context.

Legacy selection uses the SDK's fixed workspace text, including its historical
tool list and path guidance, even for an empty directory. This wording is
compatibility prompt text, **not an assertion that those tools are installed or
that a particular sandbox is active**. Real registered tools, dispatch policy
and sandbox backends remain authoritative. Hosts that require guidance limited
to the actual tool surface should use explicit `Features`. Native workspace-write
and full-access permissions both map to the SDK's `full` prompt label; read-only
uses `read-only`. No execution permission is broadened by this label.

`builder::workspace_context(&str, AccessMode)` exposes the legacy formatter for
standalone embedding. Full-builder paths must be UTF-8 rather than producing
lossy prompt text. Tool-only builds have no prompt. Specialist target instructions
are unchanged; this block belongs to the parent agent.

`python3 scripts/workspace-context-reference/run.py --check` independently
executes the pinned SDK formatter and checks its position in full-agent
instructions. Sixty strict/legacy workspace cases and ten mode-label cases
preserve exact text, tool order, whitespace and Unicode. Full parent instructions
are compared for the representable, registry-ordered inputs, including the legacy
`Active mode: chat` default. Native comparisons additionally verify normal/streamed
requests and policy-filtered names; they do not equate native registry ordering,
mode/role composition or sandbox behavior with the SDK.

## Run-wide instructions

`RunnerConfig::additional_instructions` is appended to each active agent's
instructions, including handoff targets, before structured-output guidance.
Blank sections are omitted, nonblank agent text is preserved, and the run-wide
section is trimmed. Sections use the SDK separator `\n\n---\n\n`.
The deprecated Go `ModeInstructions` alias maps to this same native field; there
is no second alias or fallback field.

Full builder configuration composes `feature_summary` (prefixed with
`Runtime surface: `), `mode_directive_text`, and `final_check_instructions` in
that order, separated by blank lines and with the aggregate trimmed. A nonblank
aggregate replaces `RunnerConfig::additional_instructions`; an empty aggregate
preserves the supplied runner value. These sections do not mutate agent text or
create instructions in a tool-only build. Durable checkpoints bind the effective
trimmed run-wide text, rejecting resumes that change it; empty defaults retain
the previous fingerprint representation.

Forty independently executed pinned SDK normal/streamed request fixtures cover
base whitespace, blank extras, Unicode and all eight builder section selections.
Native tests also cover handoff propagation and durable mismatch rejection.
Deprecated-field fallback precedence is not covered by this mapping.

## MCP request context

`AgentConfig::mcp_servers` supplies request-only server metadata. Full builder
composition populates it from **connected** owned MCP sessions, after selection
and successful assembly; merely configured or disabled servers are not listed.
Specialists do not automatically inherit the parent's names. Standalone runtime
users can supply metadata without enabling the MCP transport crate. Handoffs use
the active target's list, and nonempty lists participate in durable agent identity.

The SDK prompt block follows additional instructions and structured-output
context. Names retain order and duplicates; each is trimmed, whitespace runs
collapse to a single ASCII space, and control/nonprintable characters are removed.
The result is trimmed and capped at 64 Unicode scalar values (not UTF-8 bytes).
Empty names are omitted. Classification is deliberately pinned to Unicode 15.0,
matching the SDK, rather than changing with Rust compiler Unicode tables.

Fifty-six actual pinned normal/streamed requests and an exhaustive scalar digest
verify the formatter; builder tests verify selection and provider delivery.
Prompt names describe connections, not sandbox/tool authority or a guarantee of
resistance to arbitrary natural-language prompt injection. Invalid-UTF8 Go strings
have no direct native `String` representation and are outside this mapping.
