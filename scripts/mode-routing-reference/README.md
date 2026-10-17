# Pinned Go mode-routing resolution oracle

Run `python3 scripts/mode-routing-reference/run.py` to regenerate
`fixtures/mode-routing/observations.json`, or add `--check` to reproduce without
rewriting. Run `cargo test -p adk-codec --test mode_routing` for the comparator.
The runner archives immutable SDK commit
`1dc92b73900fac74dc357a938e4b5eee6392b418` into a disposable directory and runs
actual `ResolveModeRouting` and `ResolveRoleModeRouting`. It never modifies
`repos/sdk`. Provenance binds the archive, implementation sources, harness,
Go version and target.

There are 294 input groups with one top-level and five role resolutions each:
1,764 independently observed results. Both unnormalized output and exact Go
marshal bytes are compared. Coverage includes missing/empty routing, provider
and model aliases/defaults, unknown providers, raw model names, whitespace,
Unicode simple lowercase, all reasoning tiers, valid/invalid verbosity,
exact case/whitespace-sensitive role lookup, nil/empty/all-blank/duplicate
fallback lists, inheritance and overrides, and explicit `none` clearing a
previous thinking budget. Reference and Rust check input immutability; a native
regression also checks independent output/source fallback ownership.

The Rust APIs are `adk_codec::mode::{resolve_mode_routing,
resolve_role_mode_routing}`. Their `ModeRoutingResolution` result supports the
sealed Go encoder but is not a Go decoder target. This is pure label/settings
resolution, not executable model construction, provider validation, request
clamping, native arbitrary settings merging or runtime policy enforcement.
