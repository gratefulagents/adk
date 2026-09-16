# Offline Go → Python migration reference

Run from the workspace root. Python 3 standard library only; no provider, database,
Kubernetes cluster, credentials, pip, npm, or network needed for checked-in replay.

```sh
python3 scripts/replay/replay.py
python3 -m unittest discover -s scripts/replay -p 'test_*.py' -v
python3 scripts/replay/replay.py --emit > scripts/replay/.work/candidate.json
python3 scripts/replay/replay.py --candidate scripts/replay/.work/candidate.json
```

Create `.work` first when using the optional candidate command in a fresh checkout:
`mkdir -p scripts/replay/.work`.

## Cross-language contract

The two fixture documents are versioned JSON, with `{name, operation, input, expected}`
cases. Implement the named operations in any language, in array order. Supply a
single JSON object mapping **every case name** to its computed output to
`--candidate`. Missing/extra cases fail. Inputs and expected outputs must not be
altered to accommodate a candidate. The included Python implementation computes
outputs independently from inputs; it does not echo fixture goldens. The goldens
were computed by pinned Go code, not manually authored.

For a one-operation subprocess protocol, send `{"operation": "...", "input": ...}`
to `python3 scripts/replay/replay.py --evaluate` on stdin; stdout is output JSON.
Exit zero means success; invalid input or a mismatch exits nonzero. This is a
bounded reference of five operations, **not** a complete SDK/runtime port.

## Refresh from Go

```sh
python3 scripts/replay/run_baseline.py sdk-core sdk-session platform-state platform-transcript
python3 scripts/replay/export_fixtures.py --offline
python3 scripts/replay/export_fixtures.py --offline --output scripts/replay/.work/second
cmp fixtures/sdk.json scripts/replay/.work/second/sdk.json
cmp fixtures/platform.json scripts/replay/.work/second/platform.json
```

Go 1.26.4+ and locally available dependencies are required to regenerate. The pinned
modules declare SDK 1.26.2 and platform 1.26.4. This environment used Go 1.26.8,
`GOROOT=/usr/local/go` (override supported), `GOTOOLCHAIN=local`, `CGO_ENABLED=0`,
`GOMAXPROCS=2`. `--offline` sets GOPROXY/GOSUMDB off; it intentionally fails on a
cold cache. For dependency bootstrap only, omit `--offline`. Baselines may download
Go modules; they never opt into provider tests. Environments are credential-free
allowlists with a new empty HOME. `GRATEFUL_LIVE_TESTS=skip` is explicit.

`export.go` runs in the SDK module without modifying it. Platform export uses a
Go overlay to inject a virtual test in package `main`, permitting use of the real
private transcript codec and existing `sampleTranscriptItems`. Neither repository
gets a tracked or untracked source edit. `.work/overlay.json` contains local absolute
paths and is ignored; it is not a portable artifact. SDK filesystem state is created
in a temporary directory and removed. Static model names and tool commands are
fixture content only; **no model provider or tool command is executed**.

## Comparison and normalization policy (v1)

* Object key order and JSON formatting/escaping are immaterial. Arrays are never
  sorted by normalization. Messages, tools, event sequences and dependencies retain
  their original ordering. Duplicate keys, NaN and Infinity are rejected.
* Missing, null, empty, false and zero remain distinct. Strings are exact; no text
  trimming, substring redaction, enum translation, or blanket field deletion.
  Numeric comparison is deliberately strict (`1` and `1.0` serialize differently);
  corpus integers fit JavaScript's safe integer range. Do not infer float tolerance.
* Static model/tool/call/reasoning IDs are retained verbatim. Synthetic signatures
  and encrypted-content test strings are public source literals, not credentials.
* Only `state_ready` fixtures are normalized. Generated event/task/comment IDs are
  assigned separate first-occurrence namespaces; the same mapping is applied to
  ID/reference fields (`id`, `event_id`, `depends_on`, `blocks`, `task_id`,
  `depends_on_id`) and expected ready IDs. References are never independently hashed.
* Known state timestamps (`time`, `at`, `created_at`, `updated_at`, `closed_at`) map
  distinct chronological instants to distinct 2000-01-01 nanosecond instants.
  Equality and relative chronology survive; wall-clock date and duration do not.
  UTC RFC3339Nano timestamps are parsed as calendar seconds plus exact integer
  nanoseconds (not float or raw lexical order); trimmed fractional zeros are safe.
  Equivalent fractional precisions map to the same instant. Unsupported timezone
  formats, invalid dates and greater-than-nanosecond precision are rejected. This
  is not a general heterogeneous-timezone normalizer.
* Generated `state_dir` becomes `/fixture/state`. Workdir is explicitly synthetic
  `/fixture/repo` at creation. No user data or environment snapshot enters fixtures.
* Normalization is an **export-only** operation, not an escape hatch for candidate
  comparisons. Candidates must implement the normalized corpus contract exactly.

Tests prove ordering/identity changes fail, null/false/absence remain distinct,
normalization is idempotent and relationships survive. State replay intentionally
rejects unknown operations/events and dangling dependencies rather than claiming
forward compatibility. Ready-list sorting is behavior of the reference operation,
not normalization of input arrays. The current state case has one ready task;
multiple-ready tie-breaking is not claimed as coverage.

See `docs/migration/baseline/README.md` for baseline scope and limitations and
`fixtures/NOTICE.md` / `fixtures/manifest.json` for provenance and licensing.
