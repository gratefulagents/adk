# ADK

Rust-native agent development foundation, with reusable contracts separated from
platform integration. **This is not a production agent runner.** Providers, tool
execution, sandbox backends, MCP transport, durable storage and deployment remain
future work; no placeholder implementation is advertised as a capability.

## Workspace

- `adk-core`: native agent/model/tool/host interfaces, messages, runs, events,
  policies and errors with partial results.
- `adk-codec`: explicit Go compatibility codecs, separate from native types.
- `adk-runtime`: optional Tokio cancellation and owned-task resource management.
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
| Runtime | `cargo test --locked -p adk --no-default-features --features runtime --all-targets` | Add Tokio ownership, no runner |
| All | `cargo test --locked --workspace --all-features --all-targets` | Include codecs, runtime, platform boundary and binaries |

The facade's default feature set is empty. `compat` adds SDK codecs; `runtime`
adds the task owner. Neither adds Kubernetes/platform code. Purity checks walk
the resolved transitive all-feature Cargo graph, with negative tests for indirect
platform/Kubernetes dependencies.

```sh
cargo run --locked -p adk --example standalone --no-default-features
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

Dependency/advisory checks require network access and deliberately track the
current RustSec database. Compiler and dependency changes require a reviewed PR,
the full matrix, fixture replay, schema review and `cargo deny` again. Licensing
and inherited upstream notices are in [NOTICE.md](NOTICE.md).
