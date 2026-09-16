# adk-security

Host-owned policy composition, conservative shell authorization and input/output
secret guardrails. This crate **does not execute commands or implement a sandbox**.
It uses native `adk-core` policy, tool, approval, error and context contracts.

## Public boundary

- `SecurityPolicy { tools: ToolPolicy, git_remote_writes, allow_network,
  approve_mutations }` is host-authored, not deserializable from model arguments.
  Defaults: read-only, no network, no Git remote writes. Use `compose` for the
  intersection of platform/run/tool authority; `for_child` additionally removes
  **all** read-only mutating exceptions, including child-requested exceptions.
  Exact-name grants intersect, denies union, shortest timeout wins, stricter
  approval wins. `clamp_access` never escalates. `normalize_access` maps unknown
  **and empty** input to read-only (stricter than the SDK's legacy empty default).
- `CommandRequest::from_call(ToolCall)` accepts exactly `{ "command": string }`.
  Permission fields, approval bits, non-string commands and extra fields fail
  closed. The command is derived from the immutable owned call, not separately
  supplied for classification and execution.
- `SecurityPolicy::authorize(&Context, &dyn Host, &ToolDefinition, CommandRequest)`
  is async and requires a Tokio runtime with time enabled. The definition must
  come from a trusted registry. Name/access denial, secret detection and command
  classification run **before** `Host::approve`. No approval can override a deny.
  Approval receives the exact native call. Pending approval is raced against
  cancellation/deadline; host errors propagate. Activity is rechecked afterward.
- Result: `Authorization::Approved(AuthorizedCommand)` or
  `Authorization::Deferred(ApprovalRequest)`. Resume requires fresh authorization;
  there is no public approval-bit or preapproved-token constructor.
- `AuthorizedCommand` has read-only `call()`, `command()`, `policy()`, `run_id()`.
  It has no public fields, `Clone`, deserializer or constructor. The execution
  adapter should consume it, verify the current run ID/activity, execute exactly
  its command using fixed `/bin/sh -c`, and enforce its access, network and timeout
  ceilings. The trusted host owns cwd/environment and must keep their security
  context stable across approval and execution. Never accept a second mutable
  model command, permission field or executable selector at execution time.
- `classify_command(command, access, git_remote_writes)` returns
  `CommandClass::{ReadOnly, Mutating}` or a categorized native error.
- `check_secrets(text)` blocks complete payloads with signature-only errors.
  Invoke on decoded input and **reassembled output before** publishing, logging or
  persisting it. Streaming chunks alone cannot detect split credentials.

## Accepted shell subset and intentional restrictions

The bounded lexer recognizes literal single/double quotes, concatenated quoted
words, shell backslash rules, pipelines, `;`, newline, `&&`, `||`, and basic
`<`, `>`, `>>` redirects (including numeric descriptors). It rejects malformed
quotes/operators, NUL/control characters, expansions, substitutions, ANSI-C
quotes, globs, comments, backgrounding, functions, heredocs and process
substitution. Maximum command size: 64 KiB; maximum token count: 2048.

Only enumerated programs and options are accepted: basic read commands (`ls`,
`cat`, `grep`, `wc`, `head`, `tail`, `echo`, `pwd`, `true`, `false`), constrained
Git operations, `rm` and `mkdir`. Unknown programs/options, arbitrary executable
paths, interpreters, wrappers (`env`, `sudo`, `eval`, shell `-c`), build/test tools
and indirect executors (`find -exec`, `sed e`, `rg --pre`) are denied. This applies
**even to FullAccess**, intentionally stricter than upstream's permissive mode.
It is not an attempt to emulate an entire shell or inspect arbitrary programs.
Quoted destructive text in a grep argument is data, not a destructive invocation.

Mutations fail in read-only mode even when a tool has an exact-name mutation
exception; exceptions never weaken filesystem confinement. Protected absolute
write/removal paths and parent-directory traversal fail closed. Only `/dev/null`
is exempted from write-redirect mutation classification. No filesystem or
symlink resolution is performed here: OS confinement remains mandatory.

Git aliases, `-c`, custom exec paths, config mutation, unknown subcommands and
implicit/dynamic push destinations are denied. Remote-write denial and protected
`main`/`master`/`HEAD` push denial are independent of sandbox availability. Pushes
require an explicit remote and one statically known non-deletion/non-force
refspec; tags, implicit destinations and broad push options are rejected.
`diff`, `show`, `log` require explicit `--no-ext-diff --no-textconv` before `--`.
Protected branch names are fixed to `main`/`master`/`HEAD`, not discovered from
remote defaults; hosts protecting other branches must disable Git remote writes
or impose an additional trusted gate.

**Trusted executor requirements:** fixed system PATH/binaries; no inherited
shell startup files, exported functions, Git overrides, credentials or loader
variables. Disable Git pager, hooks and `core.fsmonitor` with host-controlled
higher-priority configuration; repository config can otherwise run helpers even
for `git status`. Configured remotes/helpers must be host-trusted. An arbitrary
replacement binary, malicious repository configuration or script can implement
network side effects that an argv classifier cannot prove absent. Do not run
these approvals through a permissive executor against untrusted configuration.
The facade/sandbox adapter must enforce these requirements; this crate cannot.

## Secret handling

Fourteen high-confidence SDK credential signatures cover AWS, GitHub, OpenAI,
Anthropic, Slack, npm, JWT, private keys, bearer headers, GCP markers and literal
secret assignments. Scan both raw and normalized text; normalization removes
ANSI CSI/OSC sequences and common zero-width/bidi characters, and maps fullwidth
ASCII. Whole-payload blocking is intentionally stricter than upstream redaction:
AWS/GCP markers and PEM headers can accompany undetectable secret material.
This is not general DLP: arbitrary encodings, split payloads, novel formats and
semantic exfiltration need host-side controls. False positives are possible.

## Provenance and verification

See [NOTICE.md](NOTICE.md) and [LICENSE](LICENSE). Regression tests exercise both
unaltered upstream corpora, obfuscation variants, unknown grammar rejection,
policy no-escalation (2,304 combinations), exact-call approvals, host denial and
pause, forged fields, normalization and hanging approval interruption.

Run `cargo test -p adk-security --locked` and
`cargo clippy -p adk-security --all-targets --locked -- -D warnings`.
