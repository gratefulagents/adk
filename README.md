# ADK

Rust-native agent development kit, with a standalone execution engine and reusable
contracts separated from platform integration. The opt-in `runtime` feature runs
host-supplied models and tools, including streamed execution and approval
continuations. The opt-in `execution` feature adds sandbox backends, subprocess
ownership, and policy/guardrails. Opt-in `durable` and `project-state` features
provide compatible persistence and project memory. Opt-in `mcp` adds bounded MCP
client transports and host-policy-gated server mode; see [MCP integration](docs/mcp.md).
Deployment remains separate work; this is not a production platform worker.

## Workspace

- `adk-core`: native agent/model/tool/host interfaces, messages, runs, events,
  policies and errors with partial results.
- `adk-codec`: explicit Go compatibility codecs, separate from native types.
- `adk-runtime`: optional Rust state-machine runner, tool-output safety, streaming,
  approval continuation, and Tokio cancellation/owned-task resource management.
- `adk-durable`: versioned Go-compatible documents, fenced filesystem/Postgres
  stores, snapshot CAS, immutable events and conservative effect recovery.
- `adk-project-state`: event-sourced project tasks and memories, filesystem/SQLite
  stores and separate embedding-assisted recall.
- `adk-mcp`: pinned configuration, bounded MCP client transports, discovery,
  reusable tools/resources and authenticated, policy-gated Streamable HTTP server mode.
- `adk-security`: composed permissions, command and secret guardrails, exact-call approval.
- `adk-sandbox`: Linux Bubblewrap/macOS Seatbelt and explicit full-access local
  execution, with owned process groups and PTY lifecycle.
- `adk`: public facade; no platform dependency, even with every feature enabled.
- `adk-platform`: one-way integration boundary and platform fixture codec.
- `adk-agent`: foundation inspection binary (`--version` only), **not a worker**.
- `adk-harness`: offline Go-baseline replay, not a live SDK runner.

See [architecture and dependency decisions](docs/architecture.md) for crate/domain
boundaries, research, cancellation ownership, license decisions and limitations.
The [migration baseline](docs/migration/README.md) contains source pins, upstream
contracts, original Go fixture provenance and unverified migration obligations.

## Reproducible development

Install [rustup](https://rustup.rs/); `rust-toolchain.toml` pins Rust **1.88.0**,
Clippy and rustfmt. Commit `Cargo.lock`; use `--locked` in CI and release builds.
The first build fetches crates from crates.io. `cargo fetch --locked` prepares
an offline cache; subsequent commands can add `--offline`. A lockfile is source
reproducibility, not a claim of bit-identical binaries across hosts.

```sh
cargo test --locked --workspace --all-features --all-targets
cargo test --locked --workspace --all-features --doc
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
python3 scripts/check-purity.py
python3 -m unittest discover -s scripts -p 'test_purity.py' -v
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --locked check
```

### Feature matrix (all sets run in CI)

| Set | Command | Meaning |
|---|---|---|
| Minimal | `cargo test --locked -p adk --no-default-features --all-targets` | Native contracts only |
| Default | `cargo test --locked --all-targets` | Default workspace members: facade, core, SDK codecs |
| Runtime | `cargo test --locked -p adk --no-default-features --features runtime --all-targets` | Add standalone runner and Tokio ownership |
| Builder | `cargo test --locked -p adk --no-default-features --features builder --all-targets` | Add host configuration and provider/tool/session composition |
| Observability | `cargo test --locked -p adk --no-default-features --features observability --all-targets` | Ordered hooks/events and private local traces |
| OpenTelemetry | `cargo test --locked -p adk --no-default-features --features otel --all-targets` | Host-owned tracer bridge, no implicit exporter |
| Execution | `cargo test --locked -p adk --no-default-features --features execution --all-targets` | Add opt-in sandbox, process ownership and security |
| All | `cargo test --locked --workspace --all-features --all-targets` | Include codecs, runtime, execution, platform boundary and binaries |

The facade's default feature set is empty. `compat` adds SDK codecs; `runtime`
adds the execution engine and task owner. `execution` adds sandbox and security
APIs; see [execution security](docs/execution-security.md) for trusted-host usage,
OS support, enforced CI tests and limitations. None adds Kubernetes/platform code.
See [durable runs and project state](docs/durable-runs.md) for fenced store
ownership, compatible Go checkpoints, recovery limits and memory APIs.
See [runner design and behavior](docs/runner-design.md) for lifecycle contracts,
Go compatibility mapping, research and explicit integration boundaries.
See [managed subagents](docs/subagents.md) for session ownership, DAG tools,
security narrowing, steering and conservative scheduler recovery. Purity checks walk
the resolved transitive all-feature Cargo graph, with negative tests for indirect
platform/Kubernetes dependencies.

## Embedding and composition

The public [runtime builder](docs/runtime-builder.md) composes host-supplied
models, tools, configuration and explicit bundle/session ownership via the
`builder` feature (which enables `providers-runtime` and `tools`). It does not
discover credentials or start platform services. [Observability](docs/observability.md)
adds ordered event/progress delivery, private trace persistence and an optional
OpenTelemetry bridge. The `observability` feature enables the local adapters;
`otel` additionally accepts a host-owned OpenTelemetry tracer. Exporter configuration,
flush and shutdown remain the embedding application's responsibility.

See the [20 offline feature examples](docs/feature-examples.md),
[Rust API research](docs/research/facade.md), and
[issue #11 verification and remaining blockers](docs/verification/issue-11.md).
Example coverage is not a declaration of full SDK behavioral parity.
The [CLI scope](docs/sdk-cli.md) explicitly excludes porting `grateful-agent-run`
and evaluation/Terminal-Bench adapters.

An independent workspace consumer builds every reusable facade feature without
depending on either development binary or `adk-platform`:

```sh
cargo run --locked --manifest-path fixtures/standalone-consumer/Cargo.toml
sh scripts/check-feature-examples.sh
```

CI checks each named facade feature independently, the runtime/tools/providers
composition, and the all-feature workspace. For a prepared dependency cache, use
`--offline`; the example script makes no live provider calls. Live provider,
OAuth and remote-service verification must be reported separately from offline
tests, and absent credentials mean **unverified**, not passed.

```sh
cargo run --locked -p adk --example standalone --no-default-features
cargo run --locked -p adk --features runtime --example runner
cargo run --locked -p adk-agent -- --version
cargo run --locked -p adk-harness -- --fixtures > candidate.json
python3 scripts/replay/replay.py --candidate candidate.json
python3 -m unittest discover -s scripts/replay -p 'test_*.py' -v
```

The harness also accepts one `{"operation":"...","input":...}` JSON request on
stdin. It computes results from inputs; it never reads fixture `expected` fields.
Fixture comparisons preserve absent/null/false/empty differences, array order,
UTF-8 and numeric representations. See codec tests for bounded edge coverage;
passing seven reference cases does not establish full Go SDK or platform parity.
The additional [runner replay](scripts/replay/runner_README.md) executes the real
Go and Rust engines over 46 deterministic normal/streaming scenarios,
comparing dispatch, semantic event order, history and final/partial outcomes:

```sh
cargo test --locked -p adk-runtime --test replay
python3 scripts/replay/runner_export.py --check
```

Regenerating the Go reference requires the pinned SDK checkout and a warm Go
module cache (or explicit `--allow-downloads`). Rust replay tests are offline once
Cargo dependencies have been fetched.

Dependency/advisory checks require network access and deliberately track the
current RustSec database. Compiler and dependency changes require a reviewed PR,
the full matrix, fixture replay, schema review and `cargo deny` again. Licensing
and inherited upstream notices are in [NOTICE.md](NOTICE.md).
