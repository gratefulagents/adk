# Run-instruction fixture provenance

`observations.json` records actual normal and streamed requests to an offline
recording model, plus `BuildRunConfig` composition, from SDK revision
`1dc92b73900fac74dc357a938e4b5eee6392b418`:

- https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/pkg/agentsdk/runtime/builder.go
- https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/internal/agent/runner.go

Reproduce with `python3 scripts/run-instructions-reference/run.py --check`.
The disposable archive leaves the SDK checkout untouched. Source/harness hashes
are recorded. Forty synthetic cases preserve exact text without normalization.
No external providers, tool execution, credentials or platform services are used.
No output-schema, MCP-context or deprecated-field fallback parity is asserted.

SDK-derived fixture text and ported composition retain upstream ownership and
**GPL-3.0-only** licensing. The original license is retained in
[`../licenses/SDK-GPL-3.0.txt`](../licenses/SDK-GPL-3.0.txt).
