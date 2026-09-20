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

The `adk-tools` manifest and tool behavior/fixtures derive from Grateful Agents SDK
v0.0.115 (`1dc92b73900fac74dc357a938e4b5eee6392b418`), under GPL-3.0-only. This
includes the built-in tool implementations and trusted host-adapter contracts;
source types and acceptance IDs are retained in the manifest. [`docs/tools.md`](docs/tools.md)
records the implementation and verification boundary. Its draft acceptance status does
not assert complete security review, native-browser availability, or live external-service validation.

Runtime facade composition, file-mode/role loading, event/trace adapters and their
regressions reference the same pinned Grateful Agents SDK v0.0.115 source
(`1dc92b73900fac74dc357a938e4b5eee6392b418`), especially `pkg/agentsdk/runtime`,
`host`, `events`, `tracestore`, `otel` and `examples/features`. They remain
GPL-3.0-only. The immutable source inventory and current acceptance overlay are
separate: API mapping does not imply behavioral parity. External Rust-framework
research records design inspiration, not copied implementation or relicensing.
The optional OpenTelemetry API/SDK dependencies retain their Apache-2.0 license.

## matchit 0.8.4

`adk-mcp` reaches `matchit` through `axum 0.8.9`. The package declares `MIT AND
BSD-3-Clause`; its bundled `LICENSE` and `LICENSE.httprouter` notices are retained
below for source and binary redistributions.

MIT License

Copyright (c) 2022 Ibraheem Ahmed

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

BSD 3-Clause License

Copyright (c) 2013, Julien Schmidt
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Third-party dependencies retain their respective licenses; `deny.toml` checks the
resolved all-feature dependency graph against an explicit SPDX allowlist. This
is an engineering gate, not legal advice or permission to ignore license duties.
