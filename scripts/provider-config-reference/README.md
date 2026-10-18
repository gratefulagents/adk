# Provider configuration reference

Run `python3 scripts/provider-config-reference/run.py` to regenerate, or add
`--check` to reproduce and compare the checked-in observations byte-for-byte.
The harness uses `git archive` of `repos/sdk` at immutable commit
`1dc92b73900fac74dc357a938e4b5eee6392b418`, not the mutable checkout. It injects
`reference_test.go` only into a disposable extraction. No production Go source
is rewritten. Go dependencies must already be available for readonly module mode.

The output is `fixtures/provider-config/observations.json`, with archive, source,
harness and Go-toolchain provenance. Its cases observe source default selection,
key/URL/API/auth override precedence, whitespace and blank overrides, supported
canonical endpoints/protocols, Local's fallback seed, route promotion,
last-registration replacement and non-inheritance. The harness invokes the real
factory for every supported case. Resolution observations call the source's
private helpers; the small projection into native protocol/endpoint labels does
not substitute for request tests.

Three real loopback calls additionally record named Anthropic API-key vs explicit
bearer headers and OAuth beta, and OpenRouter Chat Completions path, opaque model
ID and returned text. No external provider calls or real credentials are used.
Missing/nested named provider and unknown provider rejection are also observed.
Ephemeral loopback ports are deliberately not part of the fixture.

Reviewed source anchors in `pkg/agentsdk/providers/factory.go`: 119–143,
228–269, 636–657, 691–810. Related source tests: `factory_test.go` at
49, 84, 90, 100, 593, 616, 694, 707; `audit_fixes_test.go` at 78 and 115.

This is a **bounded configuration slice**, not whole `ProviderSpec`,
`ProviderRoute`, or `NewProviderFromConfig` parity. In particular it excludes
OAuth file loading/reload, OpenAI OAuth/session material, Copilot, model fallback
arrays and eager source credential loading. Native unsupported-input and
scope-security tests are not Go parity claims. Ambiguous normalized map keys are
excluded from oracle inputs: Go map iteration does not define a deterministic
winner. Empty-provider promoted configurations with top-level values are also
excluded; Go fans those values out, whereas the native API rejects the ambiguity.
See `docs/provider-configuration.md` for the native API and full boundary.

Both this harness and the archived SDK are GPL-3.0-only.
