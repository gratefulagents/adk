use adk_durable::*;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

#[derive(Deserialize)]
struct Case {
    record: String,
    case: String,
    input: Value,
    expected: Value,
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("../../../fixtures/durable/records.json")).unwrap()
}
fn assert_wire_eq(actual: Value, expected: Value, context: &str) {
    assert_eq!(actual, expected, "{context}");
}
macro_rules! field_proof {
    ($test:ident, $record:ident, $field:ident, $wire:literal, $id:literal) => {
        #[test]
        fn $test() {
            for case in cases()
                .into_iter()
                .filter(|c| c.record == stringify!($record))
            {
                let record: $record = serde_json::from_value(case.input).unwrap();
                let context = format!("{} {} {}", $id, stringify!($field), case.case);
                if case.expected.get($wire).is_some() {
                    let expected: $record = serde_json::from_value(case.expected.clone()).unwrap();
                    assert_eq!(record.$field, expected.$field, "{context}");
                } else {
                    assert_eq!(record.$field, $record::default().$field, "{context}");
                }
                // Compare the complete wire object too: omitted and explicit-null fields differ.
                assert_wire_eq(
                    serde_json::to_value(&record).unwrap(),
                    case.expected,
                    &context,
                );
            }
        }
    };
}
field_proof!(
    record_runsnapshot_schema_version,
    RunSnapshot,
    schema_version,
    "schema_version",
    "SDK-14B557B50B8753D8"
);
field_proof!(
    record_runsnapshot_tenant_id,
    RunSnapshot,
    tenant_id,
    "tenant_id",
    "SDK-8123E3663E5A6B3D"
);
field_proof!(
    record_runsnapshot_run_id,
    RunSnapshot,
    run_id,
    "run_id",
    "SDK-5EAE8CCA60B01B14"
);
field_proof!(
    record_runsnapshot_revision,
    RunSnapshot,
    revision,
    "revision",
    "SDK-129EE3451642C93B"
);
field_proof!(
    record_runsnapshot_event_sequence,
    RunSnapshot,
    event_sequence,
    "event_sequence",
    "SDK-D16611759C822DC6"
);
field_proof!(
    record_runsnapshot_status,
    RunSnapshot,
    status,
    "status",
    "SDK-559F356BBE207D70"
);
field_proof!(
    record_runsnapshot_classification,
    RunSnapshot,
    classification,
    "classification",
    "SDK-D35CEBC91498576B"
);
field_proof!(
    record_runsnapshot_state,
    RunSnapshot,
    state,
    "state",
    "SDK-D3038C3E3C1A33FA"
);
field_proof!(
    record_runsnapshot_attempts,
    RunSnapshot,
    attempts,
    "attempts",
    "SDK-7EA5C47259AF194F"
);
field_proof!(
    record_runsnapshot_steps,
    RunSnapshot,
    steps,
    "steps",
    "SDK-9C36C65CC43DA63A"
);
field_proof!(
    record_runsnapshot_tool_calls,
    RunSnapshot,
    tool_calls,
    "tool_calls",
    "SDK-E7A3560E935603A4"
);
field_proof!(
    record_runsnapshot_approvals,
    RunSnapshot,
    approvals,
    "approvals",
    "SDK-6367687A5F1C7A3A"
);
field_proof!(
    record_runsnapshot_child_runs,
    RunSnapshot,
    child_runs,
    "child_runs",
    "SDK-71E060FB94BFE1AF"
);
field_proof!(
    record_runsnapshot_effects,
    RunSnapshot,
    effects,
    "effects",
    "SDK-1F12E4E9A9058F29"
);
field_proof!(
    record_runsnapshot_cancellation,
    RunSnapshot,
    cancellation,
    "cancellation",
    "SDK-17FEFBC09F24B512"
);
field_proof!(
    record_runsnapshot_cumulative_budget,
    RunSnapshot,
    cumulative_budget,
    "cumulative_budget",
    "SDK-03DAFC2AD6ABEE9A"
);
field_proof!(
    record_runsnapshot_created_at,
    RunSnapshot,
    created_at,
    "created_at",
    "SDK-30BB3C6BB416E79A"
);
field_proof!(
    record_runsnapshot_updated_at,
    RunSnapshot,
    updated_at,
    "updated_at",
    "SDK-6F26B3B3A2E43B0C"
);
field_proof!(
    record_runsnapshot_retain_until,
    RunSnapshot,
    retain_until,
    "retain_until",
    "SDK-95F6EE29046D00A2"
);
field_proof!(record_effect_id, Effect, id, "id", "SDK-15E27CB550C0E5F9");
field_proof!(
    record_effect_classification,
    Effect,
    classification,
    "classification",
    "SDK-E86BF033A46494A7"
);
field_proof!(
    record_effect_data_classification,
    Effect,
    data_classification,
    "data_classification",
    "SDK-D82C9C424FD1C733"
);
field_proof!(
    record_effect_state,
    Effect,
    state,
    "state",
    "SDK-8CA8C120FC5F189F"
);
field_proof!(
    record_effect_idempotency_key,
    Effect,
    idempotency_key,
    "idempotency_key",
    "SDK-57C67232BE2D561E"
);
field_proof!(
    record_effect_prepared_at,
    Effect,
    prepared_at,
    "prepared_at",
    "SDK-16F67737E7980A75"
);
field_proof!(
    record_effect_updated_at,
    Effect,
    updated_at,
    "updated_at",
    "SDK-C274C87AE85F5C26"
);
field_proof!(
    record_effect_outcome,
    Effect,
    outcome,
    "outcome",
    "SDK-B069524723C7F23B"
);
field_proof!(record_event_id, Event, id, "id", "SDK-683DB8C064B3A179");
field_proof!(
    record_event_tenant_id,
    Event,
    tenant_id,
    "tenant_id",
    "SDK-32AD63943369DA92"
);
field_proof!(
    record_event_run_id,
    Event,
    run_id,
    "run_id",
    "SDK-1D64EA475C2088E0"
);
field_proof!(
    record_event_sequence,
    Event,
    sequence,
    "sequence",
    "SDK-FA957101273928BD"
);
field_proof!(record_event_at, Event, at, "at", "SDK-0B9666A920421293");
field_proof!(
    record_event_event_type,
    Event,
    event_type,
    "type",
    "SDK-70C0865DA541A4F9"
);
field_proof!(
    record_event_classification,
    Event,
    classification,
    "classification",
    "SDK-2B23190A6636E691"
);
field_proof!(
    record_event_payload,
    Event,
    payload,
    "payload",
    "SDK-AA41F1A49BBACA5D"
);
field_proof!(
    record_toolcall_id,
    ToolCall,
    id,
    "id",
    "SDK-4DA8312E87E351D5"
);
field_proof!(
    record_toolcall_name,
    ToolCall,
    name,
    "name",
    "SDK-037A24E10248EC72"
);
field_proof!(
    record_toolcall_status,
    ToolCall,
    status,
    "status",
    "SDK-457BD989E617EAC4"
);
field_proof!(
    record_toolcall_classification,
    ToolCall,
    classification,
    "classification",
    "SDK-3FE51B6137AAE4E2"
);
field_proof!(
    record_toolcall_input,
    ToolCall,
    input,
    "input",
    "SDK-A604022262154FCA"
);
field_proof!(
    record_toolcall_output,
    ToolCall,
    output,
    "output",
    "SDK-2F09802BBBAACC91"
);
field_proof!(
    record_toolcall_started_at,
    ToolCall,
    started_at,
    "started_at",
    "SDK-7CEAAF83D1F1E6F8"
);
field_proof!(
    record_toolcall_ended_at,
    ToolCall,
    ended_at,
    "ended_at",
    "SDK-F4DBF9C6D49E1733"
);
field_proof!(record_step_id, Step, id, "id", "SDK-C15BCE4F7E23B417");
field_proof!(record_step_kind, Step, kind, "kind", "SDK-9D62C34B1B692770");
field_proof!(
    record_step_status,
    Step,
    status,
    "status",
    "SDK-A47AB64465E18C45"
);
field_proof!(record_step_data, Step, data, "data", "SDK-741C1CD701CC5D2C");
field_proof!(
    record_step_started_at,
    Step,
    started_at,
    "started_at",
    "SDK-9B7FB68E160A9F5F"
);
field_proof!(
    record_step_ended_at,
    Step,
    ended_at,
    "ended_at",
    "SDK-F79DEAE6F3E03786"
);
field_proof!(
    record_approval_id,
    Approval,
    id,
    "id",
    "SDK-3EF90FF8894CEAB5"
);
field_proof!(
    record_approval_status,
    Approval,
    status,
    "status",
    "SDK-23DF19B33E4E1CB3"
);
field_proof!(
    record_approval_requested_at,
    Approval,
    requested_at,
    "requested_at",
    "SDK-F1F60073ED0B69FB"
);
field_proof!(
    record_approval_resolved_at,
    Approval,
    resolved_at,
    "resolved_at",
    "SDK-205D7E6F7393DCF2"
);
field_proof!(
    record_approval_resolved_by,
    Approval,
    resolved_by,
    "resolved_by",
    "SDK-6E84C2F00724F334"
);
field_proof!(
    record_budgetcounters_input_tokens,
    BudgetCounters,
    input_tokens,
    "input_tokens",
    "SDK-657B776E74A89E5E"
);
field_proof!(
    record_budgetcounters_output_tokens,
    BudgetCounters,
    output_tokens,
    "output_tokens",
    "SDK-028415C38DC716A8"
);
field_proof!(
    record_budgetcounters_tool_calls,
    BudgetCounters,
    tool_calls,
    "tool_calls",
    "SDK-EEE6615006C14574"
);
field_proof!(
    record_budgetcounters_cost_micros,
    BudgetCounters,
    cost_micros,
    "cost_micros",
    "SDK-DA786A5D0ED9B94A"
);
field_proof!(
    record_budgetcounters_wall_time_ms,
    BudgetCounters,
    wall_time_ms,
    "wall_time_ms",
    "SDK-6D665218AF8726F8"
);
field_proof!(
    record_childrun_id,
    ChildRun,
    id,
    "id",
    "SDK-5B494B8E9E252000"
);
field_proof!(
    record_childrun_run_id,
    ChildRun,
    run_id,
    "run_id",
    "SDK-85DC830CF4B32806"
);
field_proof!(
    record_childrun_status,
    ChildRun,
    status,
    "status",
    "SDK-FCA4493D7729A043"
);
field_proof!(
    record_childrun_started_at,
    ChildRun,
    started_at,
    "started_at",
    "SDK-4F5B72C354A8FDBD"
);
field_proof!(
    record_childrun_ended_at,
    ChildRun,
    ended_at,
    "ended_at",
    "SDK-5B6B8BA974D0C9E0"
);
field_proof!(
    record_lease_tenant_id,
    Lease,
    tenant_id,
    "tenant_id",
    "SDK-149EBF34A80AD247"
);
field_proof!(
    record_lease_run_id,
    Lease,
    run_id,
    "run_id",
    "SDK-FC48FC9F96A0445B"
);
field_proof!(
    record_lease_owner,
    Lease,
    owner,
    "owner",
    "SDK-9EEED5718B3104CB"
);
field_proof!(
    record_lease_token,
    Lease,
    token,
    "token",
    "SDK-77A1B446F3B3E1A0"
);
field_proof!(
    record_lease_expires_at,
    Lease,
    expires_at,
    "expires_at",
    "SDK-A254013842C94B2F"
);
field_proof!(record_attempt_id, Attempt, id, "id", "SDK-837D5D398134C0D2");
field_proof!(
    record_attempt_started_at,
    Attempt,
    started_at,
    "started_at",
    "SDK-73EEEED9652F9E9B"
);
field_proof!(
    record_attempt_ended_at,
    Attempt,
    ended_at,
    "ended_at",
    "SDK-BF0C8A6AC0E45436"
);
field_proof!(
    record_attempt_outcome,
    Attempt,
    outcome,
    "outcome",
    "SDK-CC48F92241013FD7"
);
field_proof!(
    record_cancellation_requested_at,
    Cancellation,
    requested_at,
    "requested_at",
    "SDK-05C3E48D57E82D82"
);
field_proof!(
    record_cancellation_requested_by,
    Cancellation,
    requested_by,
    "requested_by",
    "SDK-11977E8766A9A71A"
);
field_proof!(
    record_cancellation_reason,
    Cancellation,
    reason,
    "reason",
    "SDK-D69BCC311B785352"
);
field_proof!(
    record_cancellation_acknowledged_at,
    Cancellation,
    acknowledged_at,
    "acknowledged_at",
    "SDK-42D5079DDD9FF282"
);
field_proof!(
    record_recoverydecision_action,
    RecoveryDecision,
    action,
    "action",
    "SDK-36248A3E322F5EC3"
);
field_proof!(
    record_recoverydecision_automatic,
    RecoveryDecision,
    automatic,
    "automatic",
    "SDK-CA9B3DEE50D4707F"
);
field_proof!(
    record_recoverydecision_idempotency_key,
    RecoveryDecision,
    idempotency_key,
    "idempotency_key",
    "SDK-891A7E668DD7AFD8"
);

#[test]
fn go_decodes_rust_records_against_independent_expected_json() {
    if std::env::var("ADK_TEST_GO").as_deref() != Ok("1") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("records.json"),
        include_bytes!("../../../fixtures/durable/records.json"),
    )
    .unwrap();
    let encoded: Vec<Value> = cases()
        .into_iter()
        .map(|case| {
            macro_rules! encode {
                ($t:ty) => {
                    serde_json::to_value(serde_json::from_value::<$t>(case.input).unwrap()).unwrap()
                };
            }
            match case.record.as_str() {
                "RunSnapshot" => encode!(RunSnapshot),
                "Effect" => encode!(Effect),
                "Event" => encode!(Event),
                "ToolCall" => encode!(ToolCall),
                "Step" => encode!(Step),
                "Approval" => encode!(Approval),
                "BudgetCounters" => encode!(BudgetCounters),
                "ChildRun" => encode!(ChildRun),
                "Lease" => encode!(Lease),
                "Attempt" => encode!(Attempt),
                "Cancellation" => encode!(Cancellation),
                "RecoveryDecision" => encode!(RecoveryDecision),
                other => panic!("unknown record {other}"),
            }
        })
        .collect();
    fs::write(
        dir.path().join("rust-records.json"),
        serde_json::to_vec(&encoded).unwrap(),
    )
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("/usr/local/go/bin/go")
        .current_dir(root.join("repos/sdk"))
        .args([
            "run",
            "../../fixtures/durable/generate.go",
            "verify-records",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    for (record, pointer, replacement) in [
        ("Event", "/at", "2025-01-01T21:19:05.1Z"),
        ("RunSnapshot", "/created_at", "2025-01-01T21:19:05.1Z"),
        ("RunSnapshot", "/retain_until", "2025-01-01T21:19:05.1Z"),
        (
            "RunSnapshot",
            "/steps/0/started_at",
            "2025-01-01T21:19:05.1Z",
        ),
        (
            "RunSnapshot",
            "/cancellation/requested_at",
            "2025-01-01T21:19:05.1Z",
        ),
        ("Event", "/at", "2025-01-02T03:04:05.100+05:45"),
        ("Event", "/payload/at", "2025-01-02T01:04:05.123456789Z"),
        ("Step", "/status", "2025-01-02T01:04:05.123456789Z"),
    ] {
        let i = cases()
            .iter()
            .position(|c| c.record == record && c.case == "timestamp_01")
            .unwrap();
        let mut changed = encoded.clone();
        let field = changed[i].pointer_mut(pointer).unwrap();
        assert_ne!(field, &json!(replacement));
        *field = json!(replacement);
        fs::write(
            dir.path().join("rust-records.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        let output = Command::new("/usr/local/go/bin/go")
            .current_dir(root.join("repos/sdk"))
            .args([
                "run",
                "../../fixtures/durable/generate.go",
                "verify-records",
            ])
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "Go accepted changed {record}{pointer}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(&format!("{record}/timestamp_01")),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    println!(
        "Go rejected all 8 timestamp-string mutations (typed, nested, optional, fractional spelling, opaque)"
    );
}
#[test]
fn unknown_wire_strings_survive_v1_migration() {
    let actual =
        decode_document(include_bytes!("../../../fixtures/durable/v1-unknown.json")).unwrap();
    let expected: Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/durable/v1-unknown-migrated.json"
    ))
    .unwrap();
    assert_eq!(
        actual.snapshot.status,
        RunStatus::Unknown("Future / 世界 <v3>\nRunStatus".into())
    );
    assert_eq!(
        actual.events[0].classification,
        DataClassification::Unknown("Future / 世界 <v3>\nDataClassification".into())
    );
    assert_wire_eq(
        serde_json::from_slice(&encode_document(&actual).unwrap()).unwrap(),
        expected,
        "v1 unknown strings",
    );
}
#[test]
fn unknown_effect_values_never_authorize_recovery_or_state_transitions() {
    for unknown in ["", "Future / 世界 <v3>\n"] {
        for state in [
            EffectState::Prepared,
            EffectState::Dispatched,
            EffectState::Succeeded,
            EffectState::Failed,
            EffectState::OutcomeUnknown,
            EffectState::Unknown(unknown.into()),
        ] {
            let mut effect = Effect::new(
                &"run".into(),
                EffectClassification::Unknown(unknown.into()),
                zero_time().with_timezone(&Utc),
            );
            effect.state = state.clone();
            let decision = recover_effect(&effect);
            assert_eq!(decision.action, RecoveryAction::None);
            assert!(!decision.automatic);
            assert_eq!(decision.idempotency_key, effect.idempotency_key);
            let before = effect.clone();
            assert!(
                transition_effect(
                    &mut effect,
                    EffectState::Unknown(unknown.into()),
                    Utc::now()
                )
                .is_err()
            );
            assert_eq!(effect, before);
            for classification in [
                EffectClassification::Idempotent,
                EffectClassification::Deduplicated,
                EffectClassification::NonReplayable,
            ] {
                effect.classification = classification;
                effect.state = EffectState::Unknown(unknown.into());
                let before = effect.clone();
                assert_eq!(recover_effect(&effect).action, RecoveryAction::None);
                assert!(!recover_effect(&effect).automatic);
                assert!(transition_effect(&mut effect, state.clone(), Utc::now()).is_err());
                assert_eq!(effect, before);
            }
        }
    }
}
#[test]
fn large_integers_nanosecond_instants_and_explicit_null_remain_exact() {
    let case = cases()
        .into_iter()
        .find(|c| c.record == "RunSnapshot" && c.case == "populated")
        .unwrap();
    let snapshot: RunSnapshot = serde_json::from_value(case.input).unwrap();
    assert_eq!(snapshot.revision, 9007199254740993);
    assert_eq!(snapshot.event_sequence, 9007199254740993);
    assert_eq!(snapshot.cumulative_budget.input_tokens, 9007199254740993);
    assert_eq!(
        snapshot.state.as_ref().unwrap()["large"].as_u64(),
        Some(u64::MAX)
    );
    assert_eq!(snapshot.created_at.timestamp_subsec_nanos(), 123456789);
    assert_eq!(
        snapshot.attempts[0].ended_at.timestamp_subsec_nanos(),
        123456796
    );
    for case in cases().into_iter().filter(|c| c.record == "Event") {
        let event: Event = serde_json::from_value(case.input).unwrap();
        if case.case == "null" {
            assert_eq!(event.payload, Some(Value::Null));
        }
        if case.case == "omitted" {
            assert_eq!(event.payload, None);
            assert_eq!(event.at, zero_time());
        }
        if case.case == "populated" {
            assert_eq!(event.sequence, 9007199254740993);
            assert_eq!(event.payload.unwrap()["n"], json!(9007199254740993_u64));
        }
    }
}

#[test]
fn wire_comparison_rejects_changed_timestamp_strings() {
    let offset = json!("2025-01-02T03:04:05.123456789+02:00");
    let utc = json!("2025-01-02T01:04:05.123456789Z");
    for (actual, expected) in [
        (json!({"created_at": &offset}), json!({"created_at": &utc})),
        (
            json!({"retain_until": &offset}),
            json!({"retain_until": &utc}),
        ),
        (
            json!({"steps": [{"ended_at": &offset}]}),
            json!({"steps": [{"ended_at": &utc}]}),
        ),
        (
            json!({"payload": {"at": &offset}}),
            json!({"payload": {"at": &utc}}),
        ),
        (json!({"status": &offset}), json!({"status": &utc})),
        (
            json!({"at": "2025-01-02T01:04:05.1Z"}),
            json!({"at": "2025-01-02T01:04:05.100Z"}),
        ),
    ] {
        assert!(
            std::panic::catch_unwind(|| assert_wire_eq(actual, expected, "must differ")).is_err()
        );
    }
}
