# Pinned Go subagent reference findings

Reference: SDK **v0.0.115**, commit `1dc92b73900fac74dc357a938e4b5eee6392b418` (`repos/sdk`, verified HEAD + exact tag, clean tracked tree). Actual executable generator runs with **Go 1.26.2**. No production edits by the reference-fixture owner.

## Final strict reference result

**6 scenarios / 44 actual Go tool calls** were exported, then replayed through real Rust tools, Scheduler, RunnerChildExecutor and Runner. Go regeneration `--check` passes unchanged. Rust comparison suite: **6 passed, 0 failed, 0 ignored**.

The independent gates compare provenance, all four complete input schema JSON objects, base tool metadata, complete tool-result fields/diagnostics, scheduler outcome projections, and actual child model-request projections. The initial comparison exposed schema/envelope, duration, read-only metadata and diagnostic drift; the same strict assertions pass after bounded contract fixes. No flags, missing fields, errors or schema annotations were normalized away. Durations now derive from observed admission/termination, and provider diagnostics carry the actual model turn. Read-only adapters retain inherited narrowing while base tool flags match Go.

Separately, **103 original pinned Go tests passed, 0 failed, 0 skipped**, across internal agent, public SDK and runtime SessionState packages. Their exact command, individual outcomes, source hashes and compressed raw log are retained under `docs/verification/subagents/`. These upstream tests are executed reference evidence, not additional differential scenarios. See [the acceptance map](verification/subagents/acceptance-map.md) for exact per-requirement coverage and limitations.

## Artifacts

- `crates/adk-runtime/tests/fixtures/subagents.go`: executable importing public `pkg/agentsdk` exports. Real Go Runner, scheduler and tools; controlled fake Model responses/errors, channel gates and scheduler barriers, no sleep-based ordering.
- `crates/adk-runtime/tests/fixtures/go-subagents.json`: generated, never hand-authored expected responses. Includes SDK version/revision, Go version, exact module-relative command/environment, generator/exporter SHA256 and reference source hashes.
- `scripts/replay/subagent_export.py`: pin/tag/clean-source validation, credential-isolated HOME, offline by default, `--check` regeneration gate.
- `crates/adk-runtime/tests/subagent_reference.rs`: real Rust execution with fake Models; inputs loaded from generated fixture, no separately authored Rust expectations. Independent schema, metadata, complete tool-result, scheduler-projection, model-request and provenance tests.

## Coverage and limits

| Scenario | Exercised behavior |
|---|---|
| `single_sync` | Single sync result, explicit already-delivered wait-any, terminal views |
| `dag_sync_isolation` | Keyed three-node DAG; call read-only + task full remains narrow; one child excludes dependency evidence while the next includes it |
| `dependency_failure` | Failing Model; all_success dependent never calls its Model; all_terminal dependent runs successfully |
| `background_wait_any` | Two gated Models; first released/completed while second remains active; wait-any returns first; wait-all returns second; explicit reread lists previous delivery |
| `cancel` | Cancel after Model has started, wait for cancelled status, inspect retained error |
| `dag_background` | Background DAG ids/edges; dependent Model starts after root completion and sees dependency evidence |

Every scenario invokes terminal summary/graph/results, rereads results, and checks empty wait. Model observations compare mutation-tool exposure and presence/absence of dependency evidence. Gated Models use identical logical release/started/settle steps in both runtimes.

**Not covered by this comparator:** nonempty parent-history isolation/sharing; automatic parent result injection/finalization; steering; timeouts; activity details; restored/reconciling scheduler behavior. Direct-tool harness parent history is empty: `parent_secret=false` is an observation, **not proof of isolation from a seeded parent secret**. Wait-any tests its mixed finished/active delivery snapshot with a result already completed at the barrier; it does not claim to measure wake latency. Upstream tests and existing Rust regressions cover additional behavior separately (parent owns that evidence map).

## Comparison rules

Only generated task IDs (including references inside error strings) and existing `duration`/`started_at`/`timestamp`/`duration_ms` values are normalized in responses. Missing vs null vs empty fields remain distinct. No error strings, flags, status values or response shapes are rewritten. Scheduler projection explicitly compares agent/status/result/error presence/dependency IDs; complete error strings remain independently checked in tool responses. Model projection checks tool names and dependency evidence rather than language-specific RunItem serialization. **Schemas are compared as complete JSON values with no annotation stripping or enum normalization.** Tool prose descriptions are retained in the Go fixture for inspection but are not asserted equal to Rust's shorter tool descriptions.

## Exact reproduction commands

```sh
env GOROOT=/workspace/scratch/go GOPATH=/workspace/scratch/gopath GOCACHE=/workspace/scratch/go-cache GOMODCACHE=/workspace/scratch/go-mod python3 scripts/replay/subagent_export.py
env GOROOT=/workspace/scratch/go GOPATH=/workspace/scratch/gopath GOCACHE=/workspace/scratch/go-cache GOMODCACHE=/workspace/scratch/go-mod python3 scripts/replay/subagent_export.py --check
env PATH=/workspace/scratch/rust-1.88/bin:/usr/bin:/bin LD_LIBRARY_PATH=/workspace/scratch/rust-1.88/lib CARGO_HOME=/workspace/scratch/cargo CARGO_TARGET_DIR=/workspace/scratch/adk-target cargo test -p adk-runtime --test subagent_reference
```

The Python wrapper sets `GOTOOLCHAIN=local`, `GOTELEMETRY=off`, `GRATEFUL_LIVE_TESTS=skip`, `CGO_ENABLED=0`, `GOMAXPROCS=2`, `GOPROXY=off`, `GOSUMDB=off` and runs `go run -mod=readonly ../../crates/adk-runtime/tests/fixtures/subagents.go` from `repos/sdk`. Change `GOROOT`/cache paths for another host; generator records the actual `go version`.

Additional fresh verification: Go `gofmt -l` empty; `go vet` on the generator exits 0; Python AST syntax check clean; Rust `rustfmt` clean; `git diff --check` clean. Standard `cargo clippy` is blocked by the host's absent `/proc/self/exe`; direct Clippy driver workaround **passes** and typechecks the test target with warnings denied:

```sh
env PATH=/workspace/scratch/rust-1.88/bin:/usr/bin:/bin LD_LIBRARY_PATH=/workspace/scratch/rust-1.88/lib CARGO_HOME=/workspace/scratch/cargo CARGO_TARGET_DIR=/workspace/scratch/adk-target RUSTC_WORKSPACE_WRAPPER=/workspace/scratch/rust-1.88/bin/clippy-driver RUSTFLAGS='-D warnings' cargo check -p adk-runtime --test subagent_reference
```
