# Compatibility decisions and open questions

These are explicit gates for follow-up issues, not unspecified feature buckets. Source-exhaustive rows live in the SDK and platform ledgers; the IDs below address cross-cutting interpretation.

| ID | Decision / unresolved question | Owner role | Disposition / acceptance |
|---|---|---|---|
| D-001 | Behavioral parity, not Go object/package parity | ADK architecture (#2) | Decided: preserve observable API capabilities and contracts; Rust may use enums, traits, ownership and scoped cancellation. No Go FFI/sidecar as permanent implementation. |
| D-002 | Baseline identity | Migration maintainer (#3) | Decided: SDK checkout equals v0.0.115; platform checkout differs from epic pin only in Android CI. See source lock. |
| D-003 | Naming and stability of the Rust public API | ADK public facade (#11) | Open: ledger destinations are proposed; compile-time Rust ergonomics and semver need design. Every Go capability remains an acceptance obligation even when names differ. |
| D-004 | Multiple durable/session/workspace versions | Persistence owner (#9/#12) | Decided: version namespaces are independent; use the platform contract matrix and original JSON tags. Open: approved future-version rejection and migration policy where upstream lacks one; do not collapse formats. |
| D-005 | Upstream failures | Verification owner (each subsystem) | Decided: failed/skipped/unavailable Go checks remain evidence, not required bugs or passing parity. Follow baseline report for exact checks. Any intentional changed behavior needs a decision linked to its capability/test. |
| D-006 | Cryptography interoperability | Platform adapters (#12) | Open until bidirectional decrypt/restore tests cover the locked manifest/envelope/key derivation. Internal Rust library choice is not prescribed; wire bytes, integrity, path safety and tenant isolation are. |
| D-007 | Database concurrency | Persistence/platform adapters (#9/#12) | Open until actual Postgres tests validate transaction boundaries, competing claims, fencing, cancellation and crash recovery. Offline JSON fixtures cannot establish transactional parity. |
| D-008 | OS-specific sandbox guarantees | Sandbox owner (#6) | Open per supported backend: native OS behavior must be tested on that OS. Linux success must not be reported as Darwin/Windows security parity. Fail-closed behavior is part of the contract. |
| D-009 | Provider and MCP live behavior | Providers/MCP owners (#5/#8) | Open: offline fake-model coverage does not validate external auth, provider protocol changes or hosted transport interoperability. Pin provider SDK/protocol baselines separately when implementing. |
| D-010 | Replay normalization | Verification owner (#3 onward) | Decided: only declared nondeterministic fields may normalize; sequence, array order, null/absent distinctions, identifiers and their relationships are observable. Future adapters may not conceal extra events, reordered tools or altered errors. |
| D-011 | Platform ownership | Platform adapters/tools/executable (#12–#14) | Decided: reusable ADK has no platform dependency; controller/dashboard/API/database-service remain Go-owned. Shared wire/storage types are compatibility contracts, not a control-plane rewrite. Preserve run/slack/desktop-bridge. |
| D-012 | Code reuse licensing | Repository maintainers | Open: SDK GPL and platform AGPL notices are preserved, but this baseline grants no new licensing permission or blanket relicensing. Review before incorporating upstream implementation into runtime crates. |
| D-013 | Framework adoption | Subsystem owners | Open per subsystem after the research note: adapt verified patterns, do not sacrifice required capabilities to fit a framework. No framework selected as a mandatory runtime dependency here. |

## Completion vocabulary

- **Inventoried**: a pinned source row exists; not a claim of a tested behavior.
- **Go-verified**: a specific recorded command/assertion passed in the stated environment.
- **Fixture-verified**: offline fixture generation/comparison passes; applies only to the covered paths.
- **Rust-unimplemented / unverified**: no Rust behavior is delivered by this issue.
- **Blocked / unavailable**: a concrete dependency or environment prevents validation; never count as passing.

The fixture harness and inventories establish a starting gate, not a certification that all upstream behavior is correct. Later issues must attach executable acceptance results to their rows before claiming runtime parity.
