# History / provider compatibility contract

## Native interfaces

Existing `Message`, `RunItem::Message`, and URI-based `Content::Image` constructors remain unchanged.

- `RunItem::PhasedMessage { message: Message, phase: String }` carries a nonempty provider phase verbatim. `RunItem::Message` means phase absent. Never infer `final_answer` just because the role is assistant. The codec rejects an empty explicit phase; use ordinary `Message` instead.
- `Content::Attachment { media_type: String, data: String, detail: String }` carries the Go `ImageAttachment` payload verbatim: raw base64 data (not a URL or data URI), MIME type, and optional image detail represented by an empty string when absent. This baseline covers inline images and PDF documents, not generic audio/file expansion.

The existing wire DTO is named `MessageOutput` in Rust; its `phase` is the Go message phase. Both `MessageOutput.images` and `ToolOutputData.images` carry ordered attachment arrays. Despite the name `images`, the baseline accepts `application/pdf` here.

## Codec and local compaction

`adk_codec::approval::{encode_item, decode_item, encode_history, decode_history}` preserve explicit phase, attachment data/type/detail/order, agent provenance, approval boundaries, and provider compaction origin. Wire decode produces one leading `Content::Text` (possibly empty), followed by attachments. Encode accepts an optional leading text block followed by attachments. Empty or attachment-only native content therefore decodes to the canonical leading-empty-text form. Interleaved/multiple text blocks and URI-based media remain unsupported by this bridge rather than being reordered, fetched, or silently dropped.

Local compaction treats phased messages like ordinary messages for estimates, summaries, and history finalization. Retained phased messages keep the original phase. Image/PDF-bearing messages and tool results are protected from lossy text summarization, as are their tool-call pairs. Opaque encrypted compaction items retain their exact `created_by`, content, ID, and ciphertext.

## Provider and runner integration (owned by parent task)

- Providers must handle `PhasedMessage` anywhere they handle `Message`, forwarding explicit phase where supported and not manufacturing a phase for absent values.
- Providers must map `Attachment` with `media_type == "application/pdf"` to the baseline document block, and image attachments to image blocks, retaining `detail` where supported. Do not treat base64 `data` as a URI.
- Runner response validation, final-output extraction, and streaming message coalescing must recognize `PhasedMessage` alongside `Message`.
- `Compaction.created_by` is the issuing provider, not agent provenance. The bridge never rewrites it. Provider adapters may replay encrypted compaction only for matching origin or legacy empty origin. For a foreign origin, retain available plaintext as a `[CONTEXT SUMMARY ...]` assistant message and do not forward foreign ciphertext; omit a foreign opaque-only item. This behavior belongs in adapters, not the codec.

Baseline references: SDK `pkg/agentsdk/providers/openai/end_turn_test.go`, `tool_images_test.go`, `pkg/agentsdk/providers/anthropic/model_test.go::TestItemsToMessagesRoutesPDFToDocumentBlock`, and both providers' `compaction_origin_test.go`.
