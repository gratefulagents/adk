use adk_project_state::{Event, Memory, SessionSummary, Task, TaskComment};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

fn fixtures() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/project-state/records.json")).unwrap()
}

fn encode<T: DeserializeOwned + Serialize>(input: &Value) -> Result<Value, serde_json::Error> {
    serde_json::to_value(serde_json::from_value::<T>(input.clone())?)
}

fn compare<T: DeserializeOwned + Serialize>(name: &str) {
    let fixtures = fixtures();
    let mut failures = Vec::new();
    let mut cases = vec![
        "nondefault",
        "omitted",
        "null",
        "empty",
        "offset",
        "offset_negative",
        "offset_integral",
        "offset_zero",
    ];
    if matches!(name, "Task" | "Memory") {
        cases.extend(["metadata_null", "metadata_empty", "metadata_false"]);
    }
    if name == "Task" {
        cases.push("wide_priority");
    }
    for case in cases {
        let proof = fixtures[name]
            .get(case)
            .expect("required independent proof case");
        let input = proof.get("input").expect("required proof input");
        let expected = proof.get("expected").expect("required Go expected output");
        match encode::<T>(input) {
            Ok(actual) if actual == *expected => {}
            actual => failures.push(format!("{name}.{case}: {actual:?}; Go expected {expected}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn go_event_record_decode_encode_matches_go() {
    compare::<Event>("Event");
}
#[test]
fn go_task_record_decode_encode_matches_go() {
    compare::<Task>("Task");
}
#[test]
fn go_task_comment_record_decode_encode_matches_go() {
    compare::<TaskComment>("TaskComment");
}
#[test]
fn go_memory_record_decode_encode_matches_go() {
    compare::<Memory>("Memory");
}
#[test]
fn go_session_summary_record_decode_encode_matches_go() {
    compare::<SessionSummary>("SessionSummary");
}

#[test]
fn go_baseline_retains_task_ids_json_keys() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../fixtures/project-state/expected.json"
    ))
    .unwrap();
    let memory = expected["memories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|memory| memory["id"] == "mem_000000000001")
        .unwrap();
    assert_eq!(memory["task_ids"], serde_json::json!(["task_000000000001"]));
    assert_eq!(
        expected["sessions"][0]["task_ids"],
        serde_json::json!(["task_000000000001", "task_000000000002"])
    );
    for line in include_str!("../../../fixtures/project-state/baseline/events.jsonl").lines() {
        let event: Event = serde_json::from_str(line).unwrap();
        if event.event_type == "memory.upserted" && event.payload["id"] == "mem_000000000001" {
            let memory: Memory = serde_json::from_value(event.payload).unwrap();
            assert_eq!(memory.task_ids, ["task_000000000001"]);
        } else if event.event_type == "session.summary_saved" {
            let session: SessionSummary = serde_json::from_value(event.payload).unwrap();
            assert_eq!(session.task_ids, ["task_000000000001", "task_000000000002"]);
        }
    }
}

#[test]
fn replay_preserves_fixed_offsets_in_task_transition_payloads() {
    let fixtures = fixtures();
    let task = &fixtures["Task"]["offset"]["input"];
    let at = &fixtures["Task"]["offset_negative"]["expected"]["updated_at"];
    let mut events = vec![
        serde_json::from_value::<Event>(serde_json::json!({
            "type": "task.created", "payload": task
        }))
        .unwrap(),
    ];
    for kind in [
        "task.claimed",
        "task.dependency_added",
        "task.dependency_removed",
        "task.closed",
    ] {
        events.push(
            serde_json::from_value(serde_json::json!({
                "type": kind,
                "payload": {"id": task["id"], "at": at, "depends_on": "before", "reason": "Done"}
            }))
            .unwrap(),
        );
        let state = adk_project_state::State::replay(&events).unwrap();
        let actual = serde_json::to_value(&state.tasks["task-proof"]).unwrap();
        assert_eq!(actual["created_at"], task["created_at"]);
        assert_eq!(actual["updated_at"], *at);
        if kind == "task.closed" {
            assert_eq!(actual["closed_at"], *at);
            assert_eq!(
                actual["comments"].as_array().unwrap().last().unwrap()["created_at"],
                *at
            );
        }
    }
}
