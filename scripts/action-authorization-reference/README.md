# Pinned action authorization reference

Run `python3 scripts/action-authorization-reference/run.py --check` to reproduce
observations from a disposable archive of SDK commit
`1dc92b73900fac74dc357a938e4b5eee6392b418` (GPL-3.0-only).
The generator records hashes of source and harness inputs and the Go version.
It executes original normalization, nil-authorizer and runner decision regressions,
plus 80 normalization and 72 runner cases (including exact raw argument strings).

Native tests compare these observations without network/provider credentials.
The malformed-input approval case deliberately verifies the stricter native
journal rejection rather than claiming matching source in-memory behavior.
See `docs/execution-security.md` for native policy, lifecycle and replay boundaries.
