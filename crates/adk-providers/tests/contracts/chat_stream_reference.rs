use super::*;
use adk_core::{Content, Message, ModelEvent, ModelResponse, Reasoning, Role, RunItem};
use serde_json::Value;

fn drain(chunks: &[&str]) -> (ModelResponse, Value) {
    let sse: String = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    let mut decoder = Decoder::default();
    let mut stream = StreamState::new(Protocol::Chat);
    let mut completed = Vec::new();
    for bytes in sse.as_bytes().chunks(17) {
        for event in decoder.feed(bytes).unwrap() {
            for event in stream.event(&event.data).unwrap() {
                if let ModelEvent::Complete { response } = event {
                    completed.push(response);
                }
            }
        }
    }
    decoder.finish().unwrap();
    assert_eq!(completed.len(), 1);
    let response = completed.pop().unwrap();
    let snapshot = serde_json::from_str(response.snapshot_raw.as_ref().unwrap().as_str()).unwrap();
    (response, snapshot)
}

#[test]
fn original_reasoning_and_text() {
    let (_, response) = drain(&[
        r#"{"id":"chatcmpl-1","model":"claude-opus-4.8","choices":[{"index":0,"delta":{"role":"assistant"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"reasoning_text":"Let me think"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"reasoning_text":" about it."}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"reasoning_opaque":"SIG123","content":"Hello"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"content":" world"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
        "[DONE]",
    ]);
    let content = response["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[0]["thinking"], "Let me think about it.");
    assert_eq!(content[0]["signature"], "SIG123");
    assert_eq!(content[1]["type"], "text");
    assert_eq!(content[1]["text"], "Hello world");
    assert_eq!(response["stop_reason"], "end_turn");
    assert_eq!(response["usage"]["input_tokens"], 10);
    assert_eq!(response["usage"]["output_tokens"], 5);
}

#[test]
fn original_narration_tool_calls_quirk() {
    let (_, response) = drain(&[
        r#"{"id":"c1","model":"claude-opus-4.8","choices":[{"index":0,"delta":{"content":"Working on it"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(response["stop_reason"], "end_turn");
    assert_eq!(response["content"].as_array().unwrap().len(), 1);
    assert_eq!(response["content"][0]["type"], "text");
}

#[test]
fn original_tool_calls() {
    let (_, response) = drain(&[
        r#"{"id":"c1","model":"claude-opus-4.8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"foo","arguments":"{\"a\":"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(response["stop_reason"], "tool_use");
    assert_eq!(response["content"].as_array().unwrap().len(), 1);
    let tool = &response["content"][0];
    assert_eq!(tool["type"], "tool_use");
    assert_eq!(tool["id"], "call_1");
    assert_eq!(tool["name"], "foo");
    assert_eq!(serde_json::to_string(&tool["input"]).unwrap(), r#"{"a":1}"#);
}

#[test]
fn original_multiple_tool_calls() {
    let (_, response) = drain(&[
        r#"{"id":"c1","model":"claude-opus-4.8","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"alpha","arguments":"{\"x\":1}"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"beta","arguments":"{\"y\":2}"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(response["content"].as_array().unwrap().len(), 2);
    for (index, id, name, input) in [
        (0, "call_a", "alpha", r#"{"x":1}"#),
        (1, "call_b", "beta", r#"{"y":2}"#),
    ] {
        let tool = &response["content"][index];
        assert_eq!(tool["type"], "tool_use");
        assert_eq!(tool["id"], id);
        assert_eq!(tool["name"], name);
        assert_eq!(serde_json::to_string(&tool["input"]).unwrap(), input);
    }
    assert_eq!(response["stop_reason"], "tool_use");
}

#[test]
fn original_reasoning_then_tool() {
    let (_, response) = drain(&[
        r#"{"id":"c1","model":"claude-opus-4.8","choices":[{"index":0,"delta":{"reasoning_text":"pick a tool"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"reasoning_opaque":"SIGZ","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"foo","arguments":"{}"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(response["content"].as_array().unwrap().len(), 2);
    assert_eq!(response["content"][0]["type"], "thinking");
    assert_eq!(response["content"][0]["thinking"], "pick a tool");
    assert_eq!(response["content"][0]["signature"], "SIGZ");
    assert_eq!(response["content"][1]["type"], "tool_use");
    assert_eq!(response["content"][1]["name"], "foo");
    assert_eq!(response["stop_reason"], "tool_use");
}

#[test]
fn original_complete_chat_reasoning() {
    let response = wire::response(&json!({"id":"msg_1", "model":"claude-opus-4.8", "choices":[{
        "message":{"role":"assistant", "content":"done", "reasoning_text":"because reasons", "reasoning_opaque":"OPAQUE1"}, "finish_reason":"stop"
    }]}), Protocol::Chat).unwrap();
    let response: Value = serde_json::from_str(response.snapshot_raw.unwrap().as_str()).unwrap();
    assert_eq!(response["content"].as_array().unwrap().len(), 2);
    assert_eq!(response["content"][0]["type"], "thinking");
    assert_eq!(response["content"][0]["thinking"], "because reasons");
    assert_eq!(response["content"][0]["signature"], "OPAQUE1");
    assert_eq!(response["stop_reason"], "end_turn");
}

#[test]
fn original_reasoning_round_trip_and_plaintext() {
    for (text, signature) in [("deep thought", "SIGabc"), ("no signature here", "")] {
        let mut input = request();
        input.instructions.clear();
        input.input = vec![
            RunItem::Reasoning {
                reasoning: Reasoning {
                    text: text.into(),
                    signature: signature.into(),
                    ..Default::default()
                },
            },
            RunItem::Message {
                message: Message {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: "answer".into(),
                    }],
                },
            },
        ];
        let body = wire::request(&input, Protocol::Chat, false).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        if signature.is_empty() {
            assert_eq!(messages[0]["reasoning"], text);
            for key in ["reasoning_text", "reasoning_opaque", "reasoning_details"] {
                assert!(messages[0].get(key).is_none(), "{key}: {body}");
            }
        } else {
            assert_eq!(messages[0]["reasoning_text"], text);
            assert_eq!(messages[0]["reasoning_opaque"], signature);
        }
    }
}

#[test]
fn openrouter_details_semantic_round_trip() {
    let details = json!([{"type":"reasoning.text","text":"inspect state"},{"type":"reasoning.encrypted","data":"opaque"}]);
    let response = wire::response(&json!({"id":"gen-1", "model":"anthropic/claude-sonnet-4.6", "choices":[{
        "message":{"role":"assistant", "content":null, "reasoning":"inspect state", "reasoning_details":details, "tool_calls":[{"id":"call_1", "type":"function", "function":{"name":"inspect","arguments":"{}"}}]}, "finish_reason":"tool_calls"
    }], "usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":70,"cache_write_tokens":5}}}), Protocol::Chat).unwrap();
    let snapshot: Value =
        serde_json::from_str(response.snapshot_raw.as_ref().unwrap().as_str()).unwrap();
    assert_eq!(snapshot["content"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["content"][0]["type"], "thinking");
    assert_eq!(snapshot["content"][1]["type"], "tool_use");
    assert_eq!(response.usage.cache_read_tokens, 70);
    assert_eq!(response.usage.cache_creation_tokens, 5);
    let mut input = request();
    input.instructions.clear();
    input.input = response.items;
    let body = wire::request(&input, Protocol::Chat, false).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["reasoning_details"], details);
    assert!(messages[0].get("reasoning").is_none());
    assert!(messages[0].get("reasoning_opaque").is_none());
}

#[test]
fn original_openrouter_stream_details_and_cache_usage() {
    let (native, response) = drain(&[
        r#"{"id":"gen-1","model":"anthropic/claude-sonnet-4.6","choices":[{"index":0,"delta":{"reasoning":"inspect ","reasoning_details":[{"type":"reasoning.text","text":"inspect "}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"reasoning":"state","reasoning_details":[{"type":"reasoning.text","text":"state"}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"inspect","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120,"prompt_tokens_details":{"cached_tokens":70,"cache_write_tokens":5}}}"#,
        "[DONE]",
    ]);
    assert_eq!(response["content"].as_array().unwrap().len(), 2);
    assert_eq!(response["content"][0]["thinking"], "inspect state");
    assert_eq!(response["content"][1]["type"], "tool_use");
    let details =
        wire::decode_reasoning_details(response["content"][0]["signature"].as_str().unwrap())
            .unwrap();
    assert_eq!(
        details,
        json!([{"type":"reasoning.text","text":"inspect "},{"type":"reasoning.text","text":"state"}])
    );
    use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
    let signature = response["content"][0]["signature"].as_str().unwrap();
    assert_eq!(STANDARD_NO_PAD.decode(signature.strip_prefix("openrouter-reasoning-details:").unwrap()).unwrap(), br#"[{"type":"reasoning.text","text":"inspect "},{"type":"reasoning.text","text":"state"}]"#);
    assert_eq!(native.usage.cache_read_tokens, 70);
    assert_eq!(native.usage.cache_creation_tokens, 5);
}

#[test]
fn buffered_reasoning_signatures_preserve_pinned_go_bytes() {
    use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/providers/continuation.json"
    ))
    .unwrap();
    let cases = fixture["buffered_chat_details"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let response =
            wire::response_json(case["body"].as_str().unwrap().as_bytes(), Protocol::Chat).unwrap();
        let RunItem::Reasoning { reasoning } = &response.items[0] else {
            panic!("missing reasoning")
        };
        assert_eq!(reasoning.signature, case["signature"].as_str().unwrap());
        let raw = STANDARD_NO_PAD
            .decode(
                reasoning
                    .signature
                    .strip_prefix("openrouter-reasoning-details:")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(raw, case["details"].as_str().unwrap().as_bytes());
        let snapshot: Value =
            serde_json::from_str(response.snapshot_raw.unwrap().as_str()).unwrap();
        assert_eq!(snapshot["content"][0]["signature"], case["signature"]);
    }
}

#[test]
fn streamed_reasoning_signatures_preserve_pinned_go_bytes() {
    use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/providers/continuation.json"
    ))
    .unwrap();
    let cases = fixture["streamed_chat_details"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let chunks: Vec<_> = case["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chunk| chunk.as_str().unwrap())
            .collect();
        let (native, snapshot) = drain(&chunks);
        let RunItem::Reasoning { reasoning } = &native.items[0] else {
            panic!("missing reasoning")
        };
        assert_eq!(reasoning.signature, case["signature"].as_str().unwrap());
        assert_eq!(reasoning.text, "inspect state");
        let raw = STANDARD_NO_PAD
            .decode(
                reasoning
                    .signature
                    .strip_prefix("openrouter-reasoning-details:")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(raw, case["details"].as_str().unwrap().as_bytes());
        assert_eq!(snapshot["content"][0]["signature"], case["signature"]);
        assert_eq!(
            native.raw.as_ref().unwrap()["choices"][0]["message"]["reasoning_details"],
            serde_json::from_slice::<Value>(&raw).unwrap()
        );
    }
}
