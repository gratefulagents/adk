# Metadata compaction reference

Derived by executing the GPL-3.0-only Grateful Agents SDK at
[`1dc92b73900fac74dc357a938e4b5eee6392b418`](https://github.com/gratefulagents/sdk/tree/1dc92b73900fac74dc357a938e4b5eee6392b418).
Original copyright and licensing remain applicable; see the repository provenance notices.

`python3 scripts/metadata-compaction-reference/run.py --check` executes the pinned
source in a disposable archive, using loopback HTTP and fixture credentials only.
It checks full-name precedence, first-slash aliases, case folding (including Δ and
İ), misses, successful-fetch caching, failed-fetch cooldown and retry. The cooldown
oracle changes only the test resolver's timestamp; it does not patch SDK code.
Source and harness digests are embedded in the observations. No live-provider claim.

Native tests additionally check lazy construction, shared concurrent fetches,
cancellation before fetching, static fallback, redacted diagnostics, and builder
feature/host-resolver precedence. These checks do not close the entire provider or
builder ledger. Exhaustive Unicode-version compatibility is not established by
these two non-ASCII fixtures.
