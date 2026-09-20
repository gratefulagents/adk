# Issue #11 verification ledger — pre-implementation audit

## Status

This is the completed audit result, not final implementation validation. It
describes the repository at Rust revision
`489b1886aa760b6a2d4680d42bd3dce0a1397407` before concurrent builder,
observability, and feature-example changes are accepted. A passing module-level
test or a façade re-export does **not** close all baseline source obligations.

The authoritative disposition data is
[`docs/migration/ledger/issue-11-overlay.json`](../migration/ledger/issue-11-overlay.json).
It contains all **9,142** v0.0.115 acceptance IDs:

| Disposition | Count | Meaning |
|---|---:|---|
| `excluded` | 251 | Deliberately outside issue #11 scope; not implemented or verified. |
| `unresolved` | 8,891 | Requires acceptance-ID-specific implementation and semantic verification. |
| auto-verified | 0 | No module, API, test, command, or feature association is treated as automatic verification. |

Excluded records are `sdk_cli` (198), `sdk_evals`/`sdk_evals::*` (50), and the
three `.github/workflows/terminal-bench.yml` records misrouted to `sdk::ci`.
The latter IDs are `SDK-EE79D2040B1F61A2`, `SDK-A4A8B27E9715FD93`, and
`SDK-30D5117381CDA2A4`. Exclusion does not authorize implementation of the
prohibited CLI, evaluation, or Terminal-Bench contracts.

## Baseline and source-pin evidence

| Item | Result |
|---|---|
| Ledger baseline | SDK v0.0.115, `1dc92b73900fac74dc357a938e4b5eee6392b418` |
| Generated baseline files | `inventory.json` and `sources.json` manifest digests matched; all 460 archived source-file hashes matched their corresponding local checkout files during the audit (this does not make the checkout HEAD match the pin). |
| Local checkout at audit | SDK v0.0.116, `63afe2ed8cc5f13ca7469054f2c1cb812fcac801` |
| Drift | v0.0.116 adds computer-use metadata files after v0.0.115. The overlay is keyed only to the archived v0.0.115 ledger. |
| Standard validator | `python3 scripts/inventory/validate.py` stops at its checkout-pin assertion because `repos/sdk` is v0.0.116. This failure is expected evidence of drift, not a failing source-hash comparison. |

The generated v0.0.115 directory is immutable baseline evidence. The overlay
does not edit its historical `not_implemented` / `not_run` statuses, because
the baseline schema fixes those values and regeneration would overwrite direct
completion claims.

## Overlay contract and deterministic check

Every acceptance entry has the source record identity, category, proposed Rust
module, scope/disposition, empty Rust implementation and test evidence lists,
an upstream-regression reference-group key, `verification_status: not_run`,
semantic-closure state, and an approved-divergence placeholder. The root
resolves reference groups to pinned upstream test records and also records the
audit Rust revision, SDK drift, counts, and a module-level API/test association
index.

The association index is intentionally separate from semantic closure. For
example, a test in `crates/adk-tools/tests/registry.rs` can be a useful module
reference, but it cannot establish every tool's schema/default/approval/result
semantics. Only an entry with explicit evidence may change from `unresolved`.

Regenerate or check only the overlay from the repository root:

```sh
python3 scripts/issue11-ledger.py generate
python3 scripts/issue11-ledger.py check
```

The script reads the generated inventory and manifest, writes only
`docs/migration/ledger/issue-11-overlay.json`, and refuses an output path inside
`docs/migration/ledger/sdk-v0.0.115/`. `check` verifies deterministic bytes and
the audited totals: 9,142 total, 251 excluded, and 8,891 unresolved.

## Module references are not closure

The overlay's `module_reference_map` points to the current façade and test
locations for core, runtime, tools, providers, durable state, MCP, execution,
and observability. It is a navigation index, not a crosswalk from every Go
symbol to Rust behavior. The companion [facade research](../research/facade.md)
lists the associations and their feature gates.

No audit command ran live providers, OAuth, remote MCP services, or the
operating-system runtime matrix. Those remain unverified even where an offline
test file already exists.

## Implementation delivered on the issue #11 branch

This change adds working **native** APIs, not full source parity:

- `adk::builder::{Builder, Config, Features, ConfigSource, FileConfigSource,
  SessionState, SessionHandle, Bundle}`: provider/tool assembly, strict versus
  legacy selection, YAML/Markdown mode/role overrides, read-only narrowing,
  typed resource inputs and owned/shared lifecycle. See [builder mapping](../runtime-builder.md).
- `adk::observability`: ordered awaited hooks/host events, bounded incremental
  JSONL decoding, cumulative progress, metadata-first capture, explicit full raw
  capture, private Unix trace storage and a host-owned real OTel tracer bridge.
  See [observability mapping](../observability.md).
- Twenty named, runnable [feature scenarios](../feature-examples.md), including
  a custom auditing namespace-memory backend. Directory-level coverage is not
  an assertion that every Go example entrypoint is equivalent.
- An independent consumer workspace, individual Cargo-feature CI matrix,
  original native JSONL fixture, research/licensing notes and explicit CLI exclusions.

### Local validation

Validation used **Rust 1.88.0, x86_64 Linux**, with dependencies cached before
using `--offline`. The installed rustup/cargo-clippy launchers could not resolve
`/proc/self/exe` in this worker. A directly installed pinned compiler plus an
explicit library path worked; Clippy ran using `RUSTC_WORKSPACE_WRAPPER` and
`CLIPPY_ARGS=-Dwarnings`. This is not validation on a newer compiler.

| Check | Observed result |
|---|---|
| `cargo test --offline --locked --workspace --all-features --all-targets` | **788 passed, 0 failed, 30 ignored** |
| `cargo test --offline --locked --workspace --all-features --doc` | **1 passed** |
| `builder` integration tests | **18 passed**; strict/legacy selection, unavailable tool failures, routing/overrides, policy narrowing, lifecycle/rebuilds, typed resources and isolated HOME rejection |
| `observability` tests with `otel` | **22 passed**; real in-memory OTel SDK exporter, paused continuation, ended-parent IDs, shared-run isolation, two-turn success spans, ordering/backpressure, raw outputs, redaction, private file/quota regressions |
| `event_fixtures` | **1 passed**; exact native-schema bytes and fragmented order; not a Go trace-schema fixture |
| Feature executable | **20/20 PASS**, including custom store operation assertions; no live credentials or provider calls |
| Clippy direct driver, workspace/all-features/all-targets, `-Dwarnings` | Passed |
| `cargo doc --offline --locked --workspace --all-features --no-deps`, `RUSTDOCFLAGS=-Dwarnings` | Passed |
| Rustfmt direct check over workspace and consumer sources; `git diff --check` | Passed |
| `cargo deny --locked check` | `advisories ok, bans ok, licenses ok, sources ok`; narrow hashbrown duplicate exception documents YAML/SQLite dependency generations |
| Existing offline codec harness/replay | **7 fixture cases passed**; **17 Python replay tests passed** |
| `sh scripts/check-facade-matrix.sh` with offline cache | Passed: minimal, each of 12 individual features, runtime/tools/providers composition, all features, independent consumer and transitive purity check |
| Independent consumer workspace | Compiled and ran with every reusable facade feature; all-feature dependency closure is platform-free |
| Purity negative tests | **4 passed**, including independent provider/tool roots |
| `python3 scripts/issue11-ledger.py check` | **9,142 IDs**, **251 excluded**, **8,891 unresolved**; no invented semantic closure |

Independent source review found four concrete bugs and all received fixes and
regressions: paused runs terminalizing observations, HOME fallback reading an
untrusted checkout, per-run shutdown ending other shared OTel runs, and normal
multi-turn runs producing error agent spans.

The thirty ignored tests are 28 pinned-Go MCP interop cases, one pinned-Go
configuration/history replay case, and one sandbox-invoked network helper.
They are **not** counted as passing. Provider/OAuth credentials, real Postgres/
pgvector, remote MCP/OTLP collectors, required OS-confinement jobs and non-Linux
execution were not configured or verified here. The native examples use an
explicit local subprocess backend, not a claim of sandbox enforcement.

### Blockers to closing #11

1. The complete retained ledger still needs acceptance-ID-specific semantic
   evidence; the immutable audit overlay deliberately remains unresolved.
2. Native observation schema 1 and event-store layout do not implement Go trace
   schema 2, category files, metadata/score/artifact/list APIs, reopening or every
   span attribute. Existing Go event adapters remain separate, not a claim of
   full trace/event format parity.
3. The builder does not automatically construct specialist/handoff graphs,
   prime project state, discover MCP stores/connections, install arbitrary
   guardrail callbacks or implement all public ChatLoop/AutoLoop/phase helpers.
   Mode subagent/runtime limits and default reasoning/verbosity mappings have
   explicit unsupported/different behavior in the builder document.
4. Go OTLP endpoint/environment/stdout constructor defaults and complete child/
   session progress inference are not provided by the injected-tracer bridge.
5. Live-provider, external-service and cross-OS evidence is unverified. The
   checkout-pin discrepancy was resolved in the continuation below.

These are blockers, **not accepted divergences or capabilities hidden behind
flags**. The PR must remain draft and must not auto-close #11. No worker CLI,
evaluation/Terminal-Bench adapter, release, merge or production default switch
is part of this delivery.

## PR33 continuation: authoritative reference and store/exporter APIs

The source checkout is now clean at the authoritative
`1dc92b73900fac74dc357a938e4b5eee6392b418`. Both
`python3 scripts/inventory/validate.py` and
`python3 scripts/check-source-lock.py` pass. The latter also required restoring
the clean attached platform reference to its already-accepted source-lock
revision `08e65c970830f05042c251bcbb46ec6a9e3719b9`; no platform source was
changed. No accepted baseline, source lock, archived hash or historical audit
snapshot was rewritten.

Fresh independent Go execution is recorded in
[`issue-11-pinned-reference.json`](issue-11-pinned-reference.json):
`go test -count=1 -json ./pkg/agentsdk/...`, Go 1.26.8, **729 passed test/subtest
records, 5 skipped**, across 34 packages. Package results include packages
without tests; they are not counted as passing tests. The overlay now attaches
exact source-test execution evidence to **644 acceptance IDs**, separately from
Rust semantic verification. These source results do not close the 8,891 retained
Rust obligations.

New [category-store and exporter APIs](../trace-store.md):

- Linux `tracestore::TraceStore` / `FilesystemTraceStore`, metadata and scores,
  category rotation, artifact writes, reopen, list/filter, private atomic
  metadata replacement and explicit close.
- An independently generated pinned-Go store fixture plus Rust filesystem and
  quota regressions. The original schema-1 event writer is unchanged.
- `telemetry::Telemetry` constructors with explicit/environment/stdout endpoint
  selection, gRPC/TLS, five-second batching, service resource/instrumentation
  defaults, flush/shutdown and explicit global installation.

### Fresh continuation verification

Rust **1.88.0**, Linux x86_64:

| Check | Result |
| --- | --- |
| Full locked workspace/all-features/all-targets | **797 passed, 0 failed, 30 ignored** |
| Locked workspace doctests | **1 passed** |
| Strict workspace Clippy via direct driver | Passed |
| Strict all-feature rustdoc and workspace Rustfmt | Passed |
| Facade feature matrix and independent consumer | Passed |
| Offline feature scenarios | **20/20 passed** |
| Store and telemetry targeted regressions | **5 + 4 passed** |
| Cargo deny 0.20.2 | advisories, bans, licenses, sources passed |
| Python purity/replay tests | **4 + 17 passed** |
| Pinned Go store fixture, source lock, inventory and overlay | Passed |

The dependency review added exact duplicate-version exceptions for `rand` and
`rand_core` 0.9.5: OpenTelemetry SDK 0.31 requires them while Postgres requires
0.10. No advisory or license failure was waived. The older locally installed
cargo-deny 0.18.4 could not parse a CVSS 4 advisory; the repository's already-pinned
0.20.2 checker was installed and used instead. Direct compiler/Clippy binaries
avoid the worker's missing `/proc/self/exe`; no new compiler was substituted for
project verification.

The 30 ignored Rust tests remain unverified, not passing. The five skipped Go
records are the MCP subprocess helper, Linux automatic confinement/daemonized
child checks, and two Darwin Seatbelt checks (exact names are in the reference
report). No live collector, provider credentials, external Postgres/pgvector or
non-Linux target was verified in this continuation. Fresh independent Rust code
re-review has not been obtained.

The remaining trace work is **producer parity**, including the complete Go
schema-2 hook/span writer. Constructor defaults now have offline coverage, but
Go stdout JSON format, complete span attributes and trace-ID callbacks are not
implemented. Higher-level runtime/session/guardrail helper parity and complete
acceptance-ID Rust closure remain unfinished implementation, not external
blockers. This continuation must not be represented as completion of #11.
