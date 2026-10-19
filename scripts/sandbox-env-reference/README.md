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

The POSIX base-environment projection additionally compares106 hermetic cases.
Only the disposable Go test subprocess changes its own environment; snapshots
contain fixture-owned dummy values, never live host values. Cases cover inherited
identity/locale variables, secret exclusion, fallback/configured/inherited PATH,
prepend/append precedence, lexical cleanup/deduplication, GOROOT, sorted extra
variable expansion, original configuration-test inputs, all25 Go whitespace
scalars and non-whitespace/invalid UTF-8 controls. Native execution policy and
request-override ordering are not changed or claimed by this projection.

ConfigFromEnv is checked separately with363 explicit snapshots, comparing every
constructed field and the derived environment. Both original config regressions
are included.30 flag spellings preserve ParseBool-before-yes/y/on behavior (mixed
TrUe is false), plus raw strings, cleaned path lists, newline/comma selection,
trimmed duplicate keys and literal extra values. These are data-only projections,
not ambient acquisition or native permission grants.
