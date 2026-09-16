# Provider client research (issue #5, 2026-09-16)

This is a bounded dependency decision, not a completed provider parity assessment.

## Version/license/maintenance evidence

| Candidate | Verified version/license | Linked evidence and maintenance signal |
|---|---|---|
| async-openai | 0.42.0, MIT, declared Rust 1.75 | [Exact registry metadata](https://crates.io/api/v1/crates/async-openai/0.42.0), published 2026-09-09, unyanked; [repository](https://github.com/64bit/async-openai) |
| reqwest | 0.12.28, MIT OR Apache-2.0, declared Rust 1.64 | [Exact registry metadata](https://crates.io/api/v1/crates/reqwest/0.12.28), published 2025-12-22, unyanked; [repository](https://github.com/seanmonstar/reqwest). Registry resolution reports newer 0.13.5; selected version is not claimed to be latest. |
| Rig | Previously inspected rig-core 0.42.0, MIT | [Version/source/license evidence](rust-research.md); published 2026-08-17 |
| genai | Previously inspected 0.6.5, MIT OR Apache-2.0 | [Version/source/license evidence](rust-research.md); stable version was inspected rather than newer prereleases |
| ADK-Rust | Previously inspected adk-core 2.2.0, Apache-2.0 | [Version/source/license evidence](rust-research.md); declared Rust 1.95 exceeds repository Rust 1.88 |

Publication is evidence of activity, not an SLA. Previous framework research was
reused, not represented as a fresh compile/maintenance audit. No whole framework
was added.

## Wire-capability probe

Inspected async-openai's version-specific
[`CreateResponse`](https://docs.rs/async-openai/0.42.0/async_openai/types/responses/struct.CreateResponse.html)
and registry feature metadata. It exposes explicit `prompt_cache_key`,
`prompt_cache_retention`, `reasoning`, `include`, `store`, `stream`, metadata and
schema-related request fields. The crate exposes a `byot` feature, so it would be
incorrect to claim it cannot send a custom wire body. Its transport dependency is
reqwest 0.13, while this spike directly locks 0.12.28. This inspection did **not**
compile an async-openai capability probe or prove its compaction/SSE/OAuth parity.

The missing compatibility obligations are not merely JSON field names: Codex
request rewriting, host-owned account-scoped material/rotation, Copilot endpoint
selection and Anthropic-specific subscription conventions remain project-owned.
A provider-client wrapper does not discharge those obligations. Core history has been extended with lossless reasoning and compaction items.
Choosing a client alone would not have repaired that contract.

The explicit adapter's executable probes currently cover captured Chat request
JSON, real chunked SSE, redirect refusal, Responses cache-write extraction,
reasoning/tool deltas, native compaction and opaque continuation replay. Those are bounded probes,
not proof of missing capabilities in competing clients.

## Decisions

- **Adopt reqwest 0.12.28 for the experimental transport**, pinned exactly. It
  supports owned response bodies, request cancellation by dropping futures,
  sensitive header values, and disabling redirects/proxy discovery. Use rustls
  rather than a platform OpenSSL installation. No implicit HTTP retry policy is
  installed in the adapter. Its complete transitive dependency/license review is
  a separate build gate; the direct crate's license alone is not sufficient.
- **Adapt Rig/genai separation of model request, transport and application loop.**
  Keep the existing `adk_core::Model`/`StreamingModel` and pull-based `ModelStream`
  interfaces instead of introducing a second executor/agent framework.
- **Reject whole ADK-Rust adoption** for this lane: compiler mismatch and foreign
  runner/event semantics. This does not imply the framework is unmaintained.
- **Defer async-openai adoption**, rather than claiming its wire capabilities are
  inadequate. Its typed API and BYOT escape hatch merit an executable comparison
  before finalizing the complete provider crate. The current explicit adapter is
  experimental and should not be promoted on the strength of this source-only
  inspection.
- **Reject silently lossy continuation conversion.** Unknown opaque reasoning or
  compaction crossing incompatible protocols yields Unsupported. Native history
  now retains provider continuation; codec and replay fixtures cover it.

## Transitive TLS license/duplicate review

The pinned cargo-deny 0.20.2 audit identified five licenses outside the existing
allow-list. Their locally downloaded LICENSE texts and Cargo metadata were read:
`ring 0.17.14` (Apache-2.0 AND ISC), `rustls-webpki 0.103.15` and
`untrusted 0.9.0` (ISC), `subtle 2.6.1` (BSD-3-Clause), and `webpki-roots 1.0.9`
(CDLA-Permissive-2.0). Exact-version exceptions, not a global permissive switch,
are recorded in deny.toml. ISC/BSD notices must accompany redistribution; the
root-certificate data license requires sharing its agreement text with the data.
Upstream license files remain authoritative, including ring's per-file notices.
This is an engineering dependency review, not legal advice or a release attestation.

`ring` still depends on getrandom 0.2.17/windows-sys 0.52.0 while the runtime uses
newer APIs. Exact-version duplicate exceptions document that unavoidable API split;
all other duplicate bans remain enabled. No registry/git source policy was relaxed.

The final client selection/research acceptance in #5 is consequently still open:
version/license links and source inspection are present, but the requested full
missing-capability experiment across client candidates has not been completed.

## Material codec dependencies

- **Adopt base64 0.22.1** (MIT OR Apache-2.0), already in the transport closure,
  for the baseline raw-standard OpenRouter envelope and URL-safe JWT payloads.
  [Exact registry metadata](https://crates.io/api/v1/crates/base64/0.22.1).
- **Adopt time 0.3.47** (MIT OR Apache-2.0, Rust 1.88), for checked RFC3339
  credential timestamps rather than a second ad-hoc parser.
  [Exact registry metadata](https://crates.io/api/v1/crates/time/0.3.47): unyanked,
  published 2026-02-05. This compatible release fixes
  [RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009), which the
  dependency audit caught in the initially probed 0.3.44. No advisory exception
  was added. This pinned release is not claimed to be latest.

JWT payload decoding only extracts untrusted expiry/account hints from host-supplied
material; it is not signature validation or an authorization decision. Explicit
account scope is still validated by the session.
