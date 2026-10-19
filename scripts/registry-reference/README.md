# Registry selection runtime oracle

Pinned SDK: `1dc92b73900fac74dc357a938e4b5eee6392b418` (`repos/sdk`).

```sh
python3 scripts/registry-reference/run.py
python3 scripts/registry-reference/run.py --check
cargo test -p adk-tools --test registry_reference
```

The generator uses a disposable **git archive of the pin**, installs the Go test,
then executes the real `NewRegistry`, `Names`, `Tools` and `Get`. It never derives
expected selection from the Rust manifest or a static Go inventory. Scratch files
and Go test temporary directories use `/workspace/scratch` (override with
`REGISTRY_REFERENCE_TMPDIR`). Go dependencies must already be cached; downloads
are disabled. Provenance hashes include the harness, tools/policy source and test
files, Go module files, and the Go tool interface. `--check` reruns the constructor
matrix and compares the complete observation/provenance bytes.

## Matrix and comparison

79 cases: the no-option Go constructor plus 13 profiles crossed with three
permission modes and both remote-write gates. Profiles cover baseline, signals,
Browser with/without private URLs, web disabled, private URLs without Browser,
async shell, think, terminal, all options, all options without web, and two
mutating-allowlist attempts (including unknown name and disabled families).

The fixture records exact sorted names, actual concrete Go constructor types,
full runtime definitions (deduplicated by content hash), sorted `Tools`, and
missing lookup. Rust compares:

- Exact selected names with no exceptions, including Browser selection.
- Actual stored option booleans, not only selection inferred from option inputs.
- Selected mode-specific source types against reflection of actual Go objects.
- Full selected descriptions, schemas, read-only and approval flags.
- Actual native Registry construction with real shell/LSP dependencies and
  built-in filesystem/search/web/signal implementations; no contract-shaped mocks.
- Constructed definitions and names/tools ordering, plus `get` returning `None`
  for a missing name even when that name was allowlisted.

**This is explicitly configured selection equivalence, not constructor parity.**
Go's default set is mapped independently to explicit Rust Strict features and the
Go permission/remote-write settings. `Signals.Finish` is not implicitly inserted:
Go `WithSignalTools` only supplies question and plan tools. Native `Strict(empty)`
remains empty and has its own assertion. No native safety default is changed.

## Boundaries

Browser is included in exact selection/type/definition comparisons, but excluded
from native resource-construction comparisons. Browser process/screenshot setup,
Vision allowed-image-directory integration, duplicate `Register`, execution of
selected tools, and optional Git/memory/project-state/LSP-config integrations are
not covered. Remote-write gating is exercised through InteractiveTerminal, not
through optional Git tool constructors. No claim of execution-authorization or
network-destination policy equivalence is made.

Current verification: fixture regeneration and `--check` passed; all nine Rust
registry-reference tests passed over all 79 cases. No native/source mismatch was
observed in this bounded slice. The explicit default-selection difference above
is intentional, not hidden by weakening expected-name comparisons.
