# Rust execution-security research

Research date: 2026-09-16. This is a design review, not a production security
certification. Candidate versions below were checked against registry metadata
and source; they are not all dependencies of ADK. `Cargo.lock` is authoritative
for the selected implementation. No production rollout is included.

## Source designs and outcomes

### SDK baseline (GPL-3.0-only)

The source-derived contracts/fixtures retain the SDK license and attribution.
Inspected baseline [security documentation](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/docs/security.md)
and [shell classifier](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/tools/shell/classifier.go):
restricted modes fail closed, child policy cannot widen parent authority, exact
mutating exceptions do not transfer to children, environments are explicit,
HOME/TMPDIR are private, and Git remote-write policy applies even with filesystem
containment. Arbitrary host-file confidentiality is not a baseline promise.

### Codex (Apache-2.0; source inspected, not copied)

Inspected immutable commit
[`c56dda711cc13f94625734343cd95d4a4bf496ed`](https://github.com/openai/codex/blob/c56dda711cc13f94625734343cd95d4a4bf496ed/codex-rs/Cargo.toml).
Its workspace version `0.0.0` is not a published release identifier.

- [Bash AST analysis](https://github.com/openai/codex/blob/c56dda711cc13f94625734343cd95d4a4bf496ed/codex-rs/shell-command/src/bash.rs)
  separates extracting literals to find dangerous operations from proving a
  command statically inspectable. Those are different obligations: a parser
  recovering a harmless-looking literal does not authorize expansion around it.
- [Seatbelt profile construction](https://github.com/openai/codex/blob/c56dda711cc13f94625734343cd95d4a4bf496ed/codex-rs/sandboxing/src/seatbelt.rs)
  illustrates structured policy construction, but its shared temporary-directory
  allowances must not be copied into the SDK's private-temp contract.
- [PTY process-group code](https://github.com/openai/codex/blob/c56dda711cc13f94625734343cd95d4a4bf496ed/codex-rs/utils/pty/src/process_group.rs)
  illustrates session/group ownership; a group is not a cgroup or a guarantee
  against a hostile descendant calling `setsid`.

Rig's [tool authoring interface](https://github.com/0xPlaygrounds/rig/blob/main/crates/rig-core/src/tool/mod.rs)
is useful context/tool separation, **not** subprocess enforcement. Its
[manifest](https://github.com/0xPlaygrounds/rig/blob/main/crates/rig-core/Cargo.toml)
declares MIT; these moving-main links are illustrative, not reproducible pins.

## Candidate dependencies

| Candidate inspected | License | Declared MSRV | Supported-platform/implementation considerations |
|---|---|---|---|
| [Tokio 1.53.1](https://crates.io/api/v1/crates/tokio/1.53.1) | MIT | 1.71 | Unix/Windows process APIs; direct-child ownership is not tree containment |
| [nix 0.31.3](https://crates.io/api/v1/crates/nix/0.31.3) | MIT | 1.69 | Unix syscall wrappers; Linux amd64/arm64 and Apple arm64 listed Tier 1 in inspected README |
| [rustix 1.1.4](https://crates.io/api/v1/crates/rustix/1.1.4) | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 1.63 | Owned-FD-friendly Unix operations; Windows principally Winsock, not process/PTY parity |
| [portable-pty 0.9.0](https://crates.io/api/v1/crates/portable-pty/0.9.0) | MIT | Unspecified | Unix PTY and Windows ConPTY; blocking I/O/wait, not an async lifecycle supervisor |
| [tree-sitter 0.25.10](https://crates.io/api/v1/crates/tree-sitter/0.25.10) | MIT | 1.76 | Inspected compatible line, not claimed latest; target C toolchain required |
| [tree-sitter-bash 0.25.1](https://crates.io/api/v1/crates/tree-sitter-bash/0.25.1) | MIT | Unspecified | Bash AST grammar; syntax-tree recovery/error nodes must not authorize unknown constructs |
| [shlex 2.0.1](https://crates.io/api/v1/crates/shlex/2.0.1) | MIT OR Apache-2.0 | 1.46.0 | Word splitting/quoting, **not** a shell AST or policy boundary |

Platform evidence: [nix README](https://github.com/nix-rust/nix/blob/v0.31.3/README.md),
[rustix README](https://github.com/bytecodealliance/rustix/blob/v1.1.4/README.md),
[tree-sitter manifest](https://github.com/tree-sitter/tree-sitter/blob/v0.25.10/lib/Cargo.toml).
Maintained upstream activity/registry releases are evidence of availability, not
a promise of support for every target or a vulnerability audit.

### Selection and tradeoffs

Reuse Tokio and tokio-util already selected by the runtime. The implementation
uses narrowly scoped Unix `libc` operations instead of adding both nix/rustix or
portable-pty. This reduces dependency duplication but increases the importance
of reviewing each unsafe FD/session/signal operation and of native OS tests.
`regex` is used only for bounded secret-pattern detection, not for shell parsing.
Selected and registry-verified versions are [libc 0.2.189](https://crates.io/api/v1/crates/libc/0.2.189)
(MIT OR Apache-2.0, declared MSRV 1.65) and
[regex 1.13.1](https://crates.io/api/v1/crates/regex/1.13.1) (MIT OR Apache-2.0).
Tokio resolves to 1.53.1; tokio-util to 0.7.19. libc's Unix FFI supplies
`openpty`, `setsid`, `waitid`, signal and FD operations; unsupported targets never
invoke them. Exact resolved versions/licenses are recorded by the lockfile and
cargo metadata; `cargo deny` checks the all-feature dependency graph.

The command authorizer deliberately accepts a **smaller literal grammar**, with
quote/escape/separator/redirection tokenization and explicit program/option
classification. Expansion, indirect execution and unrecognized syntax fail
closed even for full-access commands. This is not full Bash compatibility and
must not be silently relaxed to word splitting or substring matching. A future
full-shell implementation should use a structural parser, reject parse/error
nodes and retain these regression cases; adding a parser alone does not solve
runtime expansion or remote side effects.

## Rust ownership and cleanup

[Tokio Child documentation/source](https://docs.rs/tokio/1.53.1/tokio/process/struct.Child.html)
explicitly distinguishes dropping a handle, requesting kill and awaiting reap.
`start_kill()` does not reap. `kill().await` addresses the direct child, not every
descendant. The supervisor should own child, readers, group and private files;
callers own cancellation authority and an awaitable completion boundary.

The selected design uses an independent async supervisor. Normal completion,
explicit cancellation and timeout perform TERM, bounded grace, KILL, direct-child
reap and reader completion. Dropping an awaiting request signals abandonment;
it cannot await cleanup. The Tokio runtime must remain alive for the supervisor
to finish. RAII guards are last-resort synchronous safety, not a claim that Drop
runs an async protocol or reaps arbitrary grandchildren.

[portable-pty source](https://docs.rs/portable-pty/0.9.0/src/portable_pty/lib.rs.html)
was specifically inspected: cloned Unix killer behavior sends SIGHUP to one PID,
not the required tree-wide TERM/grace/KILL sequence. If adopted later, it must be
an adapter under a separate supervisor, not the lifecycle implementation.

## Backend limits

[Bubblewrap v0.11.0 documentation](https://github.com/containers/bubblewrap/blob/v0.11.0/bwrap.xml)
and [README](https://github.com/containers/bubblewrap/blob/main/README.md) describe
namespace/policy machinery, not a complete universal sandbox. ADK invokes a host
binary; it does not vendor or link Bubblewrap. Its
[COPYING](https://github.com/containers/bubblewrap/blob/v0.11.0/COPYING) contains
GNU Library GPL v2; exact per-file license suffixes and binary redistribution
obligations must be reviewed by packagers. The reviewed release is not a claim
that every installed distribution has that exact version. Actual user/mount/PID
namespace support and the generated mount policy must be tested. `--die-with-parent`
and group signals are complementary; neither on its own proves all descendant
cleanup when the workload leader exits.

macOS `sandbox-exec`/Seatbelt is a deprecated compatibility boundary, not a stable
public Apple sandbox API. Runtime launch failure must fail closed; profile
construction needs parameter/escaping tests and functional read/write/network
checks on native macOS. Linux cannot establish Seatbelt enforcement. A passing
profile-string unit test is not evidence that the OS enforced it.

The local full-access backend intentionally offers **no filesystem/network
containment**. Process groups do not prevent a privileged or hostile local
process from escaping its group. Kernel compromise, resource exhaustion, network
egress destinations, all host secrets, and runtime destruction before async
cleanup are not solved merely by owning Rust handles. These limitations must
remain visible in public API documentation and CI evidence.

### Native macOS runtime compatibility

Native macOS 14 arm64 CI exposed a Rust 1.88 startup failure before `main`:
`failed to allocate a guard page: Invalid argument`. A bounded C probe confirmed
that `getpagesize()` returned `-1` inside the profile versus `16384` outside,
with identical stack limits. Apple's numeric `CTL_HW/HW_PAGESIZE` API resolves
to **`hw.pagesize_compat`**, not the separately named `hw.pagesize` already in
the allowlist. The profile now permits that exact read; it does not grant global
sysctl access. References:

- [Apple XNU 10063.121.3 page-size sysctl definitions](https://github.com/apple-oss-distributions/xnu/blob/xnu-10063.121.3/bsd/kern/kern_mib.c)
- [Apple libc numeric page-size query](https://github.com/apple-oss-distributions/Libc/blob/Libc-1592.100.35/gen/FreeBSD/getpagesize.c)
- [Rust 1.88 guard alignment and mapping](https://github.com/rust-lang/rust/blob/1.88.0/library/std/src/sys/pal/unix/stack_overflow.rs)

CI runs the Rust helper under the real profile and separately verifies that
`kern.procargs2` cannot read the trusted parent's environment. Startup probes
must execute representative runtimes, not only `/bin/sh`.
