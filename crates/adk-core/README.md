# adk-core

Runtime-neutral native contracts; no provider, agent execution loop, networking,
Tokio dependency, credentials, or Go wire compatibility implementation.

- `Agent`, `Model`, optional `StreamingModel`, `Tool`, `Host`, and `Cancellation`
  are object-safe. Operations return borrowing, boxed `Send` futures.
- `Context` supplies a read-only cancellation signal and monotonic deadline.
  Implementations must interrupt in-flight I/O themselves, not merely call
  `check_active` once. `ToolContext` is host-trusted, not model-controlled.
- `RunItem` preserves ordered messages, tool call/result IDs and handoffs.
  `RunError` retains a partial `RunResult`, including rewritten replay history,
  generated items, responses, usage, and pending approvals.
- `ToolPolicy` evaluates exact names and read-only mutation exceptions before
  approval. Denial wins. Tool-required approval cannot be disabled by host policy.
  Policy declarations do not implement filesystem confinement or timeouts.
- DTOs derive Serde and JSON Schema. Their native representation deliberately
  differs from Go JSON; `adk-codec` owns compatibility serialization.

`RunPolicy` requires an explicit nonzero turn budget. Neither policy nor traits
claim a working run capability. Hosts implement event delivery and approval;
providers and runners are separate follow-up implementations.

Dependencies: workspace `serde`, `serde_json`, `schemars`, `thiserror`.

```sh
cargo test -p adk-core
cargo clippy -p adk-core --all-targets -- -D warnings
cargo doc -p adk-core --no-deps
```
