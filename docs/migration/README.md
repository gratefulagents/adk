# ADK behavioral compatibility baseline

Baseline **adk-go-v0.0.115-1** establishes the implementation gate for [issue #3](https://github.com/gratefulagents/adk/issues/3), under [epic #1](https://github.com/gratefulagents/adk/issues/1). It inventories existing behavior and supplies representative offline Go reference vectors. **No Rust runtime or production deployment is delivered here.**

## Start here

| Artifact | Purpose |
|---|---|
| [Source lock](source-lock.json), [version policy](version-policy.md) | Full revisions, dependency hashes, SDK tag reconciliation, later-fix tracking and upstream license notices |
| [SDK ledger](ledger/README.md) | Complete tracked-source/API/package/tool/schema/default/CLI/config/example/test/eval/OS inventory, proposed Rust destination and owner, acceptance IDs and status |
| [Platform boundary and ABI](platform/baseline.md) | Agent dependency closure, worker/control-plane/shared ownership, commands/env/mounts, Kubernetes, Postgres, durable state, transcripts and encrypted workspace contracts |
| [Go baseline results](baseline/README.md) | Reproducible selected-suite commands, original output, failures and limitations |
| [Fixtures and provenance](../../fixtures/NOTICE.md) | Sanitized deterministic actual-Go model/tool/event/state/transcript reference vectors and original licensing |
| [Cross-language replay](../../scripts/replay/README.md) | Offline reference implementation, candidate protocol and precise normalization rules |
| [Rust research](rust-research.md) | Pinned inspected Rig, genai and ADK-Rust sources; useful idioms and rejected alternatives |
| [Decisions](decisions.md) | Explicit compatibility decisions, unverified contracts and responsible follow-up roles |

## What is locked

- SDK checkout and `v0.0.115` both resolve to `1dc92b73900fac74dc357a938e4b5eee6392b418`; platform requires that exact tag. There are no later checkout fixes to reconcile.
- Inspected platform is `08e65c970830f05042c251bcbb46ec6a9e3719b9`. Epic pin `67fcfff804930a6679976aea97124cc7aa04e500` differs only by three Android CI lines, not worker contracts. Both identities and the delta are recorded.
- Inventory schema v1, fixture schema v1 and upstream durable/storage schema versions are **independent namespaces**. Never infer compatibility from equal version integers.
- Source-derived declarations and fixtures retain SDK GPL and platform AGPL attribution; this baseline does not relicense either project.

## Verify without providers

A fresh clone can run the checked-in replay with only Python 3:

```sh
python3 scripts/replay/replay.py
python3 -m unittest discover -s scripts/replay -p 'test_*.py' -v
```

To verify provenance and regenerate source inventories, obtain both upstream repositories at the exact revisions from `source-lock.json` under `repos/sdk` and `repos/gratefulagents`, including the SDK `v0.0.115` tag and epic platform commit. Then:

```sh
python3 scripts/check-source-lock.py
go run scripts/inventory/main.go -check
go test -v scripts/inventory/main.go scripts/inventory/main_test.go
python3 scripts/inventory/validate.py
```

See the platform document for its generator command and the Go baseline document for selected suite/export commands. The inventory generator uses only the Go standard library. Go-based fixture regeneration requires module dependencies already cached for `--offline`; bootstrapping that cache requires dependency downloads. On the capture host, Go needed explicit `GOROOT=/usr/local/go GOTOOLCHAIN=local`; this is an environment workaround, not a repository requirement.

## Baseline evidence versus migration acceptance

- SDK inventory covers all tracked source files and assigns all indexed records. Source expressions preserve dynamic schemas/defaults rather than inventing evaluated values. Go type-system effective method sets and symbol-level code coverage are not claimed.
- Local platform import closure includes non-SDK helpers and all build-tag variants. Package reachability is not a demand to port control-plane reconciliation or a symbol-level call graph.
- Selected Go suites passed **445 test/subtest events** (nested subtests counted separately), with no failures/skips. This is not the full upstream test suite.
- **7 Go-derived cases** replay in an independent Python reference; **17 harness tests** check strict comparisons, order, identity and normalization. Two independent Go exports were byte-identical.
- Rust implementation and verification remain pending. Acceptance IDs and proposed modules/role owners are traceability obligations, not fabricated passing tests. Use the ledger's existing Go test links and recorded baseline scope to determine what has actually run.
- Live provider/API behavior, real Postgres concurrency, Kubernetes integration, multi-OS sandbox parity and comprehensive streaming/recovery scenarios remain explicitly unverified. These are mapped follow-up contracts, not silently dropped capabilities.

Compatibility applies to observable behavior, security and wire/storage contracts. Implementations should use idiomatic Rust ownership, enums, traits, explicit resource lifetimes and cancellation; they need not reproduce Go package structure.
