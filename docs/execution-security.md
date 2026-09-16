# Execution security (opt-in; no production rollout)

`adk`'s `execution` feature exposes `adk-security` and `adk-sandbox`. Default
builds remain contracts-only. This work does not register a shell tool, enable a
platform worker, or change any deployment configuration.

## Trust boundary

The embedding host is trusted; model arguments, commands, repository files and
subprocess output are not. Host-selected policy, workspace, environment grants,
backend configuration and approval identity must never be copied from arbitrary
model JSON. OS filesystem enforcement and command authorization are independent:
approval cannot turn a denied operation into an allowed one, and an OS write
boundary cannot enforce remote Git policy.

The low-level sandbox executor is a **trusted-host API**, not a model-facing
tool. Applications must compose platform, parent and tool policy before invoking
it. Do not expose a generic tool that accepts an `AccessMode` or environment map
from the model. A child must receive the intersection of its parent's authority
and its requested authority, never a replacement policy.

## Supported targets and verification

| Target | Restricted execution | Local execution |
|---|---|---|
| Linux | Bubblewrap and functional namespaces required; no permissive fallback | Explicit full access only |
| macOS | Seatbelt (`/usr/bin/sandbox-exec`) required; deprecated compatibility backend | Explicit full access only |
| Other targets (including Windows) | Unsupported, error before spawn | Unsupported in this Unix implementation |

The dedicated [CI matrix](../.github/workflows/execution-security.yml) runs Linux
and macOS backend tests with `ADK_REQUIRE_SANDBOX=1`: unavailable enforcement is
a failure, not a skip. Windows checks compilation and unsupported-target
fail-closed behavior. The ordinary workspace tests can run on nested/container
hosts without user namespaces; those tests must not be interpreted as proof of
OS enforcement when the backend is unavailable.

The implementation environment for this change has Bubblewrap installed but
its mount view lacks `/proc/sys/kernel/overflowuid`; the Bubblewrap probe fails
before any workload starts. Accordingly, local fail-closed and process tests are
separate evidence from the required CI backend tests. Seatbelt cannot be tested
on a Linux host.

## Limits

This is not a VM or an arbitrary hostile native-code isolation guarantee. OS
kernel compromise, resource exhaustion, side channels and arbitrary host-file
confidentiality require additional host controls. Network grants must be made
by the trusted host and do not constitute a destination-aware egress policy.
A process-group cleanup boundary is not equivalent to cgroup ownership: a
permissive local process is already trusted with full host authority. Local and
Seatbelt execution cannot guarantee cleanup of malicious `setsid`/double-fork
daemons; do not use those backends where adversarial descendant lifetime
containment is required. Linux uses PID namespace teardown for that boundary.

Linux workspace-write requires host-precreated protected metadata mountpoints
(listed in the sandbox README), including `.git/config`, `.git/hooks` and agent
configuration paths. Ordinary top-level creation and Git index/object writes
remain possible. Workspaces require exclusive trusted ownership from validation
through cleanup; concurrent hostile host-side filesystem races, hardlinked
files, linked Git worktrees and privileged workspace sockets are unsupported.
These stricter prerequisites are intentional compatibility limits, not silent
fallbacks to weaker enforcement.

See [Rust design/dependency research](execution-security-research.md) for source
links, license/version evidence, process ownership and asynchronous shutdown
tradeoffs.

## Authorization API and composition

1. Construct `SecurityPolicy` from trusted host configuration. `compose` takes the
   intersection of grants, union of denials, minimum access/timeout, stricter
   approval, and AND of network/remote-write grants. `for_child` additionally
   removes mutating exceptions; arbitrary `agent_` tool names do not grant access.
2. Parse model input with `CommandRequest::from_call`. Its schema accepts only a
   nonempty `command` string, not access/network/environment/approval fields.
3. Call `SecurityPolicy::authorize` with the host's registered `ToolDefinition`
   and `Host`. Denials and command/input guardrails precede approval. A hanging
   approval is raced against cancellation and deadline. Deferred approval returns
   an `ApprovalRequest`, not an executable capability; reauthorize on resume.
4. Pass the non-cloneable `AuthorizedCommand` to `adk::execution::run_authorized`.
   It checks the run ID, fixes `/bin/sh -c`, uses the executor's workspace root,
   applies the authorized access/network/timeout, and consumes the capability.
   The executor must be the trusted instance for the workspace shown at approval.
5. The bridge checks stdout/stderr for secret signatures **before** returning
   output to the tool. Detected payloads are blocked wholesale, without echoing
   the secret. Hosts using the low-level executor directly must apply equivalent
   checks before logging, tracing, publishing or persistence.

The literal-shell classifier is deliberately conservative. It recognizes quoted
and escaped literal words, separators and bounded redirections, then checks
programs and options. Dynamic expansion, substitutions, heredocs, wrappers,
unknown programs and indirect execution are rejected, not guessed safe. This is
not full Bash support. Even OS-enforced execution does not bypass remote-write
and destructive-command authorization. Git `diff`, `show` and `log` require
`--no-ext-diff --no-textconv`; fixed protected push targets include `main`,
`master` and `HEAD`. Hosts with additional protected branches must apply their
own stricter gate or leave remote writes disabled.

Secret signatures and shell-obfuscation fixtures are ported from the pinned SDK,
with exact provenance in [adk-security/NOTICE.md](../crates/adk-security/NOTICE.md).
Pattern detection handles the tested ANSI/Unicode obfuscations, not arbitrary
encoding or deliberate data exfiltration; it is not a comprehensive DLP system.
See the [security crate](../crates/adk-security/README.md) and
[sandbox crate](../crates/adk-sandbox/README.md) for API-specific restrictions.
