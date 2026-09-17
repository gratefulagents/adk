# Licensing and provenance

This foundation is unpublished (`publish = false`). Reusable workspace crates use
GPL-3.0-only, preserving the upstream SDK's license for source-derived contracts.
The platform adapter and platform-linked binaries use AGPL-3.0-only, preserving
the platform baseline's license. These choices do not relicense either upstream.
See `LICENSE`, `crates/adk-platform/LICENSE`, and `fixtures/NOTICE.md` for license
texts, original provenance and notices. The offline harness links the platform
codec and is therefore explicitly on the platform side of the boundary.

Execution-policy and secret/shell regression provenance is recorded in
[`crates/adk-security/NOTICE.md`](crates/adk-security/NOTICE.md). The optional
sandbox invokes host-installed Bubblewrap or Seatbelt; it does not redistribute
those OS/backend binaries. Research comparisons do not relicense or copy the
external agent implementations cited in the design document.

The `adk-tools` manifest, signal/plan/search/filesystem behavior and fixtures derive from Grateful
Agents SDK v0.0.115 (`1dc92b73900fac74dc357a938e4b5eee6392b418`), under
GPL-3.0-only. Source types and acceptance IDs are retained in the manifest;
[`docs/tools.md`](docs/tools.md) records the implementation/verification boundary.

Third-party dependencies retain their respective licenses; `deny.toml` checks the
resolved all-feature dependency graph against an explicit SPDX allowlist. This
is an engineering gate, not legal advice or permission to ignore license duties.
