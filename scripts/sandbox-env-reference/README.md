# Explicit sandbox environment expansion

`run.py --check` executes `sandbox.ExpandSafe` from an immutable SDK archive at
1dc92b73900fac74dc357a938e4b5eee6392b418. The fixture binds source, Go stdlib and
harness hashes and covers 540 byte inputs: ASCII boundaries, special variables,
malformed braces, missing values, Unicode, invalid UTF-8, NUL, and nonrecursive substitutions.
No ambient environment values are included. This checks only the standalone
explicit-map projection, not SafeEnv inheritance, configuration or execution.

The oracle also records all nine public configuration-name constants through the
ordered SandboxConfigEnvNames result and separate SandboxModeEnv value. The list
intentionally excludes mode and GOROOT, and Rust returns fresh owned names.
