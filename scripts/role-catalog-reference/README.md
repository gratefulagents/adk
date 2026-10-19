# Pinned Go role/catalog codec oracle

Run `python3 scripts/role-catalog-reference/run.py` to regenerate
`fixtures/role-catalog/observations.json`; add `--check` to reproduce and compare
it without rewriting. Run `cargo test -p adk-codec --test roles` for the Rust
comparator. The Go runner archives immutable SDK commit
`1dc92b73900fac74dc357a938e4b5eee6392b418` into a temporary directory and copies
only the oracle test into that checkout. It never modifies `repos/sdk`.

The oracle invokes actual `encoding/json.Unmarshal` and `json.Marshal` on
`agentsdk.RoleSpec` and `agentsdk.RoleCatalog`. Provenance records the archive,
source, and harness SHA-256 hashes, Go version, OS, and architecture. The
fixture also records reflected field names, types, JSON tags, and catalog kind.
Each case contains input hex, decode status/error, decoded fields, marshal
status/error, and exact output hex. Successful decoded fields are compared
without nil/empty, access-level, name, or fallback normalization. Rejected cases
compare rejection only, not Go's partially mutated state or error wording.

Coverage includes root null and wrong types, scalar null no-ops, missing/nil/
empty fallback slices and catalogs, null elements, repeated field backing-slot
reuse and reset, ASCII and Unicode key folding, unknown fields, escaped and
invalid UTF-8 strings, lone/pair surrogates, HTML and separator escaping,
malformed JSON, and ignored values at the shared Go nesting boundary. Role
cases also run as catalog elements, exercising the additional nesting level.

The Rust targets are compatibility-only `adk_codec::dto::{RoleSpec, RoleCatalog}`
through `from_go_json` and `to_go_json_bytes`. Ordinary Serde does not promise
Go byte decoding (notably invalid UTF-8 and lone surrogates). `None` retains nil
slices and `Some(vec![])` retains allocated empty slices. There is no native
specialist conversion, Agent codec, runtime/tool construction, or runnable
wrapper parity claim.
