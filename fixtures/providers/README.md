# Provider fixture provenance

`responses-cache.json` is the sanitized synthetic response literal from
`repos/sdk/internal/openai/cache_usage_test.go::TestResponsesCacheWriteTokensNonStreaming`
at SDK `1dc92b73900fac74dc357a938e4b5eee6392b418` (GPL-3.0-only; see repository
LICENSE/NOTICE.md and migration source-lock). It contains no live credential or
customer data. The original response ID is synthetic and retained.

The provider tests also contain deliberately synthetic SSE and loopback HTTP
vectors, authored for the Rust implementation. They are **not** claimed to be
executed Go/Rust differential fixtures. In particular, rejecting an opaque
compaction item demonstrates fail-closed behavior, not compaction parity.
