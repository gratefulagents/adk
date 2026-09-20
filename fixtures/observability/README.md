# Native observation fixture

`native-v1.jsonl` is an original deterministic Rust observation-schema fixture,
not exported Go trace data. It pins field names, explicit null usage metadata,
floating-point costs, cumulative progress, sequence order and JSONL delimiters.
`crates/adk/tests/event_fixtures.rs` checks seven-byte fragmented decoding and an
exact byte round trip. It is GPL-3.0-only under the repository license.

The native schema is intentionally versioned separately from Go trace schema 2.
This fixture does not establish Go trace-store or span-schema compatibility;
those remain explicit issue #11 blockers.
