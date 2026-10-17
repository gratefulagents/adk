# Pinned Go mode codec oracle

Run `python3 scripts/mode-codec-reference/run.py` to regenerate
`fixtures/mode-codec/observations.json`, or add `--check` to reproduce without
rewriting. Run `cargo test -p adk-codec --test modes` for the Rust comparator.
The runner tests an immutable SDK archive at
`1dc92b73900fac74dc357a938e4b5eee6392b418`; it does not modify `repos/sdk`.

Actual `encoding/json.Unmarshal` and `json.Marshal` observations cover mode
`TemplateSpec`, `Constraints`, `ModelRouting`, `RoleModelRouting`, and the
runner's `ModeModelRouting` and `ModeRoleModelRouting` aliases, plus the open
`ModeReasoningLevel` and `ModeTextVerbosity` string types and ten constants. Provenance
records archive, source and harness hashes, Go version, OS and architecture.
The fixture records reflected field names/types/tags or scalar kinds and
1,061 cases: 622 accepted and 439 rejected (993 structured-data cases plus
68 named-string cases). Accepted cases compare unnormalized decoded state
and exact output bytes. Rejected cases compare rejection, not partial mutation
state or error wording. Integer expectations are the pinned 64-bit Go target.

Coverage includes root null/wrong types, scalar null no-ops, integer spelling
and bounds, repeated fields, nil/empty slices and maps, slice backing-slot
reuse, map merging with fresh entry values, pointer merging and reset,
backing slots across repeated pointer objects, invalid fields before pointer
reset, Unicode key folding, invalid UTF-8 and surrogates, escaping, malformed
JSON and ignored values at the Go depth boundary.

Rust compatibility-only targets are
`adk_codec::dto::{ModeTemplateSpec, ModeConstraints, ModeRouting, ModeRoleRouting}`
through sealed `from_go_json` and `to_go_json_bytes`. Ordinary Serde is not a
Go byte-decoding entrypoint. Named labels use open
`dto::{ModeReasoningLevel, ModeTextVerbosity}` wrappers with typed associated
constants; unknown strings remain valid and the empty default is not explicit
`NONE`. This does not establish native builder settings,
model binding, runtime constraint enforcement, or full mode-conversion parity.
