use adk_codec::snapshots::{ModelSettings as SnapshotModelSettings, OutputSchema, ToolSnapshot};
use adk_codec::{GoJsonDecode, GoJsonEncode, approval, dto::*, from_go_json, to_go_json_bytes};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

trait Observation: GoJsonDecode + GoJsonEncode + Serialize {
    fn fields(&self) -> Value;
    fn raw(&self) -> Option<&RawJson> {
        None
    }
}
macro_rules! fields {
    ($ty:ident, $($field:ident),* $(; $raw:ident)?) => {
        impl Observation for $ty {
            fn fields(&self) -> Value { json!({$(stringify!($field): self.$field),*}) }
            $(fn raw(&self) -> Option<&RawJson> { Some(&self.$raw) })?
        }
    };
}
impl Observation for SnapshotModelSettings {
    fn fields(&self) -> Value {
        json!({"temperature": self.temperature.map(|v| format!("{:016x}", v.to_bits())),
            "top_p": self.top_p.map(|v| format!("{:016x}", v.to_bits())),
            "max_tokens": self.max_tokens, "tool_choice": self.tool_choice,
            "parallel_tool_calls": self.parallel_tool_calls, "thinking_budget": self.thinking_budget,
            "reasoning_effort": self.reasoning_effort, "text_verbosity": self.text_verbosity,
            "stop_sequences": self.stop_sequences})
    }
}
fields!(ImageAttachment, media_type, data, detail);
fields!(OutputSchema, name, strict; schema);
fields!(ToolSnapshot, name, description, read_only, needs_approval, timeout_seconds; input_schema);
fields!(ToolCallData, id, name; input);
fields!(ToolCallSnapshot, id, name; input);
fields!(ToolApprovalSnapshot, tool_name, call_id, approved; input);
fields!(HandoffCallData, from_agent, to_agent);
fields!(HandoffOutputData, from_agent, to_agent);
fields!(
    ReasoningData,
    id,
    text,
    signature,
    redacted_data,
    encrypted_content
);
fields!(CompactionData, id, content, encrypted_content, created_by);
fields!(
    ReasoningSnapshot,
    id,
    text,
    thinking,
    signature,
    redacted_data,
    encrypted_content
);
fields!(
    CompactionSnapshot,
    id,
    content,
    encrypted_content,
    created_by
);
fields!(ToolApprovalData, tool_name, call_id, approved; input);
impl Observation for MessageOutput {
    fn fields(&self) -> Value {
        json!({"text": self.text, "phase": self.phase,
            "images": self.images.iter().map(Observation::fields).collect::<Vec<_>>()})
    }
}
impl Observation for ToolOutputData {
    fn fields(&self) -> Value {
        json!({"call_id": self.call_id, "content": self.content, "is_error": self.is_error,
            "images": self.images.iter().map(Observation::fields).collect::<Vec<_>>()})
    }
}
impl Observation for RunItemType {
    fn fields(&self) -> Value {
        json!({"value": self.0})
    }
}

fn compare<T: Observation>(case: &Value) {
    let id = case["id"].as_str().unwrap();
    let input = unhex(case["input_hex"].as_str().unwrap());
    let decoded = from_go_json::<T>(&input);
    assert_eq!(
        decoded.is_ok(),
        case["decode_ok"].as_bool().unwrap(),
        "{id}: {:?}",
        decoded.as_ref().err()
    );
    let Ok(decoded) = decoded else { return };
    let mut expected = case["decoded_fields"].clone();
    // Public Vec fields intentionally collapse nil and empty; both omit on marshal.
    if expected.get("images") == Some(&Value::Null) {
        expected["images"] = json!([]);
    }
    if expected.get("stop_sequences") == Some(&Value::Null) {
        expected["stop_sequences"] = json!([]);
    }
    assert_eq!(decoded.fields(), expected, "decoded fields: {id}");
    match decoded.raw() {
        Some(raw) => {
            assert_eq!(
                !raw.is_missing(),
                case["raw_input"]["present"].as_bool().unwrap(),
                "raw presence: {id}"
            );
            assert_eq!(
                raw.bytes().as_ref(),
                unhex(case["raw_input"]["hex"].as_str().unwrap()),
                "raw bytes: {id}"
            );
        }
        None => assert!(case["raw_input"].is_null(), "{id}"),
    }
    let marshaled = to_go_json_bytes(&decoded);
    assert_eq!(
        marshaled.is_ok(),
        case["marshal_ok"].as_bool().unwrap(),
        "marshal success: {id}"
    );
    if let Ok(bytes) = marshaled {
        assert_eq!(
            bytes,
            unhex(case["marshal_hex"].as_str().unwrap()),
            "marshal bytes: {id}"
        );
    }
}

#[test]
fn pinned_payload_decode_oracle() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/payloads/observations.json")).unwrap();
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(fixture["int_size"], 64);
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    for (name, kind) in [
        ("RunItemMessage", RunItemType::MESSAGE),
        ("RunItemToolCall", RunItemType::TOOL_CALL),
        ("RunItemToolOutput", RunItemType::TOOL_OUTPUT),
        ("RunItemHandoffCall", RunItemType::HANDOFF_CALL),
        ("RunItemHandoffOutput", RunItemType::HANDOFF_OUTPUT),
        ("RunItemReasoning", RunItemType::REASONING),
        ("RunItemToolApproval", RunItemType::TOOL_APPROVAL),
        ("RunItemCompaction", RunItemType::COMPACTION),
    ] {
        assert_eq!(fixture["enum_values"][name], kind.0);
    }
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2598);
    for case in cases {
        match case["type"].as_str().unwrap() {
            "ModelSettings" => compare::<SnapshotModelSettings>(case),
            "LLMOutputSchema" => compare::<OutputSchema>(case),
            "LLMToolSnapshot" => compare::<ToolSnapshot>(case),
            "MessageOutput" => compare::<MessageOutput>(case),
            "ImageAttachment" => compare::<ImageAttachment>(case),
            "ToolCallData" => compare::<ToolCallData>(case),
            "ToolOutputData" => compare::<ToolOutputData>(case),
            "HandoffCallData" => compare::<HandoffCallData>(case),
            "HandoffOutputData" => compare::<HandoffOutputData>(case),
            "ReasoningData" => compare::<ReasoningData>(case),
            "CompactionData" => compare::<CompactionData>(case),
            "LLMReasoning" => compare::<ReasoningSnapshot>(case),
            "LLMCompaction" => compare::<CompactionSnapshot>(case),
            "LLMToolCall" => compare::<ToolCallSnapshot>(case),
            "LLMToolApproval" => compare::<ToolApprovalSnapshot>(case),
            "ToolApprovalData" => compare::<ToolApprovalData>(case),
            "RunItemType" => compare::<RunItemType>(case),
            other => panic!("unknown oracle type: {other}"),
        }
    }
}

#[test]
fn public_raw_representations_share_go_marshal_depth_limit() {
    for depth in [10_000, 10_001] {
        let text = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        for raw in [
            RawJson::Encoded(serde_json::value::RawValue::from_string(text.clone()).unwrap()),
            RawJson::EncodedBytes(text.as_bytes().to_vec()),
        ] {
            let call = ToolCallData {
                input: raw.clone(),
                ..Default::default()
            };
            let approval = ToolApprovalData {
                input: raw.clone(),
                ..Default::default()
            };
            let schema = OutputSchema {
                schema: raw.clone(),
                ..Default::default()
            };
            let tool = ToolSnapshot {
                input_schema: raw,
                ..Default::default()
            };
            assert_eq!(to_go_json_bytes(&schema).is_ok(), depth == 10_000);
            assert_eq!(to_go_json_bytes(&tool).is_ok(), depth == 10_000);
            assert_eq!(adk_codec::to_go_json_bytes(&call).is_ok(), depth == 10_000);
            assert_eq!(
                adk_codec::to_go_json_bytes(&approval).is_ok(),
                depth == 10_000
            );
        }
    }
}

#[test]
fn ordinary_serde_retains_duplicate_and_in_place_semantics() {
    let mut message = MessageOutput {
        text: "before".into(),
        images: vec![ImageAttachment {
            data: "keep".into(),
            detail: "high".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut de = serde_json::Deserializer::from_str(
        r#"{"TEXT":null,"images":[{"MEDIA_TYPE":"new"}],"Images":[null]}"#,
    );
    MessageOutput::deserialize_in_place(&mut de, &mut message).unwrap();
    assert_eq!(message.text, "before");
    assert_eq!(message.images[0].data, "keep");
    assert_eq!(message.images[0].media_type, "new");
    MessageOutput::deserialize_in_place(
        &mut serde_json::Deserializer::from_str("null"),
        &mut message,
    )
    .unwrap();
    assert_eq!(message.images[0].detail, "high");
    let mut kind = RunItemType(99);
    RunItemType::deserialize_in_place(&mut serde_json::Deserializer::from_str("null"), &mut kind)
        .unwrap();
    assert_eq!(kind.0, 99);
    assert_eq!(serde_json::from_str::<RunItemType>("-0").unwrap().0, 0);
    assert!(serde_json::from_str::<MessageOutput>(r#"{"text":"\ud800"}"#).is_err());
    assert_eq!(
        from_go_json::<MessageOutput>(br#"{"text":"\ud800"}"#)
            .unwrap()
            .text,
        "�"
    );
}

#[test]
fn raw_projection_and_bridges_fail_without_panicking_or_false_equality() {
    let call = from_go_json::<ToolCallData>(br#"{"input":{"x":"\ud800"}}"#).unwrap();
    assert!(call.input.try_value().is_err());
    assert!(call.input.value().is_none());
    assert_eq!(call.input, call.input.clone());
    assert_ne!(call.input, RawJson::Missing);
    assert_ne!(call.input, RawJson::Present(Value::Null));
    let different = from_go_json::<ToolCallData>(br#"{"input":{"x":"\ud801"}}"#).unwrap();
    assert_ne!(call.input, different.input);
    let wire = RunItem {
        kind: RunItemType::TOOL_CALL,
        tool_call: Some(call.clone()),
        ..Default::default()
    };
    assert!(approval::decode_item(&wire).is_err());
    assert!(
        approval::approval_call(&ToolApprovalData {
            input: call.input,
            ..Default::default()
        })
        .is_err()
    );
    let deep = format!("{{\"input\":{}0{}}}", "[".repeat(200), "]".repeat(200));
    let call = from_go_json::<ToolCallData>(deep.as_bytes()).unwrap();
    assert!(call.input.try_value().is_err());
    assert_eq!(call.input, call.input.clone());
}

#[test]
fn raw_spans_are_not_normalized_or_value_roundtripped() {
    let raw = r#"{ "x" : 1e+09, "x" : -0, "escape" : "\ud800", "input" : "\u0061" }"#;
    let json = format!(r#"{{"NAME":"\ud800","INP\u0055T":{raw},"id":"after"}}"#);
    let call = from_go_json::<ToolCallData>(json.as_bytes()).unwrap();
    assert_eq!(call.name, "�");
    assert_eq!(call.id, "after");
    assert_eq!(call.input.try_text().unwrap(), raw);
    let call = from_go_json::<ToolCallData>(br#"{"input":{"old":1},"INPUT":null}"#).unwrap();
    assert!(!call.input.is_missing());
    assert_eq!(call.input.try_text().unwrap(), "null");
}

#[test]
fn invalid_utf8_raw_is_preserved_but_native_projections_reject_it() {
    let call: ToolCallData = from_go_json(b"{\"input\":{\"x\":\"\xff\"}}").unwrap();
    assert!(matches!(call.input, RawJson::EncodedBytes(_)));
    assert_eq!(call.input.bytes().as_ref(), b"{\"x\":\"\xff\"}");
    assert!(call.input.try_text().is_err());
    assert!(call.input.try_value().is_err());
    assert!(call.input.value().is_none());
    assert_eq!(call.input, call.input.clone());
    assert_ne!(call.input, RawJson::Missing);
    assert_ne!(call.input, RawJson::Present(Value::Null));
    assert_ne!(
        call.input,
        RawJson::EncodedBytes(b"{\"x\":\"\xfe\"}".to_vec())
    );
    assert!(serde_json::to_vec(&call).is_err());
    assert!(adk_codec::snapshots::to_go_json(&call).is_err());
    assert!(
        approval::decode_item(&RunItem {
            kind: RunItemType::TOOL_CALL,
            tool_call: Some(call.clone()),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        approval::approval_call(&ToolApprovalData {
            input: call.input,
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn raw_overwrites_validate_every_occurrence_and_restore_only_the_last() {
    let call: ToolCallData =
        from_go_json(b"{\"input\":\"\xff\",\"INPUT\":{\"input\":\"nested\"},\"name\":\"input\"}")
            .unwrap();
    assert!(matches!(call.input, RawJson::Encoded(_)));
    assert_eq!(call.input.bytes().as_ref(), br#"{"input":"nested"}"#);
    let call: ToolCallData = from_go_json(b"{\"input\":null,\"INP\\u0055T\":\"\xff\"}").unwrap();
    assert_eq!(call.input.bytes().as_ref(), b"\"\xff\"");
    let call: ToolCallData = from_go_json(b"{\"input\":\"\xff\",\"INPUT\":null}").unwrap();
    assert!(matches!(call.input, RawJson::Encoded(_)));
    assert_eq!(call.input.bytes().as_ref(), b"null");
    for input in [
        b"{\"input\":[\"\xff\",],\"input\":null}".as_slice(),
        b"{\"input\":\"\\q\xff\",\"input\":null}",
        b"{\"input\":\"\xff\",\"id\":false,\"call_id\":false}",
        b"{\"input\":\"\xff\"} trailing",
        b"{\"input\":\xff,\"input\":null}",
    ] {
        assert!(from_go_json::<ToolCallData>(input).is_err(), "{input:?}");
        assert!(
            from_go_json::<ToolApprovalData>(input).is_err(),
            "{input:?}"
        );
    }
}

#[test]
fn public_raw_bytes_validate_syntax_not_value_projection() {
    for raw in [
        b"".as_slice(),
        b" ",
        b"true false",
        b"[1,]",
        b"{\"x\":}",
        b"\"\\q\"",
        b"\"\xff\n\"",
        b"\xff",
        b"{\"x\":\"\xff\",}",
    ] {
        let call = ToolCallData {
            input: RawJson::EncodedBytes(raw.to_vec()),
            ..Default::default()
        };
        assert!(to_go_json_bytes(&call).is_err(), "{raw:?}");
    }
    for raw in [
        br#"{"x":1e9999,"x":"\ud800"}"#.to_vec(),
        format!("{}0{}", "[".repeat(200), "]".repeat(200)).into_bytes(),
        b"\"\xff\"".to_vec(),
    ] {
        let call = ToolCallData {
            input: RawJson::EncodedBytes(raw.clone()),
            ..Default::default()
        };
        assert!(to_go_json_bytes(&call).is_ok(), "{raw:?}");
    }
    let call = ToolCallData {
        input: RawJson::EncodedBytes(
            format!("{}0{}", "[".repeat(10_001), "]".repeat(10_001)).into_bytes(),
        ),
        ..Default::default()
    };
    assert!(to_go_json_bytes(&call).is_err());
}

#[test]
fn byte_compactor_preserves_raw_escapes_and_invalid_sequences() {
    let input = RawJson::EncodedBytes(
        b" { \"x\" : \"\xff<>&\xe2\x80\xa8\xe2\x80\xa9\xe2\x80 \\\" \\\\ \\u003c\", \"x\" : -0 } "
            .to_vec(),
    );
    let call = ToolCallData {
        id: "<&".into(),
        name: "\u{2028}".into(),
        input: input.clone(),
    };
    assert_eq!(to_go_json_bytes(&call).unwrap(), b"{\"id\":\"\\u003c\\u0026\",\"name\":\"\\u2028\",\"input\":{\"x\":\"\xff\\u003c\\u003e\\u0026\\u2028\\u2029\xe2\x80 \\\" \\\\ \\u003c\",\"x\":-0}}");
    let approval = ToolApprovalData {
        input,
        ..Default::default()
    };
    let marshaled = to_go_json_bytes(&approval).unwrap();
    let decoded: ToolApprovalData = from_go_json(&marshaled).unwrap();
    assert_eq!(to_go_json_bytes(&decoded).unwrap(), marshaled);
}

#[test]
fn user_input_helpers_receive_original_raw_bytes() {
    let call: ToolCallData =
        from_go_json(b"{\"name\":\"AskUserQuestion\",\"input\":{\"question\":\"\xf0\x90\x80\"}}")
            .unwrap();
    let pause = adk_codec::userinput::detect_user_input_pause(
        &[RunItem {
            kind: RunItemType::TOOL_CALL,
            tool_call: Some(call),
            ..Default::default()
        }],
        "",
    );
    assert!(pause.requested);
    assert_eq!(pause.question, "\u{fffd}\u{fffd}\u{fffd}");
}

#[test]
fn snapshot_raw_encoding_matches_go_omission_bytes_and_rejection() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/payloads/observations.json")).unwrap();
    let cases = fixture["snapshot_encode_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 28);
    for case in cases {
        let raw = if case["raw_nil"].as_bool().unwrap() {
            RawJson::Missing
        } else {
            RawJson::EncodedBytes(unhex(case["raw_hex"].as_str().unwrap()))
        };
        let encoded = match case["type"].as_str().unwrap() {
            "LLMOutputSchema" => to_go_json_bytes(&OutputSchema {
                schema: raw,
                ..Default::default()
            }),
            "LLMToolSnapshot" => to_go_json_bytes(&ToolSnapshot {
                input_schema: raw,
                ..Default::default()
            }),
            "LLMToolCall" => to_go_json_bytes(&ToolCallSnapshot {
                input: raw,
                ..Default::default()
            }),
            "LLMToolApproval" => to_go_json_bytes(&ToolApprovalSnapshot {
                input: raw,
                ..Default::default()
            }),
            other => panic!("unknown type {other}"),
        };
        assert_eq!(
            encoded.is_ok(),
            case["marshal_ok"].as_bool().unwrap(),
            "{case}"
        );
        if let Ok(encoded) = encoded {
            assert_eq!(
                encoded,
                unhex(case["marshal_hex"].as_str().unwrap()),
                "{case}"
            );
        }
    }
}
