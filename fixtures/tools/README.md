# SDK tool result fixtures

Source: Grateful Agents SDK v0.0.115, commit
`1dc92b73900fac74dc357a938e4b5eee6392b418`, GPL-3.0-only.

`signals.json` contains deterministic cases for `think`, `AskUserQuestion`,
`present_plan` and `finish`. The Rust test executes each case and compares text,
error status and pause status. `verify-signals.go` verifies these same cases by
executing the actual pinned SDK tools (not a rewritten reference algorithm):

```sh
cd repos/sdk
go run ../../fixtures/tools/verify-signals.go
```

This is a small, explicit subset of tool behavior, not the full security corpus.
Rust malformed-input diagnostics are not yet byte-for-byte Go diagnostics.
Plan artifact and namespace-memory tests use injected recording/in-memory stores;
project-state integration tests use the existing real filesystem/SQLite stores.
The original project-state Go comparison fixtures remain in `../project-state`.
