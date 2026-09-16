# Issue #3: Go baseline and offline replay

## Pins / environment

- SDK: `1dc92b73900fac74dc357a938e4b5eee6392b418` = local tag v0.0.115.
- Platform: `08e65c970830f05042c251bcbb46ec6a9e3719b9`; depends on SDK v0.0.115.
- Runtime: Go **1.26.8 linux/amd64**, Python **3.13.5**. Modules declare SDK Go
  1.26.2 / platform Go 1.26.4. Explicit GOROOT `/usr/local/go` required here.
- Baseline flags: `-mod=readonly -p=2 -count=1 -timeout=180s -json`;
  `GOMAXPROCS=2 CGO_ENABLED=0 GOTOOLCHAIN=local GOTELEMETRY=off`.
- Credential-free environment allowlist and empty HOME; `GRATEFUL_LIVE_TESTS=skip`.
  Dependency downloads were allowed in baseline setup only. Regeneration verified
  with `GOPROXY=off GOSUMDB=off`; checked-in Python replay is fully offline.
- Both attached repositories remained unchanged. All new files are confined to
  `docs/migration/baseline/`, `fixtures/`, `scripts/replay/`.

## Commands and fresh results

From workspace root:

```sh
python3 scripts/replay/run_baseline.py sdk-core sdk-session platform-state platform-transcript
python3 scripts/replay/export_fixtures.py --offline
python3 scripts/replay/export_fixtures.py --offline --output scripts/replay/.work/second
cmp fixtures/sdk.json scripts/replay/.work/second/sdk.json
cmp fixtures/platform.json scripts/replay/.work/second/platform.json
python3 -m unittest discover -s scripts/replay -p 'test_*.py' -v
python3 scripts/replay/replay.py
```

| Suite | Scope | Passing test/subtest events | Fail / skip | Exit |
|---|---|---:|---:|---:|
| sdk-core | all `internal/agent`, `pkg/agentsdk/events`, `projectstate`, `policy` | 401 | 0 / 0 | 0 |
| sdk-session | selected public session/conversation/chat-loop/user-input tests | 20 | 0 / 0 | 0 |
| platform-state | all `api/platform/v1alpha1`, `internal/projectstate` | 21 | 0 / 0 | 0 |
| platform-transcript | selected transcript roundtrip/image stripping/unsupported item tests | 3 | 0 / 0 | 0 |
| Python harness | reference comparison, ordering/identity mutations, strict JSON, normalization | 17 | 0 / 0 | 0 |
| Go → Python replay | 7 cases across model/tool/event/state/platform serialization | 7 | 0 / 0 | 0 |

**445 Go test/subtest pass events, no selected-test failures.** Nested subtests
count separately; this is not 445 independent top-level tests or a coverage percent.
The initial smoke run also passed events/projectstate/policy; mode had no test files.
Independent normalized Go exports were byte-identical for both fixture documents.

Machine-readable `*.json` reports retain complete argv, revision, environment,
exit status, wall duration and package/test counts. `*.jsonl.gz` are full,
losslessly compressed combined Go JSON output plus download/build diagnostics,
not excerpts. Read with `gzip -cd docs/migration/baseline/sdk-core.jsonl.gz`.
`replay-tests.log`, `replay.log`, exporter logs, and `verification.log` preserve
local verification. `sdk-tests.log/json` retain the initial smoke invocation.

## Failures / limitations / remaining risks

- **Environment failure, worked around:** bare `go version` initially failed:
  `go: cannot find GOROOT directory: 'go' binary is trimmed and GOROOT is not set`.
  It also warned `failed to start telemetry sidecar: os.Executable: readlink
  /proc/self/exe: no such file or directory`. Explicit GOROOT fixed execution;
  the sidecar warning was nonfatal. No repository fix was made.
- `replay-red.log` records the initial missing-module failure before writing the
  Python reference. This is harness-development RED, **not** a regression in Go.
  GREEN output follows; no production implementation was changed.
- Review found and fixed a harness-only timestamp ordering bug: RFC3339Nano
  fractional trailing zeros made raw string sorting non-chronological. Exact
  second/nanosecond keys now preserve order and equal instants; three regression
  tests bring the suite to 17. A fresh offline Go export after the fix matched
  both existing fixture files and the provenance manifest byte-for-byte.
- **High / not claimed:** no full `go test ./...`, live provider/API equivalence,
  real PostgreSQL, controller envtest, Kubernetes/Kind, browser/e2e, frontend,
  real model streaming, compaction-provider wire formats, or deployed integration.
  Provider credentials were not supplied. Modules may use localhost test servers.
- **High / incomplete migration coverage:** seven fixtures are a representative
  seed, not a comprehensive migration acceptance suite. No request token-estimator
  fixture, streamed delta ordering/backpressure/cancellation trace, interrupted-run
  resume/denial fixture, parallel tool execution, or scheduler lifecycle fixture.
  Existing core Go tests cover some of these behaviors but do not establish a
  foreign implementation's parity.
- **Medium:** state reference supports only initialized/created/claimed/closed
  events and the ready-ID projection used here. It does not implement memory,
  arbitrary task patches, corrupted-log recovery, locks, multi-ready tie-breaks,
  SQL semantics or the whole durable runner. Baseline includes filesystem/SQLite
  lifecycle and torn-tail tests, but fixture parity for them remains future work.
- **Medium:** no `-race`, coverage percentage, repeat/flakiness campaign, stress,
  throughput benchmark or exact original Go minor-version matrix. Selected tests
  are predominantly unit tests with local filesystem/SQLite integration; no
  artificial 70/20/10 claim is made for this deliberately offline baseline.
- JSON contract is strict (including false/null/absence and integer-vs-float
  lexical serialization); corpus values are small integers. See normalization
  policy before adding large integers, arbitrary metadata or non-UTC timestamps.
- License review required before redistributing/relicensing: SDK GPLv3 and
  platform AGPLv3 originals and commit-addressed provenance are retained in
  `fixtures/licenses/`, `fixtures/NOTICE.md`, `fixtures/manifest.json`.

Next useful acceptance fixtures: approval denial/resume, multi-call interleaved
streaming, unknown persisted item rejection, task dependency cycles/patches, and
failure/cancellation propagation. They should be exported from actual pinned Go
behavior and fail against an intentionally mutated foreign adapter.
