# MCP prompt provenance

`observations.json` comes from independent execution of SDK revision
`1dc92b73900fac74dc357a938e4b5eee6392b418` in a disposable source archive:

- https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/mcp_prompt.go
- https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/runner.go

Reproduce with `python3 scripts/mcp-prompt-reference/run.py --check`. Source and
harness SHA256s are retained in the fixture. Fifty-six offline normal/streamed
requests cover ordered/duplicate/blank names, malicious control text, Unicode,
64-rune boundaries and ordering after additional/structured-output sections.
The scalar digest length-frames the exact sanitizer output for `a<scalar>b`, for
every valid Unicode scalar in ascending order, using little-endian u32 UTF-8
byte lengths followed by those bytes. It verifies Unicode 15.0 classification
without storing a million synthetic strings. Names have no execution authority.
No external providers or tool invocations are used.

SDK-derived text and the ported formatter retain upstream ownership and
**GPL-3.0-only** licensing; the original license is retained in
[`../licenses/SDK-GPL-3.0.txt`](../licenses/SDK-GPL-3.0.txt).
The Unicode classifier is separately provided by Apache-2.0-licensed
`unicode-general-category` 0.6.0; see the pinned dependency source and
https://docs.rs/unicode-general-category/0.6.0/src/unicode_general_category/lib.rs.html.
No whole-MCP or general prompt-injection security equivalence is asserted.
