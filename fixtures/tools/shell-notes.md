# Shell / Terminal replay notes

Implementation: `crates/adk-tools/src/shell.rs`, `shell_buffer.rs`, `shell_policy.rs`, `shell_session.rs`.

## Host integration

- Export `pub mod shell` from the tools crate and keep its `adk-sandbox` dependency.
- Construct `shell::ShellBundle::new(shell::Config { sandbox, access, git_remote_writes, environment })`; inject `bundle.tools()` into registry composition. Keep the bundle alive for the run. Await `bundle.close()` before stopping the Tokio runtime. Drop signals cancellation even if tool handles survive.
- `shell::definition(name, access, &shell::Limits::from_environment(&trusted_environment))` supplies the expected dynamic Bash/BashStart definition, including all three Bash read-only flags. Static shell definitions are copied from the pinned manifest. Registry must not reject dynamic `definition: null` entries before checking supplied implementations.
- The environment map is trusted configuration for `GRATEFUL_BASH_DEFAULT_TIMEOUT_MS`, `GRATEFUL_BASH_MAX_TIMEOUT_MS`, and `GRATEFUL_BASH_MAX_OUTPUT_BYTES`; it is not forwarded to subprocesses. Approval remains the runtime's exact-call responsibility, as required by `ToolContext`; tool-local name/access denial still runs before execution.
- No subprocess is spawned directly by tools. All executions use `Executor::start_session`. FullAccess requires an explicitly configured Local backend; restricted variants never fall back to Local. GitRemoteWrites disabled fails closed without filesystem enforcement.

## Verification and oracle

`fixtures/tools/shell-generate.go` generates `shell.json` from the pinned Go SDK: 168 policy decisions/reasons and four environment/schema configurations. From `repos/sdk`:

```
GOROOT=/usr/local/go GOTELEMETRY=off go run ../../fixtures/tools/shell-generate.go > ../../fixtures/tools/shell.json
```

`tests/shell.rs` and `tests/terminal.rs` exercise policies, schemas, head/tail output caps, incremental/non-incremental polling, input EOF, exit/timeout contracts, aborted-call reaping, bundle/drop cancellation, PTY size/state/control keys, list/read/send/kill, and access restrictions. Real-process tests explicitly configure FullAccess + Local + network Allow, never Local for a restricted request. Until shared module wiring lands, these integration tests compile the owned shell module by path and re-export the pinned catalog.

## Streaming API constraints

- Sandbox output retention is bounded **between polls**. A continuous collector builds a second bounded head/tail buffer (or terminal sliding window), so `wait()` returning only unread bytes never erases prior output. Sandbox-level loss is reported explicitly; a produced-byte count then is only a lower bound.
- `start_session` exposes no spawn-ready handshake. Synchronous failures are reported immediately; a backend/spawn failure after session creation appears as BashPoll `status: error`. Terminal start reports it if supervision has finished during its initial wait. BashStart cannot guarantee Go's immediate spawn-error response without an additional readiness API.
- Cleanup guarantees are those of `adk-sandbox`: awaited direct-child reaping and owned process-group cancellation. Local processes that escape that group (including hostile interactive job-control descendants) require stronger sandbox/session cleanup from the backend; the tools do not add unconfined process scanning or signaling.
- The static classifier is defense in depth, not a shell interpreter or OS boundary. It additionally treats newline statements as separators and inspects literal `printf/echo | shell` command bodies in enforced restricted mode rather than reproducing those SDK blind spots.
