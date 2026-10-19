# Pinned SDK LLM summary oracle

SPDX-License-Identifier: GPL-3.0-only

These observations and the accompanying harness are derived from execution of
[Grateful Agents SDK](https://github.com/gratefulagents/sdk) at commit
[`1dc92b73900fac74dc357a938e4b5eee6392b418`](https://github.com/gratefulagents/sdk/tree/1dc92b73900fac74dc357a938e4b5eee6392b418).
Original copyright and licensing remain applicable. The complete upstream license
is available at that commit's
[`LICENSE`](https://github.com/gratefulagents/sdk/blob/1dc92b73900fac74dc357a938e4b5eee6392b418/LICENSE)
and in `repos/sdk/LICENSE`; its SHA-256 is recorded in the fixture.
The fixture includes the upstream summary instructions verbatim.

## Reproduction

From the repository root:

```sh
python3 scripts/llm-summary-reference/run.py          # regenerate
python3 scripts/llm-summary-reference/run.py --check  # exact fixture comparison
```

Python 3.12+ (tar extraction filters), Git, the recorded Go toolchain, and already
cached Go dependencies are required. Defaults: `GOROOT=/usr/local/go`,
`GOCACHE=/workspace/scratch/go-cache`, `GOMODCACHE=/workspace/scratch/go/pkg/mod`.
These three paths can be overridden via the environment. The runner forces
`GOTOOLCHAIN=local`, `GOWORK=off`, `GOFLAGS=-mod=readonly`, `GOTELEMETRY=off`,
`GOPROXY=off`, and `GOSUMDB=off`.

The runner exports the exact pinned commit with `git archive`, injects only a new
package-`agent` test into a temporary directory, and calls the upstream internal
helpers. SDK implementation and existing tests are unchanged. No SDK CLI, evals,
network model calls, or provider credentials are used. A scripted in-process
`Model.GetResponse` captures requests and returns fixture responses/errors. No
native/Rust implementation is consulted to generate expected outputs.

Every reproduction executes the injected probe plus the 12 unchanged upstream
tests listed in `upstreamTestsPassed`. The runner verifies actual Go JSON `pass`
events for every selected name (not just the overall exit code). `--check`
compares all observations, toolchain version, and source/harness hashes. Wall
clock timestamps and temporary paths are not stored. A different Go version is
intentionally a provenance mismatch. JSON key order and indentation are stable.

## Coverage and interpretation

- Individual text trimming and truncation: 52 direct-helper vectors, including
  Unicode whitespace, combining characters, 2/3/4-byte UTF-8, zero-byte limits.
- Flattening: 44 vectors covering roles, ignored/nil payloads, empty tool calls
  and results, byte boundaries for messages/reasoning/tool input/results/errors,
  UTF-8 boundary backup, middle-out head/tail retention, exact whole-transcript
  boundaries, the default 240,000-byte budget, and prior-summary retention.
- Summarization: 18 vectors recording the actual request, instructions/settings,
  positive/default/negative timeout policy, parent deadline/cancellation,
  filtering and joining response messages, marker stripping, empty response,
  nil model, empty/ignored transcript, provider failure with/without response,
  all usage fields, and a full-budget request.
- Plans: 9 vectors exercising the real planner and explicitly constructed
  `CompactionPlan`s, deterministic fallback preservation, scope/marker insertion,
  preserved-item ordering (including deferred insertion after a user message),
  success/failure/empty/nil-model cases, and strict token-shrink boundaries.

Important pinned behavior:

1. Transcript/item limits are **UTF-8 bytes**, not Unicode scalar counts. The
   truncation marker is appended outside the byte budget. Middle-out retention
   allocates one third to the head and two thirds to the tail, then backs up the
   head and advances the tail to rune boundaries. Nonpositive whole-transcript
   budgets disable middle truncation; zero individual-text limits do not.
2. Prior-summary retention requires the exact agent name `context-summary` and
   the exact `[COMPACTED HISTORY SUMMARY]` prefix after whitespace trimming.
   The allowance is 16,000 bytes instead of 2,000; the whole-transcript budget
   still applies.
3. The summary request has one **user-role** message (nil agent), no tools,
   `max_tokens=2048`, and `reasoning_effort=low`; other settings remain zero/nil.
   The default timeout is 120 seconds. Timeout observations verify deadline
   policy without sleeping or serializing nondeterministic deadline timestamps.
4. Only nonblank message payloads are joined, after trimming each, with `\n`.
   Exactly one leading, case-sensitive summary marker is stripped. Usage is
   passed through unchanged, including requests and both cache counters, on
   success and empty-body failure. Provider errors discard response usage.
5. Application requires `candidateTokens < sourceTokens`, based on the SDK's
   item token estimator, not transcript byte length or `plan.After`. Rejection
   returns nil items and preserves the original deterministic plan. Empty and
   nonshrinking rejections retain usage; provider/nil-model errors return zero
   usage. The equality triplet records 34→34 rejected, 35→34 accepted, and
   33→34 rejected.

This is a helper-level oracle, not evidence of live-model quality, provider
integration, runner scheduling, or real elapsed-time timeout enforcement.
See [SCHEMA.md](SCHEMA.md) for the consumer contract.
