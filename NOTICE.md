# Licensing and provenance

This foundation is unpublished (`publish = false`). Reusable workspace crates use
GPL-3.0-only, preserving the upstream SDK's license for source-derived contracts.
The platform adapter and platform-linked binaries use AGPL-3.0-only, preserving
the platform baseline's license. These choices do not relicense either upstream.
See `LICENSE`, `crates/adk-platform/LICENSE`, and `fixtures/NOTICE.md` for license
texts, original provenance and notices. The offline harness links the platform
codec and is therefore explicitly on the platform side of the boundary.

Third-party dependencies retain their respective licenses; `deny.toml` checks the
resolved all-feature dependency graph against an explicit SPDX allowlist. This
is an engineering gate, not legal advice or permission to ignore license duties.
