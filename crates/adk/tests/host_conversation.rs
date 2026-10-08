#![cfg(feature = "host")]
use adk::{core::*, host::*};
use adk_codec::dto::ImageAttachment;
use std::collections::BTreeSet;

fn message(text: &str, role: Role) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn user(id: i64, content: &str, mode: &str) -> UserMessage {
    UserMessage {
        id,
        content: content.into(),
        mode: mode.into(),
        ..Default::default()
    }
}
fn image() -> ImageAttachment {
    ImageAttachment {
        media_type: "application/pdf".into(),
        data: "AP8=".into(),
        detail: "high".into(),
    }
}
fn tool(name: &str) -> RunItem {
    RunItem::ToolCall {
        call: ToolCall {
            raw_arguments: None,
            id: String::new(),
            name: name.into(),
            arguments: serde_json::json!({}),
        },
    }
}
fn output(text: &str, is_error: bool) -> RunItem {
    RunItem::ToolResult {
        call_id: String::new(),
        output: ToolOutput {
            content: vec![Content::Text { text: text.into() }],
            is_error,
            should_pause: false,
        },
    }
}

#[test]
fn truncation_counts_runes_and_only_replaces_newlines() {
    assert_eq!(
        truncate_context_text(" \u{2003}猫\n🦀é\r\t end \u{a0}", 4),
        "猫 🦀é..."
    );
    assert_eq!(truncate_context_text(" \n a\r\n\t b \n", 0), "a\r \t b");
    assert_eq!(truncate_context_text("é🦀", 2), "é🦀");
    assert_eq!(truncate_context_text(" a\n\nb ", -1), "a  b");
    assert_eq!(truncate_context_text("\n\t", 1), "");
}

#[test]
fn working_context_keeps_raw_mode_and_checks_raw_equality() {
    assert_eq!(WorkingState::default().context(), "");
    let state = WorkingState {
        goal: " goal ".into(),
        current_mode: " mode\nraw ".into(),
        current_step: "\t".into(),
        last_user_message: "goal".into(),
        last_assistant_summary: "a\nb".into(),
        recent_turn_summaries: ["omitted", "one", "", " two\nlines ", "three"]
            .map(String::from)
            .to_vec(),
        ..Default::default()
    };
    let expected = "## Durable Working State\nCurrent objective: goal\nMode:  mode\nraw \nCurrent step: \nLatest user direction: goal\nLatest assistant summary: a b\nRecent progress:\n- one\n- \n- two lines\n- three";
    assert_eq!(build_working_state_context(&state), expected);
    assert_eq!(state.context(), expected);
    let equal = WorkingState {
        goal: "same".into(),
        last_user_message: "same".into(),
        ..Default::default()
    };
    assert_eq!(
        equal.context(),
        "## Durable Working State\nCurrent objective: same"
    );
}

#[test]
fn goal_recognizes_only_exact_approval_forms() {
    for reply in [
        "",
        " \n",
        " APPROVE ",
        "deny",
        "request changes",
        "request_changes",
        "Approve: yes",
        "DENY:no",
        "request changes: x",
        "request_changes:",
    ] {
        assert_eq!(
            derive_working_state_goal(reply, " effective "),
            "effective",
            "{reply:?}"
        );
    }
    for reply in [
        "approve now",
        "approve : yes",
        "request-changes",
        "ordinary",
    ] {
        assert_eq!(derive_working_state_goal(reply, "effective"), reply);
    }
    assert_eq!(derive_working_state_goal(" APPROVE ", " \n"), "APPROVE");
}

#[test]
fn tail_filters_before_limiting_without_sorting_and_preserves_agent_labels() {
    let messages: Vec<_> = [
        (3, "assistant", " old "),
        (9, "user", "current"),
        (8, "system", " system\ntext "),
        (7, "Assistant", " \n"),
        (6, "assistant", "answer"),
        (4, "tool", "unknown"),
    ]
    .into_iter()
    .map(|(id, role, content)| ConversationMessage {
        id,
        role: role.into(),
        content: content.into(),
        images: if id == 7 { vec![image()] } else { vec![] },
    })
    .collect();
    let state = WorkingState {
        history_floor_message_id: 3,
        ..Default::default()
    };
    let batch = build_conversation_tail(&messages, &state, 9, 0);
    assert_eq!(batch.items.len(), 4);
    assert_eq!(batch.items[0], message("system text", Role::Assistant));
    assert_eq!(
        batch.provenance,
        vec![
            ItemProvenance::Agent {
                name: "system-summary".into()
            },
            ItemProvenance::Unattributed,
            ItemProvenance::Agent {
                name: "assistant-summary".into()
            },
            ItemProvenance::Unattributed
        ]
    );
    let RunItem::Message {
        message: image_message,
    } = &batch.items[1]
    else {
        panic!()
    };
    assert_eq!(image_message.role, Role::User);
    assert_eq!(
        image_message.content,
        vec![
            Content::Text {
                text: String::new()
            },
            Content::Attachment {
                media_type: image().media_type,
                data: image().data,
                detail: image().detail
            }
        ]
    );
    assert_eq!(
        build_conversation_tail(&messages, &state, 9, 2).items,
        batch.items[2..]
    );
    let many: Vec<_> = (1..=10)
        .map(|id| ConversationMessage {
            id,
            content: "x".into(),
            ..Default::default()
        })
        .collect();
    assert_eq!(
        build_conversation_tail(&many, &WorkingState::default(), 0, -1)
            .items
            .len(),
        8
    );
    let signed = vec![ConversationMessage {
        id: -1,
        content: "negative".into(),
        ..Default::default()
    }];
    assert_eq!(
        build_conversation_tail(
            &signed,
            &WorkingState {
                history_floor_message_id: -2,
                ..Default::default()
            },
            -1,
            1
        )
        .items
        .len(),
        1
    );
    assert!(batch.markers.is_empty());
}

#[test]
fn summaries_use_provenance_not_roles_and_deduplicate_after_truncation() {
    let mut batch = RunBatch {
        items: vec![
            message("unattributed assistant", Role::Assistant),
            message("unknown assistant", Role::Assistant),
            message("  answer\ntext ", Role::User),
            message("answer text", Role::Assistant),
            message("second", Role::System),
            message("third", Role::Assistant),
            output("ok", false),
            output(" bad\nnews ", true),
            output("bad news", true),
            output("worse", true),
            output("ignored", true),
            tool("z"),
            tool("b"),
            tool("z"),
            tool("a"),
        ],
        ..Default::default()
    };
    batch.provenance = batch
        .items
        .iter()
        .map(|_| ItemProvenance::Agent {
            name: "actual".into(),
        })
        .collect();
    batch.provenance[0] = ItemProvenance::Unattributed;
    batch.provenance[1] = ItemProvenance::Unknown;
    assert_eq!(
        build_assistant_turn_summary(&batch),
        "answer text\nsecond\nTools: z x 2, a x 1, b x 1\nIssues: bad news | worse"
    );
    batch.provenance.clear();
    assert_eq!(
        build_assistant_turn_summary(&batch),
        "Tools: z x 2, a x 1, b x 1\nKey results: ok\nIssues: bad news | worse"
    );
    assert_eq!(
        summarize_turn_tool_calls(&batch.items, 2),
        vec!["z x 2", "a x 1"]
    );
    assert_eq!(
        summarize_turn_tool_calls(&batch.items, -1),
        vec!["z x 2", "a x 1", "b x 1"]
    );
    assert!(summarize_turn_tool_calls(&[], 0).is_empty());
    let long = "猫".repeat(221);
    let batch = RunBatch {
        items: vec![
            message(&long, Role::Assistant),
            message(&(long + "other"), Role::Assistant),
            message("next", Role::Assistant),
        ],
        provenance: vec![ItemProvenance::Agent { name: "a".into() }; 3],
        ..Default::default()
    };
    assert_eq!(
        build_assistant_turn_summary(&batch),
        format!("{}...\nnext", "猫".repeat(220))
    );
}

#[test]
fn selectors_prioritize_immediate_but_never_skip_pending_queue() {
    let mut messages = vec![
        user(-5, " ", ""),
        user(-4, "consumed", "immediate"),
        user(9, " queued\nraw ", "enqueue"),
        user(3, " first\nnow ", "immediate"),
        user(3, "duplicate", "immediate"),
        user(12, "", "immediate"),
        user(14, "", "immediate"),
    ];
    messages[5].images.push(image());
    let mut consumed = BTreeSet::from([-4]);
    let (selected, cursor, immediate) = select_next_user_message(&messages, &consumed);
    assert!(std::ptr::eq(selected.unwrap(), &messages[3]));
    assert_eq!(selected.unwrap().content, " first\nnow ");
    assert_eq!((cursor, immediate), (-4, true));
    let (batch, cursor) = collect_immediate_run_items(&messages, &mut consumed);
    assert_eq!(cursor, -4);
    assert_eq!(batch.items.len(), 2);
    assert_eq!(batch.items[0], message("first\nnow", Role::User));
    assert_eq!(batch.provenance, vec![ItemProvenance::Unattributed; 2]);
    assert_eq!(consumed, BTreeSet::from([-4, 3, 12]));
    let (selected, cursor, immediate) = select_next_user_message(&messages, &consumed);
    assert_eq!(selected.unwrap().id, 9);
    assert_eq!((cursor, immediate), (-4, false));
    assert!(
        collect_immediate_run_items(&messages, &mut consumed)
            .0
            .items
            .is_empty()
    );
    assert_eq!(
        collect_immediate_run_items(&messages[3..], &mut consumed).1,
        14
    );
    assert_eq!(select_next_user_message(&messages[3..], &consumed).1, 14);
    assert!(
        select_next_user_message(&messages[3..], &consumed)
            .0
            .is_none()
    );
    assert_eq!(collect_immediate_run_items(&[], &mut consumed).1, 0);
    let pending = vec![user(1, "pending", "Immediate"), user(2, "now", "immediate")];
    assert_eq!(
        collect_immediate_run_items(&pending, &mut BTreeSet::new()).1,
        0
    );
    assert_eq!(USER_MESSAGE_MODE_ENQUEUE, "enqueue");
}

mod oracle {
    use super::*;
    use adk_codec::{dto, request_native::snapshot_native_items};
    use serde_json::{Value, json};

    fn array(value: &Value) -> &[Value] {
        if value.is_null() {
            &[]
        } else {
            value.as_array().unwrap()
        }
    }

    fn images(value: &Value) -> Vec<ImageAttachment> {
        serde_json::from_value::<Option<Vec<ImageAttachment>>>(value.clone())
            .unwrap()
            .unwrap_or_default()
    }

    fn state(value: &Value) -> WorkingState {
        WorkingState {
            goal: value["Goal"].as_str().unwrap().into(),
            current_mode: value["CurrentMode"].as_str().unwrap().into(),
            current_step: value["CurrentStep"].as_str().unwrap().into(),
            last_user_message: value["LastUserMessage"].as_str().unwrap().into(),
            last_assistant_summary: value["LastAssistantSummary"].as_str().unwrap().into(),
            recent_turn_summaries: array(&value["RecentTurnSummaries"])
                .iter()
                .map(|v| v.as_str().unwrap().into())
                .collect(),
            history_floor_message_id: value["HistoryFloorMessageID"].as_i64().unwrap(),
            last_response_id: value["LastResponseID"].as_str().unwrap().into(),
            data: value["Data"].as_object().cloned().unwrap_or_default(),
        }
    }

    fn queue_message(value: &Value) -> UserMessage {
        UserMessage {
            id: value["ID"].as_i64().unwrap(),
            content: value["Content"].as_str().unwrap().into(),
            mode: value["Mode"].as_str().unwrap().into(),
            created_at: Some(
                value["CreatedAt"]
                    .as_str()
                    .unwrap()
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .unwrap()
                    .into(),
            ),
            images: images(&value["Images"]),
        }
    }

    fn restore(value: &Value) -> RunBatch {
        let mut batch = RunBatch::default();
        for value in array(value) {
            let snapshot: dto::RunItemSnapshot = serde_json::from_value(value.clone()).unwrap();
            let wire = dto::RunItem {
                agent: if snapshot.agent_name.is_empty() {
                    None
                } else {
                    Some(dto::AgentRef {
                        name: snapshot.agent_name.clone(),
                    })
                },
                ..match snapshot.kind {
                    dto::SnapshotType::Message => dto::RunItem {
                        message: Some(dto::MessageOutput {
                            text: snapshot.message_text,
                            phase: snapshot.message_phase,
                            images: snapshot.message_images,
                        }),
                        ..Default::default()
                    },
                    dto::SnapshotType::ToolCall => {
                        let call = snapshot.tool_call.unwrap();
                        dto::RunItem {
                            kind: dto::RunItemType(1),
                            tool_call: Some(dto::ToolCallData {
                                id: call.id,
                                name: call.name,
                                input: call.input,
                            }),
                            ..Default::default()
                        }
                    }
                    dto::SnapshotType::ToolOutput => dto::RunItem {
                        kind: dto::RunItemType(2),
                        tool_output: snapshot.tool_output,
                        ..Default::default()
                    },
                    dto::SnapshotType::Reasoning => dto::RunItem {
                        kind: dto::RunItemType(5),
                        reasoning: Some(
                            serde_json::from_value(value["reasoning"].clone()).unwrap(),
                        ),
                        ..Default::default()
                    },
                    other => panic!("unexpected oracle input {other:?}"),
                }
            };
            batch.provenance.push(match &wire.agent {
                Some(agent) => ItemProvenance::Agent {
                    name: agent.name.clone(),
                },
                None => ItemProvenance::Unattributed,
            });
            batch
                .items
                .push(adk_codec::approval::decode_item(&wire).unwrap());
        }
        batch
    }

    fn assert_items(batch: &RunBatch, expected: &Value, name: &str) {
        assert!(batch.markers.is_empty());
        assert_eq!(
            batch.items.len() as u64,
            expected["count"].as_u64().unwrap(),
            "{name}"
        );
        let actual = snapshot_native_items(&batch.items, &batch.provenance).unwrap();
        let expected: Vec<dto::RunItemSnapshot> = array(&expected["items"])
            .iter()
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .collect();
        assert_eq!(actual, expected, "{name}");
    }

    #[test]
    fn all_eight_helpers_match_independent_sdk_inputs_and_outputs() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../fixtures/host-loop/sdk-conversation.json"
        ))
        .unwrap();
        assert_eq!(fixture["schema_version"], 1);
        for operation in [
            "build_conversation_tail",
            "build_working_state_context",
            "derive_working_state_goal",
            "truncate_context_text",
            "build_assistant_turn_summary",
            "summarize_turn_tool_calls",
            "select_next_user_message",
            "collect_immediate_run_items",
        ] {
            let cases = fixture[operation].as_array().unwrap();
            assert!(!cases.is_empty(), "{operation}");
            for case in cases {
                let name = format!("{operation}: {}", case["name"]);
                let input = &case["input"];
                let expected = &case["output"];
                match operation {
                    "build_conversation_tail" => {
                        let messages: Vec<_> = array(&input["messages"])
                            .iter()
                            .map(|v| ConversationMessage {
                                id: v["ID"].as_i64().unwrap(),
                                role: v["Role"].as_str().unwrap().into(),
                                content: v["Content"].as_str().unwrap().into(),
                                images: images(&v["Images"]),
                            })
                            .collect();
                        let batch = build_conversation_tail(
                            &messages,
                            &state(&input["state"]),
                            input["exclude_message_id"].as_i64().unwrap(),
                            input["limit"].as_i64().unwrap(),
                        );
                        assert_items(&batch, expected, &name);
                    }
                    "build_working_state_context" => {
                        let state = state(&input["state"]);
                        assert_eq!(
                            build_working_state_context(&state),
                            expected["text"].as_str().unwrap(),
                            "{name}"
                        );
                        assert_eq!(
                            state.context(),
                            expected["text"].as_str().unwrap(),
                            "{name}"
                        );
                    }
                    "derive_working_state_goal" => assert_eq!(
                        derive_working_state_goal(
                            input["raw_reply"].as_str().unwrap(),
                            input["effective_prompt"].as_str().unwrap()
                        ),
                        expected["text"].as_str().unwrap(),
                        "{name}"
                    ),
                    "truncate_context_text" => assert_eq!(
                        truncate_context_text(
                            input["text"].as_str().unwrap(),
                            input["max"].as_i64().unwrap()
                        ),
                        expected["text"].as_str().unwrap(),
                        "{name}"
                    ),
                    "build_assistant_turn_summary" => assert_eq!(
                        build_assistant_turn_summary(&restore(&input["items"])),
                        expected["text"].as_str().unwrap(),
                        "{name}"
                    ),
                    "summarize_turn_tool_calls" => {
                        let actual = summarize_turn_tool_calls(
                            &restore(&input["items"]).items,
                            input["limit"].as_i64().unwrap(),
                        );
                        let expected: Vec<_> = array(&expected["summaries"])
                            .iter()
                            .map(|v| v.as_str().unwrap())
                            .collect();
                        assert_eq!(actual, expected, "{name}");
                    }
                    "select_next_user_message" | "collect_immediate_run_items" => {
                        let messages: Vec<_> = array(&input["messages"])
                            .iter()
                            .map(queue_message)
                            .collect();
                        let mut consumed: BTreeSet<_> = array(&input["consumed_immediate"])
                            .iter()
                            .map(|v| v.as_i64().unwrap())
                            .collect();
                        if operation == "select_next_user_message" {
                            let (selected, cursor, immediate) =
                                select_next_user_message(&messages, &consumed);
                            assert_eq!(
                                selected.is_some(),
                                expected["ok"].as_bool().unwrap(),
                                "{name}"
                            );
                            assert_eq!(cursor, expected["skip_cursor"].as_i64().unwrap(), "{name}");
                            assert_eq!(
                                immediate,
                                expected["immediate"].as_bool().unwrap(),
                                "{name}"
                            );
                            if let Some(selected) = selected {
                                let expected = queue_message(&expected["message"]);
                                assert_eq!(
                                    (
                                        selected.id,
                                        &selected.content,
                                        &selected.mode,
                                        selected.created_at,
                                        &selected.images
                                    ),
                                    (
                                        expected.id,
                                        &expected.content,
                                        &expected.mode,
                                        expected.created_at,
                                        &expected.images
                                    ),
                                    "{name}"
                                );
                            }
                        } else {
                            let (batch, cursor) =
                                collect_immediate_run_items(&messages, &mut consumed);
                            assert_items(&batch, expected, &name);
                            assert_eq!(cursor, expected["cursor"].as_i64().unwrap(), "{name}");
                        }
                        assert_eq!(
                            json!(consumed),
                            expected["consumed_immediate_after"],
                            "{name}"
                        );
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}
