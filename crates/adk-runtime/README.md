# adk-runtime

Opt-in Tokio ownership primitives, not an agent runner or Tokio runtime builder.
Core users need not depend on this crate.

`CancellationToken` implements the core read-only `Cancellation` trait, with
latched cancellation, race-free waits, and parent-to-child propagation. A child
cannot cancel its parent. Clones share the same cancellation authority.

`TaskGroup` exclusively owns a `JoinSet<()>` and a child cancellation scope:

- `spawn` receives a child token; no detachable task handle is exposed.
- `join_next` supervises completion and reports task panics/cancellation.
- Drop cancels the scope and aborts remaining tasks. It **cannot join** in Drop.
- `shutdown().await` cancels, aborts, and joins all remaining tasks, returning
  counts plus retained panic errors. Cancelling this future still aborts tasks.
- For cooperative cleanup, cancel `group.cancellation()` and drain `join_next`
  before shutdown. Shutdown itself offers no async-cleanup grace period.

These guarantees require yielding async tasks on a live Tokio runtime. Blocking
threads/processes need a separate termination owner; tasks must not detach child
work. Neither cancelling a token nor dropping an owner rolls back external effects.

Dependencies: workspace `adk-core`, `tokio` and `tokio-util`.

```sh
cargo test -p adk-runtime
cargo clippy -p adk-runtime --all-targets -- -D warnings
cargo doc -p adk-runtime --no-deps
```
