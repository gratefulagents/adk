use super::*;
use adk_core::{Reasoning, ToolCall, ToolOutput};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../fixtures/llm-summary/observations.json"
    ))
    .unwrap()
}
fn recipe(value: &Value) -> String {
    format!(
        "{}{}{}",
        value["prefix"].as_str().unwrap_or_default(),
        value["repeat"]
            .as_str()
            .unwrap_or_default()
            .repeat(value["count"].as_u64().unwrap_or_default() as usize),
        value["suffix"].as_str().unwrap_or_default()
    )
}
fn items(value: &Value) -> Vec<HistoryItem> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|spec| {
            if spec["missing"] == true {
                return None;
            }
            let provenance = spec["agent"]
                .as_str()
                .map_or(ItemProvenance::Unattributed, |name| ItemProvenance::Agent {
                    name: name.into(),
                });
            let text = recipe(&spec["text"]);
            let item = match spec["kind"].as_str().unwrap() {
                "message" => RunItem::Message {
                    message: Message {
                        role: if spec["agent"].is_string() {
                            Role::Assistant
                        } else {
                            Role::User
                        },
                        content: vec![Content::Text { text }],
                    },
                },
                "reasoning" => RunItem::Reasoning {
                    reasoning: Reasoning {
                        text,
                        ..Default::default()
                    },
                },
                "toolCall" => RunItem::ToolCall {
                    call: ToolCall {
                        id: spec["id"].as_str().unwrap_or_default().into(),
                        name: spec["name"].as_str().unwrap_or_default().into(),
                        arguments: serde_json::from_str(&recipe(&spec["input"]))
                            .unwrap_or(Value::Null),
                    },
                },
                "toolOutput" => RunItem::ToolResult {
                    call_id: spec["id"].as_str().unwrap_or_default().into(),
                    output: ToolOutput {
                        content: vec![Content::Text { text }],
                        is_error: spec["isError"].as_bool().unwrap_or_default(),
                        should_pause: false,
                    },
                },
                _ => return None,
            };
            Some(HistoryItem::Native(item, provenance))
        })
        .collect()
}
fn observe(actual: &str, expected: &Value) {
    assert_eq!(actual.len() as u64, expected["bytes"].as_u64().unwrap());
    assert_eq!(
        actual.chars().count() as u64,
        expected["runes"].as_u64().unwrap()
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(actual.as_bytes())),
        expected["sha256"].as_str().unwrap()
    );
    if let Some(text) = expected["text"].as_str() {
        assert_eq!(actual, text);
    }
}
fn response(items: Vec<HistoryItem>) -> ModelResponse {
    ModelResponse {
        items: split_history(items).0,
        usage: Usage::default(),
        end_turn: None,
        response_id: None,
        metadata: Default::default(),
        raw: None,
        snapshot_raw: None,
        snapshot_projection: None,
    }
}
#[test]
fn pinned_llm_transcript_byte_limits_and_summary_message_filtering() {
    let fixture = fixture();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(INSTRUCTIONS, fixture["constants"]["instructions"]);
    for case in fixture["truncate"].as_array().unwrap() {
        observe(
            &truncate_transcript(
                case["input"].as_str().unwrap(),
                case["maxBytes"].as_u64().unwrap() as usize,
            ),
            &case["output"],
        );
    }
    let mut compared = 0;
    for case in fixture["flatten"].as_array().unwrap() {
        // Native tool arguments are JSON Values, not the SDK's arbitrary raw byte payloads.
        if case["items"].as_array().into_iter().flatten().any(|i| {
            i["kind"] == "toolCall"
                && i["missing"] != true
                && serde_json::from_str::<Value>(&recipe(&i["input"])).is_err()
        }) {
            continue;
        }
        let items = items(&case["items"]);
        observe(
            &flatten_transcript(
                items.iter(),
                case["maxBytes"].as_i64().unwrap().max(0) as usize,
            ),
            &case["output"],
        );
        compared += 1;
    }
    assert_eq!(compared, 39);
    for case in fixture["summaries"].as_array().unwrap() {
        if !case["modelError"].as_str().unwrap().is_empty()
            || case["nilModel"] == true
            || matches!(
                case["parentContext"].as_str(),
                Some("expired" | "cancelled")
            )
        {
            continue;
        }
        let source = items(&case["removed"]);
        let mut plan = plan_mixed(&source, LocalCompactionPolicy::default(), 0);
        plan.removed = (0..source.len()).collect();
        let request = plan.summary_request("oracle-model");
        if let Some(expected) = case["requests"].as_array().unwrap().first() {
            let request = request.unwrap();
            assert_eq!(request.instructions, expected["instructions"]);
            assert_eq!(
                serde_json::to_value(&request.settings).unwrap(),
                expected["settings"]
            );
            let RunItem::Message { message } = &request.input[0] else {
                panic!()
            };
            observe(
                &content_text(&message.content),
                &expected["input"][0]["message"],
            );
            assert_eq!(
                summary_body(&response(items(&case["responseItems"]))),
                case["body"].as_str().unwrap()
            );
        } else {
            assert!(request.is_none());
        }
    }
}

#[test]
fn pinned_llm_plans_exact_indices_scope_deferred_insertion_and_strict_shrink() {
    for case in fixture()["plans"].as_array().unwrap() {
        let source = items(&case["source"]);
        let cfg = &case["config"];
        let policy = LocalCompactionPolicy {
            trigger_tokens: cfg["TriggerTokens"].as_u64().unwrap(),
            target_tokens: cfg["TargetTokens"].as_u64().unwrap(),
            preserve_recent_items: cfg["PreserveRecentItems"].as_u64().unwrap() as usize,
            preserve_initial_user_messages: cfg["PreserveInitialUserMessages"].as_u64().unwrap()
                as usize,
            ..Default::default()
        };
        let mut plan = plan_mixed(&source, policy, 0);
        let protected = case["protectedIndices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i.as_u64().unwrap() as usize)
            .collect::<HashSet<_>>();
        let removed = case["removedIndices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i.as_u64().unwrap() as usize)
            .collect::<Vec<_>>();
        if case["usesPlanner"] == true {
            assert_eq!(plan.protected, protected);
            assert_eq!(plan.removed, removed);
            assert_eq!(
                plan.outcome.after_tokens,
                case["planAfter"].as_u64().unwrap()
            );
        } else {
            plan.protected = protected;
            plan.removed = removed;
        }
        assert_eq!(
            estimate_mixed_tokens(&source),
            case["sourceTokens"].as_u64().unwrap()
        );
        let before = plan.outcome.clone();
        let accepted = if case["modelError"] == true || case["nilModel"] == true {
            false
        } else {
            plan.apply_summary(&response(vec![summary_item(&recipe(
                &case["responseBody"],
            ))]))
        };
        assert_eq!(
            accepted,
            case["accepted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if accepted {
            let expected = case["items"].as_array().unwrap();
            assert_eq!(plan.outcome.history.len(), expected.len());
            for (item, expected) in plan.outcome.history.iter().zip(expected) {
                observe(
                    &item_text(&HistoryItem::Native(
                        item.clone(),
                        ItemProvenance::Unattributed,
                    )),
                    &expected["message"],
                );
            }
        } else {
            assert_eq!(plan.outcome, before);
        }
    }
}

#[test]
fn repeated_items_preserve_exact_positions_provenance_and_approval_boundaries() {
    use adk_codec::approval::ApprovalPhase;
    let duplicate = summary_item(&"same evidence ".repeat(1000));
    let call = ToolCall {
        id: "pending".into(),
        name: "write".into(),
        arguments: serde_json::json!({}),
    };
    let source = vec![
        duplicate.clone(),
        HistoryItem::Native(
            RunItem::Message {
                message: Message {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: "task".into(),
                    }],
                },
            },
            ItemProvenance::Unattributed,
        ),
        HistoryItem::Approval(ApprovalMarker::from_call(
            &call,
            ApprovalPhase::Pending,
            None,
        )),
        HistoryItem::Native(
            match duplicate {
                HistoryItem::Native(item, _) => item,
                _ => unreachable!(),
            },
            ItemProvenance::Agent {
                name: "kept-author".into(),
            },
        ),
        HistoryItem::Native(
            RunItem::ToolCall { call },
            ItemProvenance::Agent {
                name: "kept-author".into(),
            },
        ),
    ];
    let mut plan = plan_mixed(&source, LocalCompactionPolicy::default(), 0);
    plan.protected = HashSet::from([1, 2, 3, 4]);
    plan.removed = vec![0];
    assert!(plan.apply_summary(&response(vec![summary_item("sentinel")])));
    assert_eq!(
        plan.outcome.history_provenance,
        vec![
            ItemProvenance::Unattributed,
            ItemProvenance::Agent {
                name: "context-summary".into()
            },
            ItemProvenance::Agent {
                name: "kept-author".into()
            },
            ItemProvenance::Agent {
                name: "kept-author".into()
            }
        ]
    );
    assert_eq!(plan.outcome.markers[0].before_item, 2);
    assert_eq!(plan.outcome.markers[0].marker.phase, ApprovalPhase::Pending);
    assert_eq!(
        item_text(&source[3]),
        item_text(&HistoryItem::Native(
            plan.outcome.history[2].clone(),
            ItemProvenance::Unattributed
        ))
    );
}

#[test]
fn marker_text_cannot_spoof_summary_authorship_or_retention() {
    for provenance in [
        ItemProvenance::Unattributed,
        ItemProvenance::Unknown,
        ItemProvenance::Agent {
            name: "other".into(),
        },
        ItemProvenance::Agent {
            name: "context-summary".into(),
        },
    ] {
        let text = format!("{SUMMARY_MARKER}\n{}", "evidence ".repeat(2000));
        let item = HistoryItem::Native(
            RunItem::Message {
                message: Message {
                    role: Role::Assistant,
                    content: vec![Content::Text { text }],
                },
            },
            provenance.clone(),
        );
        let transcript = flatten_transcript(std::iter::once(&item), 240_000);
        let known_summary =
            matches!(&provenance, ItemProvenance::Agent { name } if name == "context-summary");
        assert_eq!(transcript.len() > 16_000, known_summary);
        if provenance == ItemProvenance::Unattributed {
            assert!(transcript.starts_with("[user]"));
        }
    }
}
