# CLI scope and standalone embedding

Issue #11 explicitly excludes porting `sdk/cmd/grateful-agent-run` and the
SDK evaluation/Terminal-Bench adapter contracts. Its CLI/evaluation acceptance
bullet is interpreted subject to that prohibition: this work does **not**
introduce a provider-running CLI, platform worker, or benchmark adapter.

Use the Rust library and examples for standalone embedding. Neither the facade
nor its optional features depend on `adk-platform`, Kubernetes, or either binary.
The dependency purity check examines the resolved all-feature graph.

## Existing development commands

```sh
cargo run --locked -p adk-agent -- --version
cargo run --locked -p adk-harness -- --fixtures > candidate.json
python3 scripts/replay/replay.py --candidate candidate.json
cargo test --locked -p adk-agent --test cli
cargo test --locked -p adk-harness
```

`adk-agent --version` exits successfully and identifies itself as a foundation
binary, not a runner. Other arguments (including no arguments) print a diagnostic
to stderr and exit with status 2. It does not read credentials, run agents, or
load platform configuration. This deliberately small interface is regression
tested; it is not compatible with `grateful-agent-run`.

`adk-harness` is the existing deterministic **offline codec fixture replay**
harness. `--fixtures` computes a candidate result for each pinned baseline case;
stdin mode accepts a single JSON operation request. See the [repository
README](../README.md) and [runner replay](../scripts/replay/runner_README.md).
Codec/runner replay is not an evaluation adapter and makes no live-provider or
Terminal-Bench claim.

No production release, default switch, worker migration, or credential discovery
is part of this change. If worker CLI or evaluation compatibility is wanted, it
requires a separately authorized scope rather than silently reintroducing the
explicitly excluded contracts.
