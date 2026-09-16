# Security implementation and regression provenance

Upstream: https://github.com/gratefulagents/sdk

Pinned revision: `1dc92b73900fac74dc357a938e4b5eee6392b418`.
The SDK is GPL-3.0; this crate preserves GPL-3.0-only workspace licensing and
includes its GPLv3 license text in `LICENSE`. Upstream authors retain their
copyrights. This is an outcome-oriented Rust implementation, not a relicensing.

## Copied artifacts

- `tests/fixtures/cmd_obfuscation.txt` copied unchanged from SDK
  `eval/audit-fixtures/cmd_obfuscation.txt` (34 cases).
  SHA-256: `e65e516178bb842d3a4ce06c38118bbf1a741ae04d53e2fc033c4976142c4b0d`.
- `tests/fixtures/secret_obfuscation.txt` copied unchanged from SDK
  `eval/audit-fixtures/secret_obfuscation.txt` (synthetic regression material).
  SHA-256: `8f2fdda01c389c4492dae8f6f66839185c3a9b75876665d918dd34bba655c84e`.
- `src/signatures.rs`: 14 signature names/regexes extracted from
  `pkg/agentsdk/guardrails/builtin.go`, adapted to Rust's non-backtracking regex
  engine. Do not print fixture values into test failure messages.

## Reviewed behavior and adapted regression coverage

- Core native contracts: `crates/adk-core/src/{policy,contracts,types,error}.rs`.
- SDK `internal/agent/policy/{permission_mode,runtime_policy,tool_policy,
  git_remote_writes}.go`, `pkg/agentsdk/policy/mapping.go`.
- SDK `pkg/agentsdk/tools/shell/classifier.go`, `findings_test.go`, shell test
  inventory including command-substitution and safe-device cases.
- SDK `pkg/agentsdk/guardrails/{builtin,builtin_test,hardening_test}.go`.
- SDK `docs/security.md` and `eval/audit-fixtures/README.md`.

Tests adapt quote/escape/root-removal regressions, quoted-text negative cases,
Git refspec/remote-write restrictions and secret corpus behavior. Additional
Rust-native tests cover exact immutable authorization, policy meets, child
exception removal, unknown syntax/program denial, timeout/cancellation racing,
zero-width/ANSI/fullwidth secret normalization and safe diagnostics.

Intentional divergences: unknown/empty permission input fails read-only; no
unknown executable or dynamic grammar is accepted in full access; wrappers and
build tools fail closed rather than being recursively approximated; only
`/dev/null` is a safe write-redirect exception; complete secret-bearing outputs
are blocked instead of redacted. Upstream's allowed `go test` case is denied
because project code could perform forbidden remote writes. Classifier outcomes
are not a sandbox and cannot constrain malicious binaries or Git configuration.

## Parser/dependency decision

No third-party shell parser crate was added. A complete Bash parser would still
need a restrictive AST policy and cannot statically resolve shell expansion,
program effects or arbitrary code. The implementation recognizes a small,
bounded literal subset and rejects unsupported grammar rather than silently
approximating it. This avoids adding an unreviewed parser/license dependency.
The existing workspace regex dependency family (MIT OR Apache-2.0) is used only
for credential signatures; regex matching is not used to parse shell syntax.
Tokio is used solely to race approval against cancellation and deadlines.
