# Successful-child display cost reference

`sdk-result-cost.json` is produced by executing the immutable SDK
`1dc92b73900fac74dc357a938e4b5eee6392b418` implementation of
`internal/agent/cost.go::estimateRunResultCost` in a disposable archive. The
harness is inserted into the original package to call the private helper; it
does not copy or reimplement the calculation. The two original cost regressions
also execute. Source/harness hashes and the Go version are recorded.

```sh
python3 scripts/cost-reference/run.py --check
```

The 21 cases record inputs, returned amount/certainty, and every fallback usage
argument. They cover stored response sums, unknown certainty, empty responses,
known-zero/unknown/nonzero-unknown estimates, missing estimators, legacy
CalculateCost-only models, componentwise clamping, cache-only residuals,
requests-only residuals, and one combined fallback estimate rather than one per
compaction call. Counts are nonnegative native-runtime inputs; this corpus is not
proof of signed counter-overflow or malformed checkpoint behavior.

This is **display estimation**, not budget accounting. In the SDK the helper is
used after successful child runs; error paths use their tracker. Native cumulative
charges must not be replaced or incremented by these residual estimates. Runtime
integration tests separately verify actual charges, original-model routing,
checkpoint uncertainty, and invalid-price handling.
