# Durable typed-record wire proof

## Result and scope

**80 / 80 candidate field acceptance IDs proven; 0 blocked.** This is a wire-field proof only, not closure of recovery-helper, store, backend, transaction, or exactly-once obligations. No ledger, inventory, source lock, project scripts, or upstream source edits; no commits.

Pinned reference: clean `repos/sdk` at `1dc92b73900fac74dc357a938e4b5eee6392b418`, `pkg/agentsdk/durable/types.go`. Acceptance IDs were read from `docs/migration/ledger/issue-11-overlay.json` without modifying it. The machine-readable companion is [durable-record-proof.json](durable-record-proof.json).

Counts: RunSnapshot 19; Effect, Event, ToolCall 8 each; Step 6; Approval, BudgetCounters, ChildRun, Lease 5 each; Attempt, Cancellation 4 each; RecoveryDecision 3.

## Independent oracle and assertions

`fixtures/durable/generate.go` imports the pinned Go package, constructs populated values, and **decodes each input into the actual Go record type before marshaling the expected object**. `records.json` contains 182 cases: 140 timestamp variants (14 per time-bearing record) plus 42 baseline cases: populated, omitted (`{}`), and every-field-null input for all 12 records, plus unknown-string cases for the six records containing string aliases. No Rust output creates expectations. Populated cases set every field nondefault, including completion/resolution/acknowledgment times and optional strings.

Each named `record_*` Rust test directly accesses its typed Rust field, compares it with the independently generated expected field, and compares the entire Rust encoding with the expected Go object. For omitted expected fields it additionally checks the typed field's default. Whole-object comparison distinguishes a missing key from explicit JSON null; it is not merely a Rust self-roundtrip. Every field test runs all cases belonging to that record. The companion gives exact input and expected JSONPaths for each case, recording expected-field presence explicitly.

`go_decodes_rust_records_against_independent_expected_json` (with `ADK_TEST_GO=1`) writes Rust encodings and invokes Go `verify-records`: Go compares the raw Rust encodings with independent expectations, decodes all 182 into actual record types, re-encodes them, and compares again using `json.Decoder.UseNumber`. Neither comparison normalizes any timestamp. It passed with: `Go decoded and compared 182 Rust-encoded typed record cases against independent Go expectations`.

Numbers retain exact integers, including `9007199254740993` in revision/event sequence/budget/event payload and `18446744073709551615` in raw snapshot state. `large_integers_nanosecond_instants_and_explicit_null_remain_exact` explicitly checks these, 123456789/123456796 nanoseconds, raw null versus omission, and year-one zero time. Actual timestamp strings, opaque strings, and raw JSON retain their exact Go spellings in comparisons; only object ordering and Go HTML escaping are not byte-identity promises. Positive/negative minute offsets (`+05:45`, `-03:30`), 0–9 fractional digits, nanosecond leading zeros, fractional trailing-zero trimming, explicit `+00:00`/`-00:00` input becoming `Z`, year-one zero, nested records, Unicode, and optional deadlines are included.

`unknown_wire_strings_survive_v1_migration` compares Rust migration plus encoding directly with `v1-unknown-migrated.json`, produced by Go `DecodeDocument`, and asserts exact unknown run-status and event-classification strings containing Unicode, spaces, punctuation, and newline. Existing `go_fixtures_migration_full_types_and_recovery` continues to pass against original migration/full-type/known-recovery fixtures.

## Codec changes and safety boundary

- Persisted timestamps (including optional retention and lease expiry) now use `DateTime<FixedOffset>`. Record serializers retain offsets, trim fractional trailing zeros like Go, and use `Z` for UTC. UTC-taking helper APIs stay UTC and convert explicitly. V1 migration deliberately converts snapshot/cancellation timestamps to UTC, matching the actual pinned Go migration; append also synthesizes a missing event timestamp in UTC. These are product behaviors, not comparator normalization.

- `RunStatus`, `DataClassification`, `EffectClassification`, and `EffectState` retain known variants and represent exact future strings with `Unknown(String)`. `RecoveryAction` receives the same treatment because `RecoveryDecision.action` is also a Go string alias and one of the 80 fields. These enums are no longer `Copy`.
- Go omitted/null cases exposed missing defaults in `Effect`, `Lease`, and `RecoveryDecision`, and rejection of scalar/time/nested-budget null fields. The codec now supplies fresh Go zero values, including year-one times and empty strings. Raw JSON null remains present; optional pointers and null slices remain absent/empty as appropriate.
- `recover_effect` returns `RecoveryAction::None`, `automatic=false`, preserving the key, for any unknown effect classification or state, including an empty unknown string. `transition_effect` rejects unknown current/next states without mutation. Regression: `unknown_effect_values_never_authorize_recovery_or_state_transitions`.
- **Intentional safety divergence:** Go `RecoverEffect` automatically retries dispatched/outcome-unknown effects with unknown classifications, and retries prepared effects regardless of classification. Rust must not authorize such automatic work. **Recovery-helper behavior obligations remain unclaimed**, even though known-value regressions and the intentional fail-closed boundary are tested.
- Store changes are only `.clone()` adaptations for non-`Copy` classifications. No store/backend guarantees changed.

The omitted/null proof concerns fresh-record field decoding, not unmarshalling into prepopulated values, every malformed input, root-level null records, or null elements inside record arrays.

## Upstream reference association

Both pinned upstream references were independently rerun and passed:

- `TestDecodeDocumentMigratesV1`: contextual association for snapshot schema/cancellation/budget, input tokens, and event tenant/run/sequence normalization.
- `TestEffectRecoveryAndTransitions`: contextual association for effect ID/key/classification/state and recovery action/automatic fields.

The companion associates those specific IDs where appropriate; an empty association means neither reference directly establishes that field. **Neither reference is standalone field proof.** The 80 named direct-field tests and independent Go codec expectations are the primary evidence.

## Timestamp correction and independent negative checks

The previous 80-field proof normalized typed timestamps to UTC and therefore masked wire offset loss. That claim was insufficient for durable format parity. This correction removes normalization from both comparators and also compares raw Rust JSON before Go re-encoding, so Go's own canonicalization cannot hide an incorrectly padded fraction. The test oracle remains exclusively the clean pinned Go SDK.

`wire_comparison_rejects_changed_timestamp_strings` rejects different spellings at typed, nested, optional and opaque locations. Live Go `verify-records` rejects eight mutations: `Event.at`, `RunSnapshot.created_at`, `retain_until`, nested `Step.started_at`, nested `Cancellation.requested_at`, a padded fraction in `Event.at`, `Event.payload.at`, and `Step.status`.

`fixed_offsets_survive_filesystem_persistence` verifies create/load/append in plaintext and encrypted stores, preserving nested/optional offsets and a supplied event time; a missing event time is synthesized in UTC just as in Go. The existing v1/v2 contract test now compares the actual encoded JSON values against Go fixtures, not just chrono instant equality.

All 80 unique acceptance IDs were checked against the read-only ledger's Go field names and named Rust assertions. The companion's case paths/presence flags now include every relevant timestamp case. These are **field-only claim candidates**, not ledger updates.

## Fresh verification

Commands used the requested Rust 1.88 toolchain/cache/target environment and pinned Go checkout.

- `ADK_TEST_GO=1 cargo test --locked -p adk-durable`: **17 contract tests + 85 record tests passed**.
- `ADK_TEST_GO=1 cargo test --locked -p adk-durable --no-default-features`: **11 contract tests + 85 record tests passed**.
- A focused `go_decodes_rust_records_against_independent_expected_json -- --nocapture` rerun printed **182 cases compared** and **all 8 mutations rejected**. Plaintext/encrypted Go filesystem continuation ran in both feature configurations.
- PostgreSQL tests without `ADK_TEST_POSTGRES_URL` and worker-only entry points return early; live database verification is **not claimed**.
- Generator run and repeat from clean SDK `1dc92b73900fac74dc357a938e4b5eee6392b418`: **all 11 JSON fixtures byte-identical on repeat**. SHA-256 hashes are in the companion.
- Pinned Go `TestDecodeDocumentMigratesV1` and `TestEffectRecoveryAndTransitions`, rerun with `-count=1`: **both passed**.
- Direct-driver Clippy: `RUSTC_WORKSPACE_WRAPPER=<rust188>/bin/clippy-driver CLIPPY_ARGS='-D__CLIPPY_HACKERY__warnings' cargo check --locked -p adk-durable --all-targets`: **passed**.
- `rustfmt --edition 2024 --check` on owned modified Rust files, `gofmt -l` (empty), owned-path `git diff --check`, and Go vet of the generator: **passed**.
- Initial runtime integration exposed E0308 at the stored checkpoint timestamp assignment. Parent applied `next.updated_at = checkpoint.created_at.fixed_offset();`; the integrated workspace tests and strict Clippy now pass. Parent also added executable-resume guards for unknown effect state/classification and stored run status, with no-execution regressions and scoped re-review.

Initial correction failures were not suppressed: fixture traversal first confused numeric `BudgetCounters.tool_calls` with the snapshot array; it now checks the array shape. The first exact migration comparison exposed Go's explicit v1 UTC conversion; Rust now matches it explicitly. Both full durable reruns passed after those corrections. Some Go invocations emitted an unavailable `/proc/self/exe` telemetry-sidecar warning but returned zero.

## Remaining claim boundaries

**80 scoped record-field candidates pass exact Go JSON-value comparison**, including timestamp strings. The parent resolved the timestamp assignment and executable-resume guards; integrated workspace validation passes. Recovery-helper parity remains unclaimed because of the documented fail-closed divergence for unknown effect classifications/states. Store/backend/transaction/exactly-once closure and live PostgreSQL verification remain outside this field proof. Fresh-record missing/null fields are covered; unmarshalling into prepopulated records, root-level null records, null array elements, malformed-input exhaustiveness and all workspace behavior are not claimed. No ledger/source-pin edits or commits were made.

## Acceptance ID → Rust field → independent Go JSONPath → named Rust assertion

All rows below are **proven**. JSONPaths are within `fixtures/durable/records.json`; named tests are in `crates/adk-durable/tests/record_codecs.rs`. Each displayed populated-field path is supplemented by omitted/null/unknown case paths in the JSON companion.

| Acceptance ID | Rust field | Go expected JSONPath | Named Rust assertion |
|---|---|---|---|
| `SDK-14B557B50B8753D8` | `RunSnapshot::schema_version` | `fixtures/durable/records.json::$[0].expected.schema_version` | `record_runsnapshot_schema_version` |
| `SDK-8123E3663E5A6B3D` | `RunSnapshot::tenant_id` | `fixtures/durable/records.json::$[0].expected.tenant_id` | `record_runsnapshot_tenant_id` |
| `SDK-5EAE8CCA60B01B14` | `RunSnapshot::run_id` | `fixtures/durable/records.json::$[0].expected.run_id` | `record_runsnapshot_run_id` |
| `SDK-129EE3451642C93B` | `RunSnapshot::revision` | `fixtures/durable/records.json::$[0].expected.revision` | `record_runsnapshot_revision` |
| `SDK-D16611759C822DC6` | `RunSnapshot::event_sequence` | `fixtures/durable/records.json::$[0].expected.event_sequence` | `record_runsnapshot_event_sequence` |
| `SDK-559F356BBE207D70` | `RunSnapshot::status` | `fixtures/durable/records.json::$[0].expected.status` | `record_runsnapshot_status` |
| `SDK-D35CEBC91498576B` | `RunSnapshot::classification` | `fixtures/durable/records.json::$[0].expected.classification` | `record_runsnapshot_classification` |
| `SDK-D3038C3E3C1A33FA` | `RunSnapshot::state` | `fixtures/durable/records.json::$[0].expected.state` | `record_runsnapshot_state` |
| `SDK-7EA5C47259AF194F` | `RunSnapshot::attempts` | `fixtures/durable/records.json::$[0].expected.attempts` | `record_runsnapshot_attempts` |
| `SDK-9C36C65CC43DA63A` | `RunSnapshot::steps` | `fixtures/durable/records.json::$[0].expected.steps` | `record_runsnapshot_steps` |
| `SDK-E7A3560E935603A4` | `RunSnapshot::tool_calls` | `fixtures/durable/records.json::$[0].expected.tool_calls` | `record_runsnapshot_tool_calls` |
| `SDK-6367687A5F1C7A3A` | `RunSnapshot::approvals` | `fixtures/durable/records.json::$[0].expected.approvals` | `record_runsnapshot_approvals` |
| `SDK-71E060FB94BFE1AF` | `RunSnapshot::child_runs` | `fixtures/durable/records.json::$[0].expected.child_runs` | `record_runsnapshot_child_runs` |
| `SDK-1F12E4E9A9058F29` | `RunSnapshot::effects` | `fixtures/durable/records.json::$[0].expected.effects` | `record_runsnapshot_effects` |
| `SDK-17FEFBC09F24B512` | `RunSnapshot::cancellation` | `fixtures/durable/records.json::$[0].expected.cancellation` | `record_runsnapshot_cancellation` |
| `SDK-03DAFC2AD6ABEE9A` | `RunSnapshot::cumulative_budget` | `fixtures/durable/records.json::$[0].expected.cumulative_budget` | `record_runsnapshot_cumulative_budget` |
| `SDK-30BB3C6BB416E79A` | `RunSnapshot::created_at` | `fixtures/durable/records.json::$[0].expected.created_at` | `record_runsnapshot_created_at` |
| `SDK-6F26B3B3A2E43B0C` | `RunSnapshot::updated_at` | `fixtures/durable/records.json::$[0].expected.updated_at` | `record_runsnapshot_updated_at` |
| `SDK-95F6EE29046D00A2` | `RunSnapshot::retain_until` | `fixtures/durable/records.json::$[0].expected.retain_until` | `record_runsnapshot_retain_until` |
| `SDK-15E27CB550C0E5F9` | `Effect::id` | `fixtures/durable/records.json::$[4].expected.id` | `record_effect_id` |
| `SDK-E86BF033A46494A7` | `Effect::classification` | `fixtures/durable/records.json::$[4].expected.classification` | `record_effect_classification` |
| `SDK-D82C9C424FD1C733` | `Effect::data_classification` | `fixtures/durable/records.json::$[4].expected.data_classification` | `record_effect_data_classification` |
| `SDK-8CA8C120FC5F189F` | `Effect::state` | `fixtures/durable/records.json::$[4].expected.state` | `record_effect_state` |
| `SDK-57C67232BE2D561E` | `Effect::idempotency_key` | `fixtures/durable/records.json::$[4].expected.idempotency_key` | `record_effect_idempotency_key` |
| `SDK-16F67737E7980A75` | `Effect::prepared_at` | `fixtures/durable/records.json::$[4].expected.prepared_at` | `record_effect_prepared_at` |
| `SDK-C274C87AE85F5C26` | `Effect::updated_at` | `fixtures/durable/records.json::$[4].expected.updated_at` | `record_effect_updated_at` |
| `SDK-B069524723C7F23B` | `Effect::outcome` | `fixtures/durable/records.json::$[4].expected.outcome` | `record_effect_outcome` |
| `SDK-683DB8C064B3A179` | `Event::id` | `fixtures/durable/records.json::$[8].expected.id` | `record_event_id` |
| `SDK-32AD63943369DA92` | `Event::tenant_id` | `fixtures/durable/records.json::$[8].expected.tenant_id` | `record_event_tenant_id` |
| `SDK-1D64EA475C2088E0` | `Event::run_id` | `fixtures/durable/records.json::$[8].expected.run_id` | `record_event_run_id` |
| `SDK-FA957101273928BD` | `Event::sequence` | `fixtures/durable/records.json::$[8].expected.sequence` | `record_event_sequence` |
| `SDK-0B9666A920421293` | `Event::at` | `fixtures/durable/records.json::$[8].expected.at` | `record_event_at` |
| `SDK-70C0865DA541A4F9` | `Event::event_type` | `fixtures/durable/records.json::$[8].expected.type` | `record_event_event_type` |
| `SDK-2B23190A6636E691` | `Event::classification` | `fixtures/durable/records.json::$[8].expected.classification` | `record_event_classification` |
| `SDK-AA41F1A49BBACA5D` | `Event::payload` | `fixtures/durable/records.json::$[8].expected.payload` | `record_event_payload` |
| `SDK-4DA8312E87E351D5` | `ToolCall::id` | `fixtures/durable/records.json::$[12].expected.id` | `record_toolcall_id` |
| `SDK-037A24E10248EC72` | `ToolCall::name` | `fixtures/durable/records.json::$[12].expected.name` | `record_toolcall_name` |
| `SDK-457BD989E617EAC4` | `ToolCall::status` | `fixtures/durable/records.json::$[12].expected.status` | `record_toolcall_status` |
| `SDK-3FE51B6137AAE4E2` | `ToolCall::classification` | `fixtures/durable/records.json::$[12].expected.classification` | `record_toolcall_classification` |
| `SDK-A604022262154FCA` | `ToolCall::input` | `fixtures/durable/records.json::$[12].expected.input` | `record_toolcall_input` |
| `SDK-2F09802BBBAACC91` | `ToolCall::output` | `fixtures/durable/records.json::$[12].expected.output` | `record_toolcall_output` |
| `SDK-7CEAAF83D1F1E6F8` | `ToolCall::started_at` | `fixtures/durable/records.json::$[12].expected.started_at` | `record_toolcall_started_at` |
| `SDK-F4DBF9C6D49E1733` | `ToolCall::ended_at` | `fixtures/durable/records.json::$[12].expected.ended_at` | `record_toolcall_ended_at` |
| `SDK-C15BCE4F7E23B417` | `Step::id` | `fixtures/durable/records.json::$[16].expected.id` | `record_step_id` |
| `SDK-9D62C34B1B692770` | `Step::kind` | `fixtures/durable/records.json::$[16].expected.kind` | `record_step_kind` |
| `SDK-A47AB64465E18C45` | `Step::status` | `fixtures/durable/records.json::$[16].expected.status` | `record_step_status` |
| `SDK-741C1CD701CC5D2C` | `Step::data` | `fixtures/durable/records.json::$[16].expected.data` | `record_step_data` |
| `SDK-9B7FB68E160A9F5F` | `Step::started_at` | `fixtures/durable/records.json::$[16].expected.started_at` | `record_step_started_at` |
| `SDK-F79DEAE6F3E03786` | `Step::ended_at` | `fixtures/durable/records.json::$[16].expected.ended_at` | `record_step_ended_at` |
| `SDK-3EF90FF8894CEAB5` | `Approval::id` | `fixtures/durable/records.json::$[19].expected.id` | `record_approval_id` |
| `SDK-23DF19B33E4E1CB3` | `Approval::status` | `fixtures/durable/records.json::$[19].expected.status` | `record_approval_status` |
| `SDK-F1F60073ED0B69FB` | `Approval::requested_at` | `fixtures/durable/records.json::$[19].expected.requested_at` | `record_approval_requested_at` |
| `SDK-205D7E6F7393DCF2` | `Approval::resolved_at` | `fixtures/durable/records.json::$[19].expected.resolved_at` | `record_approval_resolved_at` |
| `SDK-6E84C2F00724F334` | `Approval::resolved_by` | `fixtures/durable/records.json::$[19].expected.resolved_by` | `record_approval_resolved_by` |
| `SDK-657B776E74A89E5E` | `BudgetCounters::input_tokens` | `fixtures/durable/records.json::$[22].expected.input_tokens` | `record_budgetcounters_input_tokens` |
| `SDK-028415C38DC716A8` | `BudgetCounters::output_tokens` | `fixtures/durable/records.json::$[22].expected.output_tokens` | `record_budgetcounters_output_tokens` |
| `SDK-EEE6615006C14574` | `BudgetCounters::tool_calls` | `fixtures/durable/records.json::$[22].expected.tool_calls` | `record_budgetcounters_tool_calls` |
| `SDK-DA786A5D0ED9B94A` | `BudgetCounters::cost_micros` | `fixtures/durable/records.json::$[22].expected.cost_micros` | `record_budgetcounters_cost_micros` |
| `SDK-6D665218AF8726F8` | `BudgetCounters::wall_time_ms` | `fixtures/durable/records.json::$[22].expected.wall_time_ms` | `record_budgetcounters_wall_time_ms` |
| `SDK-5B494B8E9E252000` | `ChildRun::id` | `fixtures/durable/records.json::$[25].expected.id` | `record_childrun_id` |
| `SDK-85DC830CF4B32806` | `ChildRun::run_id` | `fixtures/durable/records.json::$[25].expected.run_id` | `record_childrun_run_id` |
| `SDK-FCA4493D7729A043` | `ChildRun::status` | `fixtures/durable/records.json::$[25].expected.status` | `record_childrun_status` |
| `SDK-4F5B72C354A8FDBD` | `ChildRun::started_at` | `fixtures/durable/records.json::$[25].expected.started_at` | `record_childrun_started_at` |
| `SDK-5B6B8BA974D0C9E0` | `ChildRun::ended_at` | `fixtures/durable/records.json::$[25].expected.ended_at` | `record_childrun_ended_at` |
| `SDK-149EBF34A80AD247` | `Lease::tenant_id` | `fixtures/durable/records.json::$[29].expected.tenant_id` | `record_lease_tenant_id` |
| `SDK-FC48FC9F96A0445B` | `Lease::run_id` | `fixtures/durable/records.json::$[29].expected.run_id` | `record_lease_run_id` |
| `SDK-9EEED5718B3104CB` | `Lease::owner` | `fixtures/durable/records.json::$[29].expected.owner` | `record_lease_owner` |
| `SDK-77A1B446F3B3E1A0` | `Lease::token` | `fixtures/durable/records.json::$[29].expected.token` | `record_lease_token` |
| `SDK-A254013842C94B2F` | `Lease::expires_at` | `fixtures/durable/records.json::$[29].expected.expires_at` | `record_lease_expires_at` |
| `SDK-837D5D398134C0D2` | `Attempt::id` | `fixtures/durable/records.json::$[32].expected.id` | `record_attempt_id` |
| `SDK-73EEEED9652F9E9B` | `Attempt::started_at` | `fixtures/durable/records.json::$[32].expected.started_at` | `record_attempt_started_at` |
| `SDK-BF0C8A6AC0E45436` | `Attempt::ended_at` | `fixtures/durable/records.json::$[32].expected.ended_at` | `record_attempt_ended_at` |
| `SDK-CC48F92241013FD7` | `Attempt::outcome` | `fixtures/durable/records.json::$[32].expected.outcome` | `record_attempt_outcome` |
| `SDK-05C3E48D57E82D82` | `Cancellation::requested_at` | `fixtures/durable/records.json::$[35].expected.requested_at` | `record_cancellation_requested_at` |
| `SDK-11977E8766A9A71A` | `Cancellation::requested_by` | `fixtures/durable/records.json::$[35].expected.requested_by` | `record_cancellation_requested_by` |
| `SDK-D69BCC311B785352` | `Cancellation::reason` | `fixtures/durable/records.json::$[35].expected.reason` | `record_cancellation_reason` |
| `SDK-42D5079DDD9FF282` | `Cancellation::acknowledged_at` | `fixtures/durable/records.json::$[35].expected.acknowledged_at` | `record_cancellation_acknowledged_at` |
| `SDK-36248A3E322F5EC3` | `RecoveryDecision::action` | `fixtures/durable/records.json::$[38].expected.action` | `record_recoverydecision_action` |
| `SDK-CA9B3DEE50D4707F` | `RecoveryDecision::automatic` | `fixtures/durable/records.json::$[38].expected.automatic` | `record_recoverydecision_automatic` |
| `SDK-891A7E668DD7AFD8` | `RecoveryDecision::idempotency_key` | `fixtures/durable/records.json::$[38].expected.idempotency_key` | `record_recoverydecision_idempotency_key` |
