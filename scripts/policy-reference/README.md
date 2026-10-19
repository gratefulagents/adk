# Pinned SDK policy observations

Run `python3 scripts/policy-reference/run.py --check`. The runner extracts an
immutable SDK archive at1dc92b73900fac74dc357a938e4b5eee6392b418, executes the actual
public policy aliases/mapping helpers and all six original internal/public tests,
and compares the saved observations and source/harness hashes.

496 label cases cover recognized/unknown/case/alias/control/Unicode whitespace
inputs and mutability/MCP/approval/access mappings.960 RuntimePolicy cases cross
six permission labels, five remote-write labels, all optional-feature flag states
and nil/empty/populated/duplicate lists. The Go whitespace set is enumerated over
all1,112,064 Unicode scalars and independently compared in Rust. These are pure
helper/data observations, not authorization, live provider or Go JSON-decoder
conformance claims. Fixture strings are valid UTF-8.
