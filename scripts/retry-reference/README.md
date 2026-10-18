# Pinned retry policy reference

`python3 scripts/retry-reference/run.py --check` executes a disposable archive of
SDK1dc92b73900fac74dc357a938e4b5eee6392b418 (GPL-3.0-only). It verifies all three
original offline errors_retries examples and records1890 independent backoff
observations plus explicit default settings. Source/harness hashes and Go version
are retained. Cases cover zero/negative/large delays, zero/negative/large attempts,
partial settings, fractional/default/constant multipliers, NaN/infinities, caps,
and signed64-bit duration overflow on the pinned Linux amd64 target.

The native compatibility helper returns raw signed nanoseconds, not a sleep
Duration or automatic execution permission. It does not change native runner
defaults, install custom multipliers into the runner, or claim Go JSON wire parity.
