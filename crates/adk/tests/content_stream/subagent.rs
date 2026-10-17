use super::*;
use adk::content_stream::{SubagentCompleted, SubagentStarted, SubagentTaskStatus};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../fixtures/content-stream/observations.json"
    ))
    .unwrap()
}
fn check(output: &Buffer, sub: &ContentSubscription, start: DateTime<Utc>, expected: &Value) {
    let mut delivered = drain(sub, start);
    delivered["events"] = digests(delivered["events"].take());
    assert_eq!(delivered, expected["subscriber"]);
    assert_eq!(digests(wire(output, start)), expected["wire"]);
}

#[test]
fn subagent_lifecycle_helpers_match_pinned_sdk() {
    let fixture = fixture();
    let texts = [
        String::new(),
        fixture["subagentWhitespace"].as_str().unwrap().into(),
        " \n😀\rinterior\t\ntext\x1c\x1f ".into(),
        "a".repeat(4096),
        "a".repeat(4097),
        "😀".repeat(4096),
        "😀".repeat(4097),
        format!("{}e\u{301}tail", "a".repeat(4092)),
        " \n😀\n".repeat(2000),
        "\u{feff} prefix\u{200b}\n😀 \u{feff}".into(),
    ];
    assert_eq!(
        texts.len(),
        fixture["subagentCases"].as_array().unwrap().len()
    );
    for (i, text) in texts.into_iter().enumerate() {
        let start: DateTime<Utc> = SystemTime::now().into();
        let output = Buffer::default();
        let root = ContentStream::new(output.clone());
        root.set_session(3);
        root.set_phase("workers");
        let child = root.child("task");
        child.set_step("work");
        let sub = child.subscribe(8);
        child.emit_subagent_completed(SubagentCompleted {
            status: "cold".into(),
            ..Default::default()
        });
        child.emit_subagent_started(SubagentStarted {
            tool_use_id: "launch".into(),
            description: text.clone(),
            subagent_type: "worker😀".into(),
            model: "model".into(),
            prompt: text.clone(),
            ..Default::default()
        });
        child.emit_subagent_completed(SubagentCompleted {
            status: "failed".into(),
            description: text.clone(),
            tool_count: i32::MIN,
            tokens: i64::MIN,
            duration_ms: i64::MAX,
            cost_usd: -0.125,
            cost_known: true,
            num_turns: i32::MAX,
            cache_read_tokens: -7,
            cache_create_tokens: i64::MAX,
            stop_reason: "stop\n".into(),
            result_text: text.clone(),
            ..Default::default()
        });
        child.emit_subagent_completed(SubagentCompleted {
            status: "other".into(),
            description: text.clone(),
            tool_count: i32::MAX,
            tokens: i64::MAX,
            duration_ms: i64::MIN,
            cost_usd: 0.375,
            num_turns: i32::MIN,
            stop_reason: "other".into(),
            result_text: text,
            ..Default::default()
        });
        check(&output, &sub, start, &fixture["subagentCases"][i]);
    }
}

#[test]
fn subagent_status_helpers_match_pinned_sdk() {
    let fixture = fixture();
    let statuses = [
        "",
        "pending",
        "waiting",
        "running",
        "reconciling",
        "completed",
        "failed",
        "cancelled",
        "canceled",
        "Completed",
        "unknown",
    ];
    let durations = [-1, -999999, -1000001, i64::MIN, i64::MAX, 0, 1000001];
    assert_eq!(
        statuses.len(),
        fixture["subagentStatusCases"].as_array().unwrap().len()
    );
    for (i, status) in statuses.into_iter().enumerate() {
        let start: DateTime<Utc> = SystemTime::now().into();
        let output = Buffer::default();
        let root = ContentStream::new(output.clone());
        root.set_session(3);
        root.set_phase("workers");
        let child = root.child("task");
        let sub = child.subscribe(8);
        child.emit_subagent_started(SubagentStarted {
            subagent_type: "prior".into(),
            ..Default::default()
        });
        let mut task = SubagentTaskStatus {
            status: status.into(),
            agent_name: if i % 2 != 0 {
                "agent".into()
            } else {
                String::new()
            },
            task_message: format!(" \n{}\n ", "😀".repeat(4097)),
            duration_nanos: durations[i % durations.len()],
            result: if status == "failed" {
                String::new()
            } else {
                format!(" result\n{}", "r".repeat(4097))
            },
            tool_count: -7,
            tokens: -11,
            depends_on: if i == 0 {
                vec![]
            } else {
                vec!["first".into(), String::new(), "first".into()]
            },
            waiting_on: if i == 0 { vec![] } else { vec!["first".into()] },
            messages_received: -3,
            last_parent_message: "parent\n".repeat(1000),
            current_step: "step".into(),
            last_tool: "tool".into(),
            files_written: i64::MAX,
            ..Default::default()
        };
        child.emit_subagent_task_status(task.clone());
        task.message = " \n".into();
        child.emit_subagent_task_status(task);
        check(&output, &sub, start, &fixture["subagentStatusCases"][i]);
    }
}

#[test]
fn subagent_helpers_preserve_owned_progress_and_nonfinite_delivery() {
    use adk::codec::line_writer::{LineEvent, LineWriter};
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let root = ContentStream::new(LineWriter::new(move |event| {
        sink.lock().unwrap().push(event)
    }));
    let child = root.child("task");
    let sub = child.subscribe(8);
    child.emit_subagent_started(SubagentStarted {
        subagent_type: "worker".into(),
        prompt: " \nobjective\n ".into(),
        ..Default::default()
    });
    child.emit_subagent_task_status(SubagentTaskStatus {
        status: "failed".into(),
        depends_on: vec!["dep".into(), "dep".into()],
        duration_nanos: -1000001,
        result: " résumé\n 完了 ".into(),
        ..Default::default()
    });
    child.emit_subagent_completed(SubagentCompleted {
        status: "failed".into(),
        cost_usd: f64::NAN,
        ..Default::default()
    });
    assert_eq!(
        sub.receiver.try_recv().unwrap().subagent_prompt,
        "objective"
    );
    let terminal = sub.receiver.try_recv().unwrap();
    assert_eq!(terminal.subagent_stop_reason, "failed");
    assert_eq!(terminal.subagent_result_text, "résumé  完了");
    assert_eq!(terminal.subagent_duration_ms, -1);
    assert!(sub.receiver.try_recv().unwrap().subagent_cost_usd.is_nan());
    let captured = events.lock().unwrap();
    let [LineEvent::Subagent(started), LineEvent::Subagent(failed)] = captured.as_slice() else {
        panic!("expected two serializable progress events")
    };
    assert_eq!(started.task_id, "task");
    assert_eq!(started.agent_name, "worker");
    assert_eq!(failed.agent_name, "worker");
    assert_eq!(failed.depends_on, Some(vec!["dep".into(), "dep".into()]));
    assert_eq!(failed.result_text, "résumé  完了");
}
