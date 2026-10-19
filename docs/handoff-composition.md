# Handoff history and specialist tool boundaries

These native primitives support explicitly composed handoff graphs. They do not
assert complete SDK specialist, handoff-callback or delegation parity. The
source reference is SDK `v0.0.115`, commit
`1dc92b73900fac74dc357a938e4b5eee6392b418`; see the immutable source lock and
retained licenses under `docs/migration/`.

## History is distinct from the audit record

`adk::runtime::Handoff` explicitly selects an `input_filter`:

- `HandoffInputFilter::Preserve` retains the existing history behavior.
- `HandoffInputFilter::RemoveTools` removes native tool calls/results, handoff
  records and reasoning from the receiving agent's history. Messages, phased
  messages and compaction records retain their order and exact provenance.
  Forwarded approval markers are cleared through the approval journal's history
  replacement operation.

The filter runs after transfer and deliberately skipped sibling outputs are
published, and before the handoff checkpoint and target model request. It does
**not** erase those events, approval audit entries, responses or `new_items`.
Context token estimates are updated after filtering. This distinction matters
for replay, billing and historical authorship: removing content from model input
is not permission to rewrite the execution record.

`Handoff` struct literals must now include `input_filter`; existing callers can
use `HandoffInputFilter::Preserve` to retain behavior. Non-default filters are
bound into durable graph fingerprints. Restoring a checkpoint under a graph with
changed filter semantics fails rather than silently altering the target input.
The built-in enum is deliberately smaller than a Go callback API: arbitrary
user-defined input filters are not supported by this primitive.

The independent oracle in `scripts/handoff-reference/` invokes the exported Go
filter. Its 23 cases include eight explicitly source-only cases; native tests
compare the remaining 15 with documented representability scaffolding. Agent
serialization in the fixture is a documented name-only projection, not a full
SDK Agent serializer. Unknown SDK item types, payload-less messages and other
source-only observations are not counted as native parity passes.

## Runtime access ceilings

`AgentConfig::tool_access_ceiling: Option<AccessMode>` is independent of the host
run policy. `AgentConfig::new` initializes it to `None`; direct struct literals
must add the field explicitly. `None` preserves existing behavior. An explicit ceiling narrows
host access; it cannot broaden it. Exact mutation exceptions, including implicit
control-flow grants, are removed for explicit ceilings. Host allowlists,
denylists, approval requirements, deadlines and child-turn limits remain
applicable.

The effective policy is used for access adaptation, model-visible tool
selection, approval planning, serial/parallel dispatch and `ToolContext`.
Approval does not authorize a role-forbidden call. The run-global policy is not
mutated, so using a restrictive target does not change subsequent parent runs.
Durable fingerprints bind explicit ceilings even if the resulting tool
definitions happen to be identical.

## Owner-bound adapted tool views

`adk::tools::bundle::ToolBundle::role_view(access, &excluded_names)` creates a
restricted `PreparedTools` view without constructing another resource bundle.
It reapplies capability selection for built-ins, adapts implementations before
authorization, rejects renamed adapters, and preserves extension identity rather
than inferring it solely from a name. Exclusions are explicit: callers assembling
SDK-style specialists must strip orchestrator signals and managed scheduler
tools appropriate to their graph.

The owner retains adapter implementations; returned handles are weak and share
its cancellation boundary. Close/drop invalidates saved parent and specialist
handles. Execution intersects the caller policy with the frozen view policy,
including name sets, mutation grants, approval mode and limits. A more permissive
caller cannot widen a restricted view. A view cannot widen either the owner's
configuration access or its prepared policy; an explicitly empty allowlist
remains empty.

Access declarations and trusted tool adapters are **not** OS confinement. Bash's
read-only adapter uses an `Auto` enforcing executor even when the original
host-owned Bash uses `Local`. Ordinary nonzero shell exits are reported in output
content, not necessarily by `ToolOutput::is_error`. Tests must inspect process
status and filesystem effects. This worker can exercise fail-closed behavior
when Bubblewrap is unavailable; successful Linux/macOS confinement requires the
sandbox-capable CI lanes, where `ADK_REQUIRE_SANDBOX=1` is mandatory.

## Source and design choices

- SDK filter: `internal/agent/handoff_filters.go` and its execution boundary in
  `internal/agent/runner.go`.
- SDK composition: `pkg/agentsdk/runtime/builder.go` and
  `pkg/agentsdk/specialists.go`.
- Native ownership deliberately uses one explicit `ToolBundle`, immutable agent
  targets and owner-bound adapted handles instead of copying resource owners or
  translating Go callbacks indiscriminately.
- Native host policy is always authoritative; strict graph/config validation and
  fail-closed replay semantics are not relaxed merely to reproduce permissive
  source behavior. Divergent aggregate APIs remain unresolved in the ledger.
