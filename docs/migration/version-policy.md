# Source and compatibility version policy

Baseline **adk-go-v0.0.115-1**, manifest schema **1**. The immutable inputs and dependency-file digests are in [source-lock.json](source-lock.json). Paths in inventories are relative to the named upstream checkout; a source URL is `repository/blob/revision/path#Lline`. Neither nested checkout is vendored into ADK.

## Reconciliation

At capture, both checkouts were clean. `git -C repos/sdk rev-list -n 1 v0.0.115` returned `1dc92b73900fac74dc357a938e4b5eee6392b418`, exactly the SDK checkout. Platform `go.mod:99` requires `github.com/gratefulagents/sdk v0.0.115`, with no replacement. `git -C repos/sdk log v0.0.115..HEAD` is empty. Thus there is **no checkout-versus-platform SDK version mismatch** in these inputs. This does not imply deployed workers or historical sessions use the same version.

Parent #1 requests platform `67fcfff804930a6679976aea97124cc7aa04e500`; the supplied checkout is one commit newer (`08e65c9`). `git diff --stat 67fcfff804930a6679976aea97124cc7aa04e500 HEAD` shows only three added lines in `.github/workflows/app-release.yml` (skip obsolete Android SDK tools). The module hash is identical and the worker/ABI source trees are unchanged. We retain the actual inspected checkout pin and record this harmless source discrepancy rather than silently claiming the epic revision was checked out.

Recent fixes already included in the baseline (not optional future enhancements):

| Commit | Behavior |
|---|---|
| `1dc92b7` | AnalyzeImage images passed directly to active model |
| `68c67a4` | OpenAI storage/extended caching disabled for image analysis |
| `07ccb85` | Codex 0.153.4 client version and GPT-6 Astra support |
| `0ab14b8` | Large clone survival and interrupted attach cleanup |
| `3538c45` | Durable child restore, session teardown, typed sub-agent resume errors |
| `aaeb911`, `df64444` | Updated Anthropic pricing tables |
| `fe4c7d6` | Prefer run model for vision analysis |

These descriptions are commit provenance, not substitutes for acceptance tests.

## Updating the baseline

1. Pin new full revisions first; verify the platform module requirement and its resolved source. Never silently replace the SDK checkout to match a floating tag.
2. Record the entire `old..new` commit range in `later_sdk_commits` with full revision, affected capability IDs, bug/feature classification, acceptance IDs, and adopt/defer decision with rationale. An empty range is explicitly recorded here.
3. Regenerate inventories, diff API/schema/default/config/CLI and platform closure changes, and classify each delta. An absent export is a removal, not an invitation to delete compatibility coverage.
4. Regenerate Go fixtures from their original provenance, review exact changes, rerun baselines, and retain the previous baseline in git history. Do not normalize away a newly failing comparison.
5. Increment the baseline suffix for capture/harness corrections; change the upstream version portion for source updates. Breaking fixture or ledger formats require a schema version increment. Rust runtime versioning is a separate decision.

## Interpretation and ownership

Destination modules in the ledgers are **proposed behavioral ownership**, not a requirement to duplicate Go packages/classes. Owner values are engineering roles, not invented human assignees. Acceptance identifiers are traceability anchors: unless a recorded executed test says otherwise they are pending obligations, not passing tests. `not implemented`/`unverified` is the expected Rust state before runtime work begins.

A Go test failure is evidence to triage: preserve the observed output, distinguish environment/provider requirements and upstream bugs, then decide the required behavior explicitly. Do not reproduce a defect solely because it appears in this snapshot. A Go test that passed is also not proof of Rust parity.

## Licensing and provenance

The SDK provides the GNU GPL v3 license text; platform provides GNU AGPL v3. Verbatim license texts are retained in [licenses/](licenses/). No claim is made that copied upstream declarations or derived fixtures become permissively licensed by residing in this repository. Preserve upstream file notices and pinned source/test references when redistributing or extending them; licensing review of any runtime code reuse is required. No blanket license for new ADK code is selected by this baseline.
