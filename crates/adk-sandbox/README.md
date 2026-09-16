# adk-sandbox (opt-in, no production rollout)

Trusted-host OS execution boundary. It consumes `adk_core::Context` cancellation/deadlines and `AccessMode`; it does **not** authorize tools, approve calls, classify shell commands, or redact output. Policy/configuration must never come directly from model arguments. The host must authorize and check approval before calling this crate. Output is untrusted bytes, including terminal escape sequences.

## API and lifecycle

- `Executor::new(Config::new(absolute_workspace))` validates/canonicalizes the workspace.
- `Request::new(absolute_program)` defaults to read-only, network denied, piped output and no timeout. Set `args`, workspace-relative/absolute `cwd`, `access`, `timeout`, and `output` explicitly.
- `Executor::run(&Context, Request).await` returns bounded stdout/stderr, exit status, truncation flag and `Completion::{Exited, Cancelled, TimedOut}`. An OS backend setup failure may be a non-success exit status; callers **must** inspect status, not just `Result::Ok`.
- `Executor::start` returns `RunningProcess`. `wait().await` waits for completion; `cancel_and_wait().await` cancels and explicitly awaits cleanup. Cancellation before spawn returns `Error::Cancelled` instead of a fabricated exit status.
- Each independently owned Tokio supervisor uses a new session/process group, sends TERM then KILL after the configured grace, and awaits the direct child's reaping on **all** exit paths, including successful leader exit with background descendants. The leader is observed with `waitid(WNOWAIT)` and retained until group signalling finishes, preventing recycled-PID group signalling.
- Dropping a handle or an in-flight `run`/`wait` future requests supervisor cancellation, not supervisor abortion. RAII provides best-effort KILL on unexpected supervisor destruction; Tokio owns fallback direct-child reaping. **Keep the runtime alive** to complete async cleanup; use retained handles and `cancel_and_wait` before runtime shutdown. There is no executor-wide shutdown registry. Host crash, SIGKILL, runtime destruction and power loss cannot guarantee async reaping.
- `OutputMode::Pty { rows, cols }` captures a private PTY with stdout/stderr merged into stdout. It is not an interactive terminal/session API. Pipes have null stdin. Capture keeps the first `output_limit` combined bytes and continuously drains/discards excess. Reader tasks are bounded and joined/aborted after cleanup; inherited descriptors held by escaped local/macOS daemons cannot hang completion forever.

## Containment

### Linux

`Auto` selects the fixed trusted `/usr/bin/bwrap`; missing binary, namespaces, procfs, or capability support fails closed. No root filesystem bind or local fallback exists. Bubblewrap creates user, PID, mount, IPC and UTS namespaces, drops all capabilities, uses a private `/proc`, minimal `/dev`, private `/tmp` and HOME, and denies network with a network namespace unless explicitly allowed. Runtime mounts are `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64` and loader cache/config files. Network opt-in additionally exposes resolver files. Read-only binds remain read-only through nested user namespaces; this does not provide seccomp filtering or prevent kernel attacks.

Workspace-write allows top-level file/directory creation and Git index/object writes. To protect **absent as well as existing** metadata names, the host must first create real (not symlink) mountpoints:

- regular files `.git/config` and `.mcp.json`;
- directories `.git/hooks`, `.codex`, `.claude`, `.gemini`, `.agents`.

The executor never creates these in the host workspace. Missing/wrong-type entries reject workspace-write. `.git` is pinned as a writable mount and config/hooks are read-only submounts, so the parent cannot be renamed out from under protection. Agent configuration mounts are also protected. This prerequisite intentionally narrows baseline compatibility rather than silently allowing creation/replacement of trusted metadata. Read-only execution needs no such prerequisites.

### macOS

`Auto` selects fixed `/usr/bin/sandbox-exec`. A default-deny Seatbelt profile allows only runtime reads, workspace reads, private temporary storage, process creation and same-sandbox signals/process info. Network is denied unless explicitly allowed. Parameters, not profile-string interpolation, carry paths. Known credential entries and protected metadata are excluded from grants; the workspace root and `.git` parent cannot be renamed. Unlike Linux, the profile protects absent names without mountpoint prerequisites.

Every real execution first runs a bounded, lifecycle-managed functional read/write probe: a command must read an allowed fixture, fail writing a forbidden path, and return the expected marker; the host also checks the forbidden path was not created. A failed probe prevents the requested command from starting. Seatbelt is deprecated and must be required/tested on the exact host OS release; Linux tests do not verify it.

### Explicit local and unsupported platforms

Only `Backend::Local` **and** `AccessMode::FullAccess` **and** `Network::Allow` permit unsandboxed execution. Local cannot enforce a network denial or filesystem restriction. `FullAccess` with Auto is rejected rather than silently disabling containment. Other platforms, including Windows, are unsupported and fail closed.

**Process groups do not contain adversarial `setsid`/double-fork daemonization on local or macOS.** A daemon can escape group cleanup there, although Seatbelt filesystem/network restrictions remain inherited. Local and macOS must not be used when complete malicious descendant lifetime containment is required. Linux's PID namespace teardown kills such descendants; the integration suite includes a `setsid` adversary. The supervisor reaps its own direct child, not host-global grandchildren; namespace init/host init owns orphan reaping. No subreaper setting is changed globally.

## Environment, paths and unsupported scenarios

No ambient environment is inherited. The fixed PATH is `/usr/bin:/bin`; HOME/TMPDIR are private. Only constrained locale/terminal overrides are accepted. PATH/HOME/config/search-path/loader/proxy and credential overrides are rejected, including unknown variable names. Git system/global config, prompts, hooks, fsmonitor and pagers are disabled or fixed; this is defense in depth, **not** a Git command authorization policy (repository filters and arbitrary executables can still run within the sandbox).

The mount/read allowlist excludes host homes, `/run/secrets`, host procfs and shared temporary data. Known credential/config entries at workspace root (`.ssh`, `.aws`, `.azure`, `.kube`, `.gnupg`, `.config`, `.docker`, agent credential/config directories, `.netrc`, `.git-credentials`, `.npmrc`, `.env`) are additionally hidden/denied. Arbitrary secrets in ordinary workspace/runtime files are **not** discovered or redacted. Do not place credentials there. Local mode intentionally provides no file confidentiality.

Workspace and cwd are canonicalized and cwd must remain inside the workspace. Root/system/shared-temp workspace roots and `..` paths are rejected. Programs must be absolute existing files; no ambient PATH resolution occurs. Workspace hardlinks and device/FIFO/socket nodes are rejected for enforcing backends, as are symlinks in protected metadata. Ordinary workspace symlinks are interpreted inside the OS boundary, not rebound as host mount sources.

**Exclusive trusted ownership is required from validation through cleanup.** The trusted host must serialize other writers and must not concurrently replace workspace ancestors, insert hardlinks, mount filesystems, move protected paths, or serve privileged Unix sockets inside it. Bind sources are validated paths, not pinned directory descriptors: hostile host-side filesystem races are unsupported. Git linked worktrees with external/symlinked metadata and hardlinked local clones are unsupported. Independent sandbox calls against the same writable workspace must be serialized by the host. Local/macOS escaped daemons invalidate exclusive ownership; discard that workspace instead of reusing it.

This is not a multi-tenant VM: no CPU/disk/process-count quotas, resource-DoS protection, kernel exploit mitigation, egress allowlist, general host-file confidentiality guarantee, or trusted-host adversary protection is provided. Linux requires `close_range(CLOEXEC)` (kernel 5.11+) to close inherited host descriptor capabilities at exec; macOS marks descriptors close-on-exec before exec. Only stdio is intentionally inherited.

## Verification

```
cargo test -p adk-sandbox --all-targets
ADK_REQUIRE_SANDBOX=1 cargo test -p adk-sandbox --test os_backends -- --nocapture
cargo clippy -p adk-sandbox --all-targets -- -D warnings
```

The default OS test prints an explicit `SKIP` diagnostic on backend unavailability. Setting **any** value of `ADK_REQUIRE_SANDBOX` turns that into a failure. Native Linux/macOS CI must set it; argument/profile generation tests alone are not evidence of containment. The single ignored `network_helper` test is a fixture executed *inside* the sandbox by the enforcement suite, not a skipped security assertion. Windows has a separate fail-closed test.
