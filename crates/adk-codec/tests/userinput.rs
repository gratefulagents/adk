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
