# Independent pinned Go fileconfig oracle

`fixtures/fileconfig/sdk-fileconfig.json` is produced by executing the exported
`github.com/gratefulagents/sdk/pkg/agentsdk/host/fileconfig` API, not by Rust or a
reimplementation. The SDK checkout must be clean at:

```
1dc92b73900fac74dc357a938e4b5eee6392b418
```

This module starts with `scripts/host-reference`'s module/dependency setup,
renames the module, and adds the SDK fileconfig dependency `gopkg.in/yaml.v3
v3.0.1`. From this directory, the local replacement is **`../../repos/sdk`**.
No SDK, Rust, source lock, or other reference program is changed. This is bounded
source evidence, not a claim of complete fileconfig or Rust parity.

## Run and verify

With Go 1.26.2 or newer configured in the caller's environment:

```sh
(cd scripts/fileconfig-reference && "${GO:-go}" run -mod=readonly .) \
  > fixtures/fileconfig/sdk-fileconfig.json
python3 scripts/fileconfig-reference/check.py
(cd scripts/fileconfig-reference && "${GO:-go}" test -mod=readonly -count=1 ./...)
(cd scripts/fileconfig-reference && "${GO:-go}" vet -mod=readonly ./...)
(cd scripts/fileconfig-reference && "${GO:-go}" build -mod=readonly -o /dev/null .)
```

Use regeneration only intentionally; `check.py` itself never rewrites anything.
It checks the clean SDK pin, resolves the actual Go module replacement, checks
fixture metadata, regenerates twice in memory, compares exact bytes each time,
and rechecks SDK cleanliness. It inherits all caller Go environment settings
(`GOROOT`, `GOTOOLCHAIN`, `GOPATH`, `GOMODCACHE`, `GOCACHE`, etc.) and uses `GO`
(default `go`) as the executable. No worker paths are embedded in the checker.
Initial verification used Go 1.26.8 on Linux/amd64. Error text includes native OS
messages, so byte reproducibility is verified on that platform, not promised
across operating systems. A trimmed Go installation may require the caller to
set `GOROOT` explicitly.

Files:

- `main.go`: fixture schema, fresh filesystem setup, exported SDK dispatch,
  direct SDK JSON serialization and temporary-root substitution.
- `scenarios.go`: executable inputs; no handwritten expected SDK outputs.
- `main_test.go`: semantic assertions for constraints, lookup isolation,
  precedence, cancellation, routing, aliases, source-only markers and repeatability.
- `check.py`: independent pin/replacement/fixture verification.
- `../../fixtures/fileconfig/sdk-fileconfig.json`: generated schema v1 fixture.

## Counts

51 cases, 190 queries, including 9 source-only cases:

| Operation | Queries |
| --- | ---: |
| `ListModes` | 15 |
| `GetMode` | 57 |
| `RoleCatalog` | 25 |
| `PermissionMode` | 25 |
| `ModeSnapshot` | 19 |
| `ModeDirective` | 25 |
| `dirs` | 8 |
| `BuildModeDirective` | 10 |
| `BuiltinModes` | 2 |
| `GuardrailRules` | 2 |
| `HandoffHistory` | 2 |

## Schema and Rust comparator contract

The top-level object is `{schema_version: 1, sdk_revision, cases}`. Each case is:

```
{
  name,
  source_only: [policy_reason, ...],
  input: {
    files: {relative_path: exact_text, ...},
    active_mode,
    root_style,
    home_unset,
    queries: [{operation, lookup, cancelled, template}, ...]
  },
  output: [{result, error}, ...]
}
```

`output[i]` corresponds exactly to `input.queries[i]`. Every query field is
present: unused lookup is `""`, cancelled is `false`, and template is `null`.
All Source queries use one Source for the case; there is no implicit call to
ListModes, RoleCatalog, or any validator in the harness. Each case gets a fresh
temporary root. Files are written verbatim under it; only parent directories
needed by the file map are created. A file named `modes` or `agents` deliberately
makes that directory path a regular file. The default empty file map therefore
means missing mode and agent directories, not empty precreated directories.

The Source's workDir input is the fixed string `" \\twork-dir\\n"` (actual tab and
newline). It has no public accessor or effect on these operations at this pin.
The active mode string goes directly into `WithActiveMode`; any trimming is SDK
behavior, not comparator preparation. `cancelled` supplies an already-cancelled
context. `BuildModeDirective` directly receives the provided SDK TemplateSpec
pointer, including null and an all-zero object; it does not parse YAML or use
the Source. `dirs` returns actual `RootDir`, `ModeDir`, `AgentDir`, and
`DefaultRootDir` values.

HOME is set to `<root>/home` per case, or unset when `home_unset` is true, and
restored afterward. `root_style` constructs the Source root argument as follows:

| Style | Argument before SDK handling |
| --- | --- |
| `literal` | `<root>` |
| `padded` | space + tab + `<root>` + space + newline |
| `default` | space + tab |
| `tilde` | `~` |
| `tilde-child` | `~/config` |

`result` is JSON-marshaled directly from the returned SDK value, including the
value returned alongside an error. It is not a Rust-shaped projection. SDK
structs have capitalized exported field names: TemplateSpec contains `Name`,
`Version`, `DisplayName`, `Description`, `Category`, `Autonomous`, `ToolAccess`,
`Instructions`, `ModelRouting`, and `Constraints`. Routing includes default and
role model/fallback/reasoning/verbosity fields. Constraints retain **all five**:
`MaxTurns`, `SubAgentMaxTurns`, `MaxConcurrentSubAgents`, `MaxRetries`, and
`MaxRuntimeMinutes`. Both plain and CRD cases supply nonzero distinct values for
all five. Roles retain `Name`, `Description`, `Instructions`, `ToolAccess`,
`ModelOverride`, and `FallbackModels`, including null fallback slices.

Do not collapse null/empty arrays or objects, omit zero fields, sort results,
deduplicate fallbacks, trim strings, change case, normalize access aliases, or
rewrite directive text in a comparator. Any such behavior in the fixture is
what the SDK itself returned. Query result shapes vary by operation: modes are
arrays or objects, roles arrays, permission/directive/formatter strings, and
dirs objects. Failed mode retrieval returns null, failed permission/directive
returns an empty string, and the error is recorded independently.

`error` is null or `{message, category}`. Message is the actual `error.Error()`.
Category is deliberately coarse and harness-defined, not an SDK error enum:
`cancelled` only when `errors.Is(err, context.Canceled)`; `fileconfig` otherwise.
A Rust comparator should compare its own error classes separately rather than
pretending Go YAML/OS diagnostic wording is a cross-language API contract.
Only generated temporary-root occurrences are replaced by `<root>` in results
and messages. No semantic normalization, regex scrubbing, timestamp rewriting,
or SDK object projection occurs. JSON's standard HTML escapes may represent
`<root>` as `\u003croot\u003e` in the file; decoded text is `<root>`.

## Coverage and deliberate policy boundaries

Coverage includes missing directories, builtins and declared-name overrides;
plain and CRD fields, metadata/filename/version fallback and zero-spec fallback;
filename lookup versus declared-name and display-name lookup, case-insensitive
fallback, YAML-before-YML, malformed YAML precedence; direct lookup isolation
from an unrelated malformed mode; malformed roles isolated from mode lookups
and malformed modes isolated from role catalogs; extension case, ignored files
and subdirectories; blank/traversal/missing names, cancellation validation order
and inactive-mode cancellation bypass; role frontmatter alias precedence,
whitespace, CRLF, unclosed frontmatter and empty instructions; access aliases
and inheritance; exact routing keys, trimmed/blank-filtered but duplicate-retained
fallback lists; and direct formatter null/zero/fallback-label/read-only aliases.

`source_only` is a case-level exclusion marker for a parity comparator, **not a
Rust expected result**. The 9 marked cases must be reported separately:

- `duplicate-name-policy`: Go lists both duplicate declared mode names and uses
  the first match for fallback lookup; Go keeps the first duplicate role name.
  These are not duplicate YAML mapping keys: the separate duplicate-key case
  captures yaml.v3's rejection.
- `unknown-field-policy`: Go ignores unknown mode, constraint, and role fields.
- `unknown-access-policy`: Go preserves unknown access text, and PermissionMode
  still yields workspace-write; this is not a safe Rust policy to import.
- `zero-negative-limit-policy`: Go serializes zero/negative constraint values
  without strict validation. Empty constraint objects are compared normally.
- `unterminated-frontmatter-policy`: Go treats unclosed role frontmatter as
  plain body text; Rust rejects the malformed document.
- `home-policy`: Go's fallback with HOME unset is recorded but not normative;
  Rust requires an absolute HOME. Normal absolute-HOME and tilde cases are compared.

Rust keeps its strict duplicate, unknown-field, access, zero-limit, frontmatter
and HOME policies. This oracle does not fabricate their expected errors or
assert parity for marked cases. `crates/adk/tests/fileconfig_oracle.rs` compares
all 171 eligible queries across 42 cases, including complete mode/role DTOs,
errors, independent loading, root styles and isolated-process HOME behavior.
The nine source-only cases contain 19 queries, explicitly excluded rather than
counted as passes. Run `cargo test -p adk --all-features --test fileconfig_oracle`
for the Rust comparison.

The standalone `BuiltinModes` and no-op `GuardrailRules`/`HandoffHistory`
queries run with malformed mode/role files, a nonexistent active mode, and both
active and cancelled contexts. The no-op observations are actual SDK null
slices, compared to asserted-empty native values (including empty history
sidecars), not fabricated populated results. Pure builtin results are compared
field-for-field independently of Source loading.

Not covered: filesystem permission denial, symlink security, races or concurrent
mutation, Windows path behavior, standalone LoadRoleCatalog calls (their behavior
is observed through Source), or SDK internals. These are not implied by the
counts above.
