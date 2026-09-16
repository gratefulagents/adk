# Platform migration baseline

In references below:

- **P/** = `repos/gratefulagents/`
- **S/** = `repos/sdk/`

Rust destinations and acceptance IDs below are **proposed ledger entries**, not implemented modules or passing tests.

## 1. Source lock and reconciliation

| Item | Inspected baseline | Decision |
|---|---|---|
| Platform | `08e65c970830f05042c251bcbb46ec6a9e3719b9` | Keep this actual inspected revision. |
| Epic platform pin | `67fcfff804930a6679976aea97124cc7aa04e500` | Git comparison shows only three added Android SDK configuration lines in `.github/workflows/app-release.yml:485–488`; no worker changes. |
| Platform SDK dependency | `v0.0.115` | `P/go.mod:99`. |
| SDK checkout and tag | Both `1dc92b73900fac74dc357a938e4b5eee6392b418` | Verified with `git rev-parse`; `v0.0.115..HEAD` is empty. No later-fixes range to import. |
| Go toolchain declarations | Platform `1.26.4`; SDK `1.26.2` | Record separately, using platform’s requirement for platform baseline execution: `P/go.mod:3`, `S/go.mod:3`. |

## 2. Exhaustive local import closure

A recursive import scan starting at platform `cmd/agent` and `internal/tools` found:

- **62 local packages:** 26 platform, 36 SDK.
- **470 production Go files.**
- **373 test files in those packages.**
- Including local imports from those tests adds **zero packages**.
- This is the **union of all source build constraints**, not a Linux-only `go list` result.
- Sorted production-file paths, joined with newline and a final newline, hash to:

```text
b42f09f901303e913d871b439ea457d83e6907daf77482e742120c481b6fbcca
```

### Platform packages — all 26

```text
api/platform/v1alpha1
api/triggers/v1alpha1
cmd/agent
internal/agentinfra
internal/agentplatform
internal/auth
internal/computeruse
internal/controller/triggers
internal/githubapp
internal/linear
internal/mcpattach
internal/mcppolicy
internal/orchestration
internal/projectstate
internal/security
internal/securitytoolpacks
internal/securitytoolrun
internal/slack
internal/store
internal/store/contentblob
internal/store/postgres
internal/store/postgres/sqlc
internal/store/sessionclient
internal/tools
internal/usageaccounting
rpc/auth
```

### SDK packages — all 36

```text
internal/agent
internal/agent/policy
internal/anthropic
internal/modelactivity
internal/modeldelta
internal/openai
pkg/agentsdk
pkg/agentsdk/durable
pkg/agentsdk/guardrails
pkg/agentsdk/mcp
pkg/agentsdk/memory
pkg/agentsdk/mode
pkg/agentsdk/modelsdev
pkg/agentsdk/otel
pkg/agentsdk/policy
pkg/agentsdk/projectstate
pkg/agentsdk/providers
pkg/agentsdk/providers/anthropic
pkg/agentsdk/providers/oauth
pkg/agentsdk/providers/openai
pkg/agentsdk/runtime
pkg/agentsdk/sandbox
pkg/agentsdk/tools
pkg/agentsdk/tools/browser
pkg/agentsdk/tools/fs
pkg/agentsdk/tools/git
pkg/agentsdk/tools/internal/pathutil
pkg/agentsdk/tools/lsp
pkg/agentsdk/tools/memory
pkg/agentsdk/tools/projectstate
pkg/agentsdk/tools/search
pkg/agentsdk/tools/shell
pkg/agentsdk/tools/signal
pkg/agentsdk/tools/vision
pkg/agentsdk/tools/web
pkg/agentsdk/tracestore
```

### Critical boundary finding

**An SDK-import-only inventory misses substantial required platform behavior. Conversely, package inclusion does not mean every package must be ported into the Rust worker.**

The concrete source of control-plane expansion is:

```text
cmd/agent/slack_command.go
    → internal/controller/triggers
        → internal/auth → rpc/auth
        → internal/githubapp
        → internal/linear
        → internal/security
        → internal/slack
        → internal/store
        → internal/orchestration
```

`slack_command.go` imports the controller package at line 15 and calls `CreateTriggerRun` at line 524. Its reusable run-building contract resides alongside controllers: `P/internal/controller/triggers/run_builder.go:34–79`.

**Recommended boundary:** port the Slack-facing creation/authorization/session contract, not all 47 production files of the trigger-controller package. Retain controller reconciliation as control-plane behavior. Separate the scanner execution contract from worker-side tool staging: `P/internal/securitytoolrun/contract.go:7–11`; `P/cmd/agent/loop.go:262–282`.

The closure generator records every production/test file and every import-edge witness, including non-SDK helpers.

## 3. Command, environment, and mount ABI

### Commands

| Command | Observable contract |
|---|---|
| `agent run` | Toolkit/git identity setup, preflight, conversational worker; errors exit 1, success 0. |
| `agent slack` | Same setup/preflight, then Slack connector; independently supported deployment command. |
| `agent desktop-bridge` | Exactly two argv entries required; stdin/stdout relay; bypasses toolkit/preflight; generic error and exit 1 on failure. |
| Missing/unknown command | Usage error, exit 1. Legacy `plan`/`execute` are not aliases. |

Sources: `P/cmd/agent/main.go:13–58`; legacy command test declarations at `main_test.go:52–62`.

Injected worker command is exactly:

```text
/opt/gratefulagents/bin/agent run
```

Both ordinary pods and SandboxClaim execution use it: `P/internal/controller/platform/pod_support.go:1676`, `sandbox_support.go:172`.

Slack deployment uses:

```text
/opt/gratefulagents/bin/agent slack
```

Source: `P/internal/controller/triggers/slackagent_helpers.go:224`.

Desktop bridge uses `PLANTASK_UID` and a five-second context: `P/cmd/agent/desktop_bridge.go:13–16`. Its socket path is:

```text
/tmp/gratefulagents-desktop-<first-16-SHA256-bytes-of-UID-as-hex>/relay.sock
```

Directory mode `0700`, socket mode `0600`: `P/internal/computeruse/socket.go:17–22,31–36,54–60`.

### Worker launch configuration

Required by `loadRunConfig`:

```text
POD_NAMESPACE
PLANTASK_NAME
PLANTASK_UID
MODEL
```

Defaults and optional inputs:

- `WORKSPACE_DIR=/workspace`; repository directory `<workspace>/repo`.
- Empty `REPO_URL` means repoless operation.
- `BASE_BRANCH=main`; optional `REPO_REVISION`, comma-separated `ADDITIONAL_REPO_URLS`.
- Optional `GH_PAT`.
- Provider selected from model plus `AI_PROVIDER`, default `openai`.
- `AI_AUTH_MODE` precedes `OPENAI_AUTH_MODE`.
- `OPENAI_BASE_URL`, `OPENAI_API_MODE`, `MODEL_FALLBACKS`.
- `AI_DEBUG` accepts case-normalized `1` or `true`.

Sources: `P/cmd/agent/config.go:75–105,158–194`.

Normal startup additionally requires Postgres and workspace S3 configuration; these are not merely optional observability integrations:

- `DATABASE_URL`: `P/cmd/agent/session.go:19–23`, `durable_run.go:25–45`.
- `S3_BUCKET`; default `S3_REGION=us-east-1`; optional absolute `S3_ENDPOINT` enables path-style addressing.
- `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` must appear together.
- Startup aborts on checkpoint-store/load failure.

Sources: `P/cmd/agent/workspace_checkpoint_store.go:42–78`; `plan.go:349–393`.

### Credentials and dynamic environment

- Infra credentials originate in Secret `gratefulagents-worker-infra`, keys `aws-access-key-id`, `aws-secret-access-key`, `database-url`; pods use Secret references rather than inline secret values: `P/internal/controller/platform/pod_support.go:446–469,1556–1560`.
- Legacy OAuth mount: `/var/run/gratefulagents/openai-oauth/{auth.json,account-id}`. Anthropic/Copilot can use the legacy auth mount too: `pod_support.go:34–42,781–793`.
- Additional OAuth mounts use `/var/run/gratefulagents/oauth`; ordered fallback environment lists are provider-specific. OpenAI auth/account-id fallback arrays are index-aligned: `P/cmd/agent/config.go:208–233`.
- MCP secret environment is dynamically server-scoped:

```text
GRATEFULAGENTS_MCP_<uppercase hex first 6 SHA256 bytes>_<sanitized env name>
```

Hash input is `trim(serverName) + NUL + trim(envName)`: `P/internal/mcpattach/secret_env.go:15–18`.

A literal `os.Getenv` scan alone is **not exhaustive**: helper calls, provider-derived key names, SDK sandbox constants, MCP-generated names, and mode-constraint forwarding must also be inventoried.

### Mount and process contract

- Read-only toolkit: `/opt/gratefulagents`, `subPath=gratefulagents`.
- Writable EmptyDirs: `/workspace`, separate `/workspace/scratch`.
- Optional read-only instructions: `/etc/operator-instructions`.
- Worker UID/GID: `1100`.
- Restart policy: `Never`; termination grace: `60s`.
- Init container `inject-toolkit` populates the shared toolkit volume.
- Toolkit PATH is assembled by the worker, not replaced by the controller; writable HOME falls back to `/tmp/home`.

Sources: `P/internal/controller/platform/pod_support.go:71–74,1585–1637`; `P/cmd/agent/toolkit_env.go:30–42,58–89,119–133`.

### Feature/control inputs

- Browser defaults on, conditional on Chromium availability.
- Terminal and async Bash default on; SDK write-mode/OS restrictions still apply.
- Project state and critic/verifier default on.
- Registry narrowing: `AGENTRUN_ALLOWED_TOOLS`, `AGENTRUN_DENIED_TOOLS`.
- Structured result: `AGENTRUN_TASK_OUTPUT_SCHEMA`.
- Current/parent/supervised/maintained identities use distinct `AGENTRUN_*` variables. Legacy `RUN_*` aliases refer to the **parent**, not current run.
- Checkpoint stage overrides: `WORKSPACE_CHECKPOINT_PREPARE_TIMEOUT`, `WORKSPACE_CHECKPOINT_UPLOAD_TIMEOUT`; defaults 5m/60s, shutdown 20s/10s.

Sources: `P/cmd/agent/loop.go:36–45,149–184,287–301`; `project_state.go:273,435`; `P/internal/controller/platform/pod_support.go:1734–1764`; `workspace_checkpoint_budget.go:19–30`.

Slack requires app and bot tokens, defaults health address to `:8080`, and supports dedicated-agent versus workspace connector configuration: `P/cmd/agent/slack.go:85–108,151`; `slack_workspace.go:48–54`.

## 4. AgentRun wire contract

CRD phase strings are exactly:

```text
Pending Admitted WaitingApproval Provisioning Running Question Blocked
Paused Succeeded Failed Cancelled
```

Input-request types:

```text
question approval plan_review turn_limit idle circuit_breaker stopped
```

Source: `P/api/platform/v1alpha1/agentrun_types.go:44–73`.

Do not conflate these with lowercase Postgres session phase values: initial DB default is `pending`, `P/internal/store/postgres/migrations/001_initial_schema.up.sql:5–17`.

Status includes queue/sandbox/artifacts/policy/metrics; session and retry counters; team/overseer/children; timestamps; wake/restart handled counters; immutable mode snapshot and revisions; completion request; structured output; conditions. Preserve JSON field names from `P/api/platform/v1alpha1/agentrun_types.go:742–826`.

Notable representations:

- CRD `metrics.costUsd` is a string; worker formats four decimal places.
- Session metrics use numeric `cost_usd`.
- Postgres is primary for progress when the session client exists; CRD-only writes are fallback.

Sources: `P/cmd/agent/run_status.go:182–224`; `P/internal/store/sessionclient/metadata.go:21–35`.

Canonical annotations use prefix `platform.gratefulagents.dev/`:

```text
authorization-pending
interrupt-requested-at
overseer-verdict
overseer-guidance
overseer-summary
overseer-input-response
overseer-detaching
review-verdict
review-summary
git-author-name
git-author-email
```

Sources: `P/api/platform/v1alpha1/agentrun_types.go:76–133`.

`interrupt-requested-at` carries RFC3339 time; removing it acknowledges the fallback request. Postgres is primary. `cleanup` is a **finalizer**, not an annotation; `bug-report-id` is a **label**: same file, lines 80–86 and 135–142.

Legacy `workflowMode=chat` is normalized to autonomous pacing rather than restoring old chat execution semantics: same file, lines 15–23.

## 5. Postgres transactions and state

### Message delivery

The authoritative lifecycle is:

```text
pending → claimed → completed
pending → cancelled
```

Migration 036 adds claim timestamps, UUID claim tokens and global delivery sequence; legacy metadata is backfilled: `P/internal/store/postgres/migrations/036_message_lifecycle.up.sql:1–37`.

Polling is **not cursor-based**. It selects pending user messages ordered by ID and stops before the earliest `overseer_held` message: `P/internal/store/postgres/queries/messages.sql:38–54`.

Claiming is a single conditional UPDATE of a pending row. Claim versus cancellation races on `delivery_state`; it is **not** a `SKIP LOCKED` work queue: `P/internal/store/postgres/store.go:1101–1127`.

Assistant commit contracts:

- Ordinary completion atomically completes claims and appends one assistant response; no remaining claim is an error: `store.go:1130–1158`.
- Durable completion updates claims and inserts an assistant response keyed by `metadata.durable_pass_key`; conflict reloads the existing response: `store.go:1161–1195`.
- Migration 042 supplies the unique session/pass-key index: `migrations/042_sdk_durable_runs.up.sql:29–31`.
- Recovery resets claims belonging to a different claim token to pending: `store.go:1206–1212`.

### Other transactional contracts

- Metadata-section read/modify/write locks `agent_sessions ... FOR UPDATE` and merges only the relevant top-level key: `store.go:696–737`.
- Interrupt consumption **does** use ordered `FOR UPDATE SKIP LOCKED`, atomically setting `consumed_at`: `store.go:771–792`.
- Wake reservation takes `pg_advisory_xact_lock(hashtext(sessionUUID))`, deduplicates by session/idempotency key, monotonically reserves a wake target and commits its user message plus intent together: `store.go:795–852`.
- Applied-wake marking is separate and idempotent: `store.go:866–869`.

### Notifications and activity

`change_seq` is a per-session monotonic write fingerprint. `session_change` notifications are **lossy hints**, not authoritative delivery. Statement-level child-table triggers avoid inconsistent lock ordering: `P/internal/store/postgres/migrations/056_session_change_seq.up.sql:3–13,33–60`.

Migration 057 adds interrupt inserts to this mechanism: `057_interrupt_change_seq.up.sql:1–19`.

Clients subscribe **before querying**; healthy notifications permit a 30-second safety poll, with reconnect probing: `P/internal/store/sessionclient/client.go:44–58,91–114`.

Activity is not the SDK durable event log:

- Async writer caps: 65,536 events / 64 MiB.
- Drops oldest entries under pressure, preserving the newest even if individually oversized.
- Batch size 64; close timeout 5s.
- Batch SQL preserves input order using ordinality.
- These loss/backpressure semantics must not be mistaken for exactly-once durable event persistence.

Sources: `P/cmd/agent/pg_event_writer.go:57–72,90–119,150–175`; `P/internal/store/postgres/activity_batch.go:13–17,38–48`.

### SDK durable storage

Platform migration 042 and SDK `PostgresStore.Init` agree on:

```text
durable_runs:
  PK (tenant_id, run_id)
  revision, event_sequence, snapshot BYTEA, retain_until,
  created_at, updated_at, lease_owner, lease_token, lease_until

durable_events:
  PK (tenant_id, run_id, sequence)
  body BYTEA
  cascading FK to durable_runs
```

Sources: `P/internal/store/postgres/migrations/042_sdk_durable_runs.up.sql:1–27`; `S/pkg/agentsdk/durable/postgres.go:44–69`.

Append locks the run, validates unexpired lease token and expected revision, inserts ordered events and updates the snapshot in one transaction: `S/pkg/agentsdk/durable/postgres.go:158–212`; `durable/store.go:50–78`.

Platform identities:

```text
tenant = k8s-namespace-<namespace>
owner  = <namespace>/<AgentRun name>/<hostname>
run ID = agentrun-<TaskUID>-message-<messageID>-pass-<pass>
lease TTL = 30 seconds
```

Sources: `P/cmd/agent/plan.go:371–377`; `durable_run.go:18,91–105`.

**Encryption distinction:** the platform constructs `NewPostgresStore(db)` without encryption options. Its SDK durable JSON is therefore application-level plaintext BYTEA, despite `DataSensitive` classification. This does not establish whether infrastructure encryption is configured. Sources: `P/cmd/agent/durable_run.go:40,100–105`; `S/pkg/agentsdk/durable/postgres.go:28–36,327–334`.

## 6. Independent format/version matrix

These numbers are unrelated version domains and must never share one migration switch.

| Format | Version / representation | Source |
|---|---|---|
| SDK durable document/snapshot | `schema_version=2` | `S/pkg/agentsdk/durable/types.go:15–16` |
| SDK runner continuation checkpoint | `schema_version=1` | `S/internal/agent/durable_checkpoint.go:12–13,37–49` |
| Platform transcript snapshot | `version=1`, gzip JSON | `P/cmd/agent/transcript_snapshot.go:37–43,63–103` |
| Platform subagent envelope | `version=1`, `saved_at`, scheduler `state` | `P/cmd/agent/subagent_checkpoint.go:17–20,59,87–88` |
| SDK project-state documents/indexes | `schema_version=1` | `S/pkg/agentsdk/projectstate/types.go:9,52–58` |
| Platform Postgres project state | Migration-defined tables, not SDK filesystem documents | `P/internal/store/postgres/migrations/015_project_state.up.sql:6–61` |
| Workspace manifest | `version=1` | `P/cmd/agent/workspace_snapshot.go:34,90–110` |
| Workspace key record | `version=1`, raw-base64 key | `P/cmd/agent/workspace_untracked_snapshot.go:29–46,80–88` |
| Workspace encryption envelope | Magic `GAWS\x01` | Same file, lines 35,108–161 |
| Security execution manifest | `security-tool-job-manifest/v1` | `P/internal/securitytoolrun/contract.go:48–49` |

### Transcript and working state

Transcript envelope preserves floor/seen/self-assistant/pending-user message IDs. Item types are stable strings:

```text
message tool_call tool_output handoff_call handoff_output
reasoning tool_approval compaction
```

Images are stripped; producing agent identity retains presence plus name. Default compressed cap is 4 MiB; `TRANSCRIPT_SNAPSHOT_MAX_BYTES<=0` disables persistence. Unknown item types invalidate the snapshot. Sources: `P/cmd/agent/transcript_snapshot.go:37–57,63–173`.

Storage is one upserted cascading row per session, not an event log: `P/internal/store/postgres/migrations/025_session_transcripts.up.sql:1–12`.

Separate session metadata sections are `metrics`, `working_state`, `subagent_checkpoint`. Working state includes history floor, stopped-message floor, provider response ID and durable message/pass counters; recent summaries cap at six: `P/internal/store/sessionclient/metadata.go:13–18,38–80`.

### Project state

Platform persists tasks, memories and summaries under `(project_id,id)` composite keys. Memory embeddings are `vector(1536)`: migration 015, lines 6–61.

Project ID is canonical namespace/repository identity plus the first six SHA256 bytes as hex; repoless runs use sanitized `<namespace>-chat`: `P/internal/projectstate/store.go:1078–1089`.

OpenAI embeddings are optional; absent `OPENAI_API_KEY`, recall falls back to lexical behavior: `P/cmd/agent/project_state.go:294–324`.

Project-content version bodies are a separate format:

```text
project-content/v1/<content UUID>/<version>-<SHA256>
```

Current writes retain BYTEA compatibility copies while optionally writing S3. Migration 039 adds object locators and a deletion outbox; do not silently remove dual-write compatibility. Sources: `P/internal/store/postgres/project_content.go:534–562`; `migrations/039_project_content_s3.up.sql:1–21`.

## 7. Encrypted S3 workspace ABI

Run prefix:

```text
workspace-checkpoints/v1/<namespace>/<TaskUID>
```

Objects:

```text
<prefix>/latest.json.enc
<prefix>/objects/<repository ID>/<SHA256 plaintext bundle>.bundle.enc
<prefix>/anchors/<SHA256 plaintext anchor payload>.pack.enc
```

Sources: `P/cmd/agent/workspace_checkpoint_store.go:20–22`; `workspace_snapshot.go:113–118,593–617`.

Manifest fields:

```text
version, generation, createdAt, repositories[]
repository:
  id, alias?, url, branch?, upstream?, location?, primary?,
  semanticKey, objectKey, snapshot, parent, anchor?, anchorObjectKey?
```

Validation rejects unsupported versions, incomplete/duplicate repository entries, more than one primary, and object locators outside the run’s objects/anchors prefixes: `workspace_snapshot.go:90–165`.

Publication contract:

1. Produce/upload encrypted repository bundles and any anchors.
2. Sort repository entries by ID.
3. Compute generation as SHA256 of each JSON entry followed by NUL.
4. Publish encrypted latest manifest only after every payload succeeds.
5. A mutating tool boundary fails closed if checkpoint publication fails.

Sources: `workspace_snapshot.go:60–74,461–518,593–619`.

Cryptography:

- Per-session 32-byte random key stored under private session metadata key `workspace_snapshot_encryption_key`.
- Record `{version:1,key:<base64.RawStdEncoding>}` persisted before use.
- AES-256-GCM envelope: `GAWS\x01 || nonce || authenticated ciphertext`.
- AAD is exactly the magic bytes—not run identity or object key.
- Maximum encrypted payload/read size: 512 MiB.
- Key records and manifests have independent version checks.

Sources: `P/cmd/agent/workspace_untracked_snapshot.go:28–46,54–105,108–161`; `workspace_checkpoint_store.go:96–121`.

**Tradeoff:** synchronous publication protects acknowledged mutations across pod replacement but adds storage latency and makes S3 failure a tool/run failure. Preserve this ordering first; any asynchronous relaxation is a separate compatibility decision.

## 8. Proposed Rust capability ledger

All entries below: **source inspected; Rust implementation and replay verification pending**. Acceptance IDs are proposed.

| Capability / explicit source families | Rust destination | Role / owner | Acceptance ID and Go provenance |
|---|---|---|---|
| CLI, config, toolkit, provider/OAuth material | `agent_worker::bootstrap` | Worker / runtime | `PLAT-ABI-001`; `cmd/agent/main_test.go:10–72` |
| Pod/SandboxClaim launch, mounts, secrets, termination | `platform_contracts::launch` | Shared contract; controller remains control-plane | `PLAT-ABI-002`; `internal/controller/platform/pod_support.go:1536–1644` |
| AgentRun schema/status/annotations, progress | `platform_contracts::agentrun` | Shared / platform | `PLAT-ABI-003`; `cmd/agent/run_status.go:182–224` |
| Claims, cancellation, interrupts, wake intents | `platform_store::sessions` | Shared / persistence | `PLAT-PG-001`; `internal/store/postgres/store_test.go:367,425,456` |
| Activity buffering, ordering, notification hints | `agent_worker::events`, `platform_store::activity` | Worker + shared | `PLAT-PG-002`; `cmd/agent/pg_event_writer_test.go:268,418,479,507` |
| SDK lease/pass/session bridge | `agent_worker::durable` | Worker / runtime | `PLAT-DUR-001`; `cmd/agent/durable_run_test.go:8,35,50` |
| Transcript, working state, subagent continuation | `agent_worker::state` | Worker / runtime | `PLAT-STATE-001`; `cmd/agent/transcript_snapshot_test.go:109,196,205,245,318` |
| Project tasks, memories, summaries, identity | `platform_store::project_state` | Shared / persistence | `PLAT-STATE-002`; `internal/projectstate/store_test.go:33,52,90,113,129` |
| Workspace checkout, bundles, anchors, encryption, restore, budgets | `agent_worker::workspace` | Worker / runtime | `PLAT-S3-001`; `cmd/agent/workspace_snapshot_test.go:208,257,341,364,384,414` |
| Project-content bodies and deletion outbox | `platform_store::content` | Shared / persistence | `PLAT-S3-002`; `internal/store/postgres/project_content.go:534–562` |
| Git/GitHub, PR review, plan/finish, skills, teammate, structured output | `platform_tools::{git,github,flow,skills,teammates}` | Worker / tools | `PLAT-TOOLS-001`; `cmd/agent/loop.go:210–221,248–254,287–312,329–339` |
| Registry permission/name gates, MCP attachment/materialization/break-glass | `platform_tools::policy`, `agent_worker::mcp` | Worker + shared | `PLAT-TOOLS-002`; `internal/tools/registry.go:195–255,271–319`; `cmd/agent/loop.go:341–346` |
| Maintainer fleet/work-item commands and overseer tools | `platform_tools::{maintainer,overseer}` | Worker client of control-plane | `PLAT-TOOLS-003`; `cmd/agent/loop.go:222–246`; `internal/tools/maintainer_workitem_commands.go:21–57` |
| Findings, research, hypotheses, coverage, variant sweeps, artifacts, PoCs/bounties, scanner jobs | `platform_tools::security`, `platform_contracts::security_job` | Worker + shared; job/controller separately owned | `PLAT-TOOLS-004`; `cmd/agent/loop.go:255–285`; `internal/securitytoolrun/contract.go:7–49` |
| Slack connector, dispatch/drafts/sessions/files/home/interactions; read tools | `platform_connector::slack`, `platform_tools::slack` | Connector/control-plane-facing + worker tools | `PLAT-SLACK-001`; `cmd/agent/slack_command.go:50–82,524`; `loop.go:321–327` |
| Desktop bridge/broker/protocol and computer-use tool | `agent_worker::desktop` | Worker / desktop integration | `PLAT-DESKTOP-001`; `cmd/agent/desktop_bridge.go:13–30`; `internal/computeruse/socket.go:17–60` |
| Admin introspection, traces, metaharness, metrics/cost | `platform_tools::admin`, `agent_worker::observability` | Worker / observability | `PLAT-OBS-001`; `cmd/agent/loop.go:313–318`; `run_status.go:154–224` |
| Trigger reconciliation, GitHub token refresh, Linear integration, auth server | Retain Go control plane; expose narrow contracts | Control-plane / platform | `PLAT-BOUNDARY-001`; `cmd/agent/slack_command.go:15,524`; `internal/controller/triggers/run_builder.go:34–79` |

Registry defaults require special care: standalone platform registry starts workspace-write, but worker startup resolves missing RuntimeProfile to read-only. Deny wins over allow; `finish`, `save_plan`, `get_plan`, `RequestMCPBreakGlass` are exempt control-flow tools. Sources: `P/internal/tools/registry.go:195–200,271–319`; `P/cmd/agent/plan.go:328–346`.

A `Name()` declaration scan is **not** a complete tool inventory: SDK-backed wrappers and dynamic names also exist. Use worker registration composition plus SDK ledger, not the method-name count as the number of tools.

## 9. Materialized evidence and remaining acceptance gaps

The architect findings above are now materialized. See the [artifact guide](README.md) for commands, extraction boundaries and licensing/provenance.

Completed evidence:

- [Closure inventory](closure.json): **62 packages (26 platform / 36 SDK), 470 production files, 373 tests**, zero test-import package additions; production path SHA-256 `b42f09f901303e913d871b439ea457d83e6907daf77482e742120c481b6fbcca`.
- Every package has a proposed Rust destination/owner, unique acceptance ID, capability association, disposition, source-test list and explicit `not_implemented` / `not_run` Rust status. SDK package destinations align with its ledger; finer per-symbol routing remains there.
- Exact source content/hashes are archived for **278 platform production files**, including all **50 internal/tools files**. Complete schemas/defaults and dynamic wrappers are preserved, not reduced to method-name counts.
- [Environment inventory](environment.json): **1,178 tracked Go files**, **1,979 environment expressions**, **606 enclosing declarations**, **293 literal candidates**, constant definitions and **6,564 supplementary deployment/shell candidates**. Dynamic provider, MCP, sandbox and mode-forwarding expressions are retained. The exhaustive claim is syntactic expression coverage, not runtime dataflow resolution or third-party implicit reads.
- Registration composition: **606 expressions**, including **46 worker-loop expressions**. Metadata inventory: **1,249 literal/declaration/access expressions**, including trigger/maintainer runtime-trigger-name, generated-runtime, project identity, review-round and constant-key annotation/label accesses.
- `scripts/platform/verify.py` verifies counts/hash, source witnesses, package assignments, all tool files, key dynamic expressions, AST agreement with the pinned regex import scan, and byte-identical regeneration. AST regression tests and `go vet` pass; see [fresh verification log](verification.log).
- The separate [Go baseline and offline replay evidence](../baseline/README.md) records **445 passing Go test/subtest events, 17 Python harness tests and 7 Go-exported replay cases**. Its reports contain commands, limits and failure history. These are Go/reference-adapter results, **not Rust parity**.

Remaining acceptance gaps are substantive rather than missing handoff artifacts: real PostgreSQL message-claim races/wake idempotency/lease CAS; controller/Kubernetes integration; corrupt or unknown transcript versions; failed bundle/manifest publication; and Rust equivalents of these contracts. Seven replay cases are a seed, not comprehensive contract coverage. No Rust implementation or Rust replay has been run.

Database tests require `TEST_DATABASE_URL`, run migrations, and delete shared test tables. They must use a disposable database; skipped database tests are not parity evidence: `P/internal/store/postgres/store_test.go:19–43`.

Normalization must preserve message/pass/claim relationships, event ordering, transcript watermarks, manifest repository ordering and generation derivation. Random IDs, timestamps and encryption nonces may be replaced consistently, but ciphertext byte equality is not an appropriate replay assertion.

