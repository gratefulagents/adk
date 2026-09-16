# Fixture provenance and license notices

These are deterministic **synthetic public test vectors**, not production captures.
No provider was contacted. No credentials, private conversations, or real encrypted
provider blobs are included. The words `sig`, `enc`, `sig_1`, `encrypted_1`, and
`provider-visible thinking` are literal data from the upstream tests.

Sources:
- SDK `github.com/gratefulagents/sdk`, commit
  `1dc92b73900fac74dc357a938e4b5eee6392b418` (local tag v0.0.115), root LICENSE:
  GNU General Public License version 3. Complete unmodified copy:
  `licenses/SDK-GPL-3.0.txt`.
- Platform `github.com/gratefulagents/gratefulagents`, commit
  `08e65c970830f05042c251bcbb46ec6a9e3719b9`, root LICENSE:
  GNU Affero General Public License version 3. Complete unmodified copy:
  `licenses/PLATFORM-AGPL-3.0.txt`.

Retain upstream ownership and these notices. No permissive relicensing of derived
fixtures or reference implementations is asserted. SDK-derived portions follow the
SDK license; platform-derived portions follow the platform license. The combined
reference contains both; review the retained license terms before redistribution.
Commit-addressed source URLs and SHA-256 checksums are in `manifest.json`; full source
is available in the attached repositories and at those URLs.

## Case provenance / modifications

| Fixture case | Actual Go origin and export path |
|---|---|
| model-items | SDK `internal/agent/llm_snapshot_test.go`, `TestBuildLLMResponseSnapshotCapturesReasoningAndRaw`; exported by actual `SnapshotRunItems` |
| model-response-explicit-false | Same test's synthetic model response, exported by `BuildLLMResponseSnapshot` |
| model-response-omitted-end-turn | Added minimal synthetic response exercising nil EndTurn in the same actual Go snapshot function; not claimed as an existing test vector |
| child-tool-0 | Added synthetic start partner for `TestContentEventLineHelpers`; actual `ParseContentEventLine` + `ChildToolEventFromContentEvent` |
| child-tool-1 | SDK `pkg/agentsdk/session_event_stream_test.go`, `TestContentEventLineHelpers`; actual parser/projection, error output and 42ms duration retained |
| state-ready-after-close-and-reopen | SDK `pkg/agentsdk/projectstate/filesystem_test.go`, `TestFilesystemStoreTaskLifecycleAndIndexes`; actual filesystem store initialize/create/depend/claim/close/reopen, exported event log + ready IDs |
| platform-all-item-types-roundtrip | Platform `cmd/agent/transcript_snapshot_test.go`, `sampleTranscriptItems` and `TestTranscriptSnapshotRoundTripPreservesAllItemTypes`; actual `persistedItemsFromRun`, `encodeTranscriptSnapshot`, `decodeTranscriptSnapshot` via virtual test overlay |

SDK export uses the public aliases to the same internal implementations. Platform
input is the SDK analysis snapshot of the existing sample items; expected output
is the real platform durable DTO after codec roundtrip (not a guessed JSON shape).
The normalized state timestamps/IDs/path are the only replacements. All other
values, omissions, array ordering, booleans, watermark numbers, and identifier
relationships remain as Go emitted them. The reference does not persist images;
Go baseline `TestPersistedItemsFromRunStripsImages` covers that separately.

Regeneration source lives in `scripts/replay/`, including the small synthetic
extensions described above. Normalization policy and known coverage limitations
are in its README and `docs/migration/baseline/README.md`.

## Runner execution replay (issue #4)

`runner*.json` adds synthetic model/tool-loop scenarios executed by the real Go
runner at the same SDK pin and compared with the Rust runtime. These are
SDK-derived GPL-3.0-only fixtures, not platform derivatives. Exact exporter/input
hashes and source links are in `runner_manifest.json`; execution and normalization
scope are documented in [runner replay](../scripts/replay/runner_README.md).
