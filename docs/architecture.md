# ADR 0001: Rust-native foundation, explicit compatibility boundary

Status: accepted for the issue #2 foundation; not approval of a production runner.
The table below records the original foundation scope. Issue #4 adds the optional
standalone engine, output protection, in-process approval continuation and replay;
see [runner design](runner-design.md) for the current runtime implementation and
its deliberately separate provider/durability/platform boundaries.

## Boundaries and dependency direction

```text
adk -> adk-core
    -> adk-codec   [compat feature]
    -> adk-runtime [runtime feature] -> adk-core
adk-platform -> adk-core, adk-codec
adk-agent -> adk-platform
adk-harness -> adk-platform
```

`adk-codec` owns SDK wire codecs; platform-derived codecs live only in
`adk-platform`. Native types need not reproduce Go field names or invalid union
states. Explicit DTOs at the boundary make omission/default/enum decisions
reviewable. Codec crates are not a promise of complete provider compatibility.
Local `adk-core` is our unpublished crate, not the same-named ADK-Rust registry
package. `publish = false` avoids accidental publication/name confusion.

Core combines agent, model, tool, host, run, policy and event contracts because
these types are mutually related; splitting one crate per Go package would create
cycles. Tokio task ownership warrants a separate runtime crate to keep executors
out of the minimal API. The facade makes optional capabilities discoverable.
Platform implements lower-level core traits; core never imports platform types.
`scripts/check-purity.py` rejects transitive platform/Kubernetes dependencies in
the all-feature reusable closure, including indirect dependencies. Cargo rejects
crate cycles. This gate names known platform crates, not every possible future
integration; new adapters must extend the gate during review.

| Domain | Foundation boundary | Not implemented |
|---|---|---|
| Core | Native traits, messages, runs, policies, errors | Complete scheduling semantics |
| Runtime | Owned tasks and cancellation | Agent loop, recovery, streaming orchestration |
| Providers | Model trait | HTTP clients, credentials, provider adapters |
| Tools | Tool trait and approval policy | Tool registry, actual side effects |
| Sandbox | Host capability/policy boundary | OS isolation or sandbox enforcement |
| MCP | Future adapter of model/tool contracts | MCP transport/session protocol |
| State | Serializable run/partial-result data; host boundary | Transactional persistence and resumability |
| Observability | Typed events and host event sink | Exporters, metrics, redaction pipeline |
| Platform | Identity and baseline platform codecs | Kubernetes client, deployment, production worker |

No empty crate is created for every future domain. Extract implementations only
when they have independent dependencies/features and no backward dependency on
the agent loop. `adk-agent --version` identifies the foundation; other invocations
fail, rather than succeeding as fake workers. `adk-harness` is offline replay.

## Ownership and async contracts

Object-safe interfaces use `Pin<Box<dyn Future<Output = T> + Send + 'a>>`: the
explicit lifetime ties in-flight work to borrowed contexts/resources. This is
intentional dyn dispatch, not a mechanical Go interface translation. Native async
helpers can avoid allocation when dispatch is static. Direct `async fn` traits
are not dyn-compatible ([Rust reference](https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility)).

Core cancellation is an executor-independent interface; the optional runtime
adapts Tokio tokens and retains task handles. Tasks have an owner: `TaskGroup::shutdown` cancels, aborts and joins all tasks,
waiting for future destruction/RAII cleanup. It is immediate teardown, not an
async-cleanup grace period; drop requests cancellation/abort without awaiting. Task
futures must yield for cancellation/abort to take effect; neither stops arbitrary
blocking code. Child tokens propagate parent-to-child, not upward. Token clones
share authority. Cancellation does not roll back external effects. Persisted
partial results and call IDs are the basis for future recovery, not evidence of
exactly-once execution. Policies are data, not a sandbox or authorization engine.

We considered `TaskTracker`, but close does not prevent spawning, wait requires
closed **and** empty, and drop does not abort. Prefer an owned task scope with
exclusive spawn access and retained handles for this foundation; do not leak
spawn authority around a shutdown gate.
[CancellationToken](https://docs.rs/tokio-util/0.7.16/tokio_util/sync/struct.CancellationToken.html),
[TaskTracker](https://docs.rs/tokio-util/0.7.16/tokio_util/task/task_tracker/struct.TaskTracker.html).

## Research: adopt patterns, not entire frameworks

Rechecked registry metadata and published source for the versions below. Release
activity is maintenance evidence, not an SLA/security audit. Dates are observed
release dates, not a claim about the historical Go baseline. More source detail
and pinned checksums are in [baseline research](migration/rust-research.md).

| Candidate | Verified version/license/activity | Decision |
|---|---|---|
| [Rig](https://crates.io/api/v1/crates/rig-core) | 0.42.0, MIT, unyanked, published 2026-08-17; no declared MSRV | **Adapt** portable tool boundaries and scripted mocks; **reject** mandatory framework coupling. Rust 1.88 compatibility not established. |
| [genai](https://crates.io/api/v1/crates/genai/0.6.5) | 0.6.5, MIT OR Apache-2.0, unyanked, 2026-06-06; newer 0.7.0-beta.23 observed, no declared MSRV | **Adapt** explicit continuation/correlation; **defer** provider dependency until behavior tested. Stable findings do not apply automatically to beta. |
| [ADK-Rust](https://crates.io/api/v1/crates/adk-core) | 2.2.0, Apache-2.0, unyanked, 2026-09-01; Rust 1.95 required | **Reject** dependency for our 1.88 baseline; **adapt** typed contexts/events, not persistence semantics. |

Source findings:
- [Rig portable tool module](https://docs.rs/rig-core/0.42.0/src/rig_core/tool/mod.rs.html)
  excludes mutable context, registry/lifecycle and executor: keep execution owned
  by applications rather than hiding it in reusable tool definitions.
- [genai tool-use example](https://docs.rs/crate/genai/0.6.5/source/examples/c20-tooluse.rs)
  preserves call IDs but executes only the first call while appending all calls;
  it is not a complete multi-call dispatcher to copy.
- [ADK-Rust model source](https://docs.rs/adk-core/2.2.0/src/adk_core/model.rs.html)
  uses `#[serde(skip)]` for executable tools: JSON round-trip cannot rehydrate
  executable context. Runtime objects must be reconstructed from trusted config.

## Dependency choices and reproducibility

Rust **1.88.0**, edition 2024, resolver 3. Root `Cargo.lock` records exact selected
versions and checksums. Manifests specify compatible major versions, not floating
Git branches; CI uses `--locked`. Upgrades are deliberate PRs with fixture/schema,
MSRV, dependency and license review. No dependency is selected to mimic a Go
package name. CI tests minimal/default/runtime/all as listed in the root README.
The facade default is empty; schemas are core/codec API, not a separate feature.

| Dependency | Choice and rationale |
|---|---|
| serde 1 / serde_json 1 | **Adopt** explicit DTO encoding; `arbitrary_precision` prevents silent f64 narrowing. Native serialization is not declared Go compatibility. |
| schemars 1 | **Adopt** generated JSON Schema for typed DTOs. Lock and test schema structure; schema generation is not validation. |
| thiserror 2 | **Adopt** typed library errors/categories with source messages and partial results; avoid string-only downcasting or catch-all library `anyhow`. |
| tokio 1 / tokio-util 0.7 | **Adopt**, isolated to optional runtime, for cooperative async tasks/token cancellation; reject implicit detached task ownership. |
| async-trait | **Not needed**: explicit boxed future aliases expose Send/lifetime costs without macro indirection. |
| External ADK runtimes | **Defer**: none proves required Go wire/error/stopping/recovery compatibility. |

The tested lock selects serde 1.0.229, serde_json 1.0.151, schemars 1.2.2,
thiserror 2.0.20, tokio 1.53.1 and tokio-util 0.7.19; all compile on Rust 1.88.
Selected transitive licenses are checked by pinned
[cargo-deny 0.20.2](https://crates.io/api/v1/crates/cargo-deny/0.20.2)
(MIT OR Apache-2.0, declared MSRV 1.88) against an explicit SPDX allowlist;
RustSec checks intentionally use the current database. An initial 0.18.4 check
could not parse current CVSS 4.0 advisories, so it was rejected rather than
suppressing the advisory gate.
This is not a legal audit. Foundation/core and SDK-derived code retain GPL-3.0-only;
platform-derived codecs/binaries retain AGPL-3.0-only. See root NOTICE and original
[SDK license](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/LICENSE)
and [platform license](https://github.com/gratefulagents/gratefulagents/blob/08e65c970830f05042c251bcbb46ec6a9e3719b9/LICENSE).
No upstream code is relicensed as permissive.

## Wire/schema acceptance and limits

Test baseline inputs to computed Go outputs independently; never deserialize and
re-emit fixture `expected` as a compatibility test. Preserve absent/null, array
order, IDs, UTF-8 and numeric precision through explicit codecs. Reject unknown operations
and persisted transcript discriminants. SDK numeric item discriminants instead
map to `unknown`, matching Go; that is not a claim of forward compatibility. Timestamp
validation must retain nanoseconds and avoid floating-point epoch conversion.
`serde_json::Number` with arbitrary precision does not promise every original
JSON lexical spelling or byte-identical object ordering
([API](https://docs.rs/serde_json/1.0.143/serde_json/struct.Number.html)).

[Schemars](https://docs.rs/schemars/1.0.4/schemars/) allows generated schema changes
without a breaking release, and minor-version MSRV increases. The lockfile and
schema tests are essential. Fixture, inventory, durable-storage and generated
schema versions are independent namespaces; do not conflate them. Tests cover a
bounded corpus, not every provider schema or full SDK serialization surface.

Acceptance here is the foundation build matrix, typed core, offline fixture
replay, standalone example and dependency/ownership gates. Future gates include
live provider streaming/cancellation, authorization before effects, OS sandbox
parity, MCP interoperability, crash-consistent persistence, redaction, and deployed
platform integration. None is claimed by this foundation.
