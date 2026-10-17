use adk_codec::{
    dto::{RawJson, RunItem, RunItemType, ToolCallData},
    userinput::*,
};
use serde_json::{Value, json};

fn actions(value: Option<Vec<u8>>) -> Value {
    value
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or(Value::Null)
}

#[test]
fn budget_context_helpers_match_pinned_go() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/userinput/observations.json"
    ))
    .unwrap();
    let cases = fixture["caps"].as_array().unwrap();
    assert_eq!(cases.len(), 7);
    for case in cases {
        let value = case["value"].as_i64().unwrap();
        assert_eq!(build_run_budget_context(value), case["run_budget"]);
        assert_eq!(
            build_subagent_budget_context(value),
            case["subagent_budget"]
        );
    }
}

#[test]
fn pinned_userinput_helpers() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/userinput/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let bytes: Vec<u8> = case["input_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(
            extract_ask_user_question(&bytes),
            case["question"],
            "{input}"
        );
        assert_eq!(
            actions(extract_ask_user_choices(&bytes)),
            case["choices"],
            "{input}"
        );
        let (summary, plan_actions) = extract_present_plan_data(&bytes);
        assert_eq!(summary, case["summary"], "{input}");
        assert_eq!(
            String::from_utf8(plan_actions.clone().unwrap_or_default()).unwrap(),
            case["action_bytes"],
            "{input}"
        );
        assert_eq!(
            String::from_utf8(extract_ask_user_choices(&bytes).unwrap_or_default()).unwrap(),
            case["choice_bytes"],
            "{input}"
        );
        assert_eq!(actions(plan_actions), case["actions"], "{input}");
        // Malformed raw JSON cannot be stored in the typed RunItem DTO.
        let Ok(raw) = serde_json::from_slice::<Box<serde_json::value::RawValue>>(&bytes) else {
            continue;
        };
        for pause in case["pauses"].as_array().unwrap() {
            let items = [RunItem {
                kind: RunItemType(1),
                tool_call: Some(ToolCallData {
                    name: pause["name"].as_str().unwrap().into(),
                    input: RawJson::Encoded(raw.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            }];
            let actual = detect_user_input_pause(&items, pause["final"].as_str().unwrap());
            assert_eq!(
                json!({"Requested":actual.requested,"PlanReview":actual.plan_review,"Question":actual.question,"Actions":actions(actual.actions)}),
                pause["pause"],
                "{input}"
            );
        }
    }
    assert_eq!(
        String::from_utf8(marshal_quick_actions(None)).unwrap(),
        fixture["marshal_nil"]
    );
    assert_eq!(
        String::from_utf8(marshal_quick_actions(Some(&[]))).unwrap(),
        fixture["marshal_empty"]
    );
    assert_eq!(build_auto_turn_cap_prompt(-12), fixture["cap"]);
    for cap in fixture["caps"].as_array().unwrap() {
        assert_eq!(
            build_auto_turn_cap_prompt(cap["value"].as_i64().unwrap()),
            cap["text"]
        );
    }
}

#[test]
fn first_matching_call_wins_and_missing_call_is_ignored() {
    let items = vec![
        RunItem {
            kind: RunItemType(1),
            ..Default::default()
        },
        RunItem {
            kind: RunItemType(1),
            tool_call: Some(ToolCallData {
                name: "present_plan".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
        RunItem {
            kind: RunItemType(1),
            tool_call: Some(ToolCallData {
                name: "AskUserQuestion".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    ];
    let pause = detect_user_input_pause(&items, "  ");
    assert!(pause.requested && pause.plan_review);
    assert_eq!(pause.question, "The agent needs your input to continue.");
    assert_eq!(
        detect_user_input_pause(&[], "text"),
        UserInputPause::default()
    );
}

#[test]
fn original_quick_actions_omit_legacy_value_key() {
    let bytes = marshal_quick_actions(Some(&[
        QuickAction {
            id: "approve".into(),
            label: "Approve".into(),
            style: "primary".into(),
        },
        QuickAction {
            id: "request_changes".into(),
            label: "Request Changes".into(),
            ..Default::default()
        },
    ]));
    let actions: Vec<std::collections::BTreeMap<String, String>> =
        serde_json::from_slice(&bytes).unwrap();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0]["id"], "approve");
    assert_eq!(actions[1]["id"], "request_changes");
    assert!(!actions[0].contains_key("value"));
}

#[test]
fn original_ask_user_choices_receive_ids_and_styles() {
    let bytes = extract_ask_user_choices(br#"{"choices":["Ship it","Revise"]}"#).unwrap();
    assert!(!bytes.is_empty());
    let actions: Vec<QuickAction> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(actions.len(), 2);
    assert_eq!(
        actions[0],
        QuickAction {
            id: "choice_0".into(),
            label: "Ship it".into(),
            style: "primary".into()
        }
    );
    assert_eq!(actions[1].style, "secondary");
    assert!(extract_ask_user_choices(br#"{"question":"Continue?"}"#).is_none());
}

#[test]
fn original_present_plan_data_drops_legacy_mode() {
    let (summary, actions) = extract_present_plan_data(br#"{"summary":"Do the thing","actions":[{"id":"approve","label":"Approve","mode":"build"}],"recommended":"approve"}"#);
    assert_eq!(summary, "Do the thing");
    let actions = actions.unwrap();
    let parsed: Vec<QuickAction> = serde_json::from_slice(&actions).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "approve");
    let text = String::from_utf8(actions).unwrap();
    assert!(!text.contains("\"mode\""));
    assert!(!text.contains("\"build\""));
}

#[test]
fn original_user_input_pause_detects_question_plan_and_fallback() {
    for (name, input, final_text, plan, question) in [
        (
            "AskUserQuestion",
            r#"{"question":"Continue?","choices":["Yes","No"]}"#,
            "",
            false,
            "Continue?",
        ),
        (
            "present_plan",
            r#"{"summary":"Review this plan","actions":[{"id":"approve","label":"Approve"}]}"#,
            "",
            true,
            "Review this plan",
        ),
        (
            "AskUserQuestion",
            "{}",
            "fallback text",
            false,
            "fallback text",
        ),
    ] {
        let pause = detect_user_input_pause(
            &[RunItem {
                kind: RunItemType(1),
                tool_call: Some(ToolCallData {
                    name: name.into(),
                    input: RawJson::Encoded(serde_json::from_str(input).unwrap()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            final_text,
        );
        assert!(pause.requested);
        assert_eq!(pause.plan_review, plan);
        assert_eq!(pause.question, question);
        if question == "Continue?" {
            assert!(
                String::from_utf8(pause.actions.unwrap())
                    .unwrap()
                    .contains("\"label\":\"Yes\"")
            );
        }
    }
}

#[test]
fn original_auto_turn_cap_prompt() {
    assert_eq!(
        build_auto_turn_cap_prompt(123),
        "Auto mode global turn cap (123) reached."
    );
}
