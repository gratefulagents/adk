# Metadata compaction reference

Derived by executing the GPL-3.0-only Grateful Agents SDK at
[`1dc92b73900fac74dc357a938e4b5eee6392b418`](https://github.com/gratefulagents/sdk/tree/1dc92b73900fac74dc357a938e4b5eee6392b418).
Original copyright and licensing remain applicable; see the repository provenance notices.

`python3 scripts/metadata-compaction-reference/run.py --check` executes the pinned
source in a disposable archive, using loopback HTTP and fixture credentials only.
It checks full-name precedence, first-slash aliases, case folding (including Δ and
İ), misses, successful-fetch caching, failed-fetch cooldown and retry. The cooldown
oracle changes only the test resolver's timestamp; it does not patch SDK code.
A length-prefixed UTF-8 SHA-256 digest checks the first normalized lookup key for
all 1,112,064 Unicode scalar values using the pinned Go Unicode 15 tables.
Source and harness digests are embedded in the observations. No live-provider claim.

Native tests additionally check lazy construction, shared concurrent fetches,
cancellation before fetching, static fallback, redacted diagnostics, and builder
feature/host-resolver precedence. These checks do not close the entire provider or
builder ledger. The scalar digest establishes lookup-key normalization on the recorded Rust
1.88 target, not all metadata parsing or picker behavior.
