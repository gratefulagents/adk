# Pinned MCP display reference

`python3 scripts/mcp-display-reference/run.py --check` executes the pinned SDK
public sanitizers in a disposable archive.160 cases include all five original
sanitizer regressions, whitespace/control/format/private-use scalars, long server
tags and byte512/1024 cuts through multi-byte UTF-8, plus exact quoted-name framing. An independent SHA-256 stream
covers1,112,064 Unicode scalars. SDK, Go Unicode and harness hashes are retained;
source is GPL-3.0-only, Go Unicode data is BSD-3-Clause (retained Go LICENSE).

The native helper accepts UTF-8 strings and returns bytes to preserve partial
UTF-8 truncation. Only the body is capped in this legacy format. It is not an
identity/authorization check or a replacement for native bounded/quoted tool
framing. Printable Unicode data is shared with the existing logger, whose separate
oracle continues to verify identical logger output after relocating the table.
The generator also executes both original offline MCP examples.
