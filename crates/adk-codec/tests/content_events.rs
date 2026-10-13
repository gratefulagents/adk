use adk_codec::{dto::ContentEvent, events::compact_content_event, snapshots::to_go_json};
use serde_json::Value;

#[test]
fn typed_content_event_format_matches_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/content-event/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let event: ContentEvent = serde_json::from_value(case["event"].clone()).unwrap();
        assert_eq!(
            compact_content_event(&event),
            case["compact"].as_str().unwrap()
        );
    }
}

#[test]
fn canonical_content_event_fields_match_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/content-event/observations.json"
    ))
    .unwrap();
    let expected_fields = [
        ("Timestamp", "ts", "time.Time"),
        ("Type", "type", "string"),
        ("Session", "session", "int32"),
        ("Message", "message", "string"),
        ("Tool", "tool", "string"),
        ("ToolUseID", "tool_use_id", "string"),
        ("ParentCallID", "parent_call_id", "string"),
        ("IsError", "is_error", "bool"),
        ("AgentName", "agent_name", "string"),
        ("InputRaw", "input_raw", "string"),
        ("Output", "output", "string"),
        ("ToolDurationMS", "tool_duration_ms", "int64"),
        ("Phase", "phase", "string"),
        ("LLMAttempt", "llm_attempt", "int32"),
        ("LLMScope", "llm_scope", "string"),
        ("AttemptNumber", "attempt_number", "int32"),
        ("AttemptStatus", "attempt_status", "string"),
        ("Scope", "scope", "string"),
        ("RequestedModel", "requested_model", "string"),
        ("ResolvedModel", "resolved_model", "string"),
        ("CanonicalModel", "canonical_model", "string"),
        ("Provider", "provider", "string"),
        ("Turn", "turn", "int32"),
        ("UsageAvailable", "usage_available", "bool"),
        ("HasPromptTokens", "has_prompt_tokens", "bool"),
        ("HasCompletionTokens", "has_completion_tokens", "bool"),
        ("HasTotalTokens", "has_total_tokens", "bool"),
        ("PromptTokens", "prompt_tokens", "int64"),
        ("CompletionTokens", "completion_tokens", "int64"),
        ("TotalTokens", "total_tokens", "int64"),
        ("AttemptLatencyMs", "attempt_latency_ms", "int64"),
        ("RetryPlanned", "retry_planned", "bool"),
        ("RetryAfterMs", "retry_after_ms", "int64"),
        ("FallbackPlanned", "fallback_planned", "bool"),
        ("FallbackFromModel", "fallback_from_model", "string"),
        ("FallbackToModel", "fallback_to_model", "string"),
        ("FallbackReason", "fallback_reason", "string"),
        ("FailureKind", "failure_kind", "string"),
        ("TaskID", "task_id", "string"),
        ("Status", "status", "string"),
        ("SubagentType", "subagent_type", "string"),
        ("SubagentModel", "subagent_model", "string"),
        ("SubagentPrompt", "subagent_prompt", "string"),
        ("SubagentResultText", "subagent_result_text", "string"),
        ("SubagentToolCount", "subagent_tool_count", "int32"),
        ("SubagentTokens", "subagent_tokens", "int64"),
        ("SubagentDurationMs", "subagent_duration_ms", "int64"),
        ("SubagentCostUsd", "subagent_cost_usd", "float64"),
        ("SubagentCostKnown", "subagent_cost_known", "bool"),
        ("SubagentNumTurns", "subagent_num_turns", "int32"),
        (
            "SubagentCacheReadTokens",
            "subagent_cache_read_tokens",
            "int64",
        ),
        (
            "SubagentCacheCreateTokens",
            "subagent_cache_create_tokens",
            "int64",
        ),
        ("SubagentStopReason", "subagent_stop_reason", "string"),
        ("SubagentDependsOn", "subagent_depends_on", "[]string"),
        ("SubagentWaitingOn", "subagent_waiting_on", "[]string"),
        ("SubagentCurrentStep", "subagent_current_step", "string"),
        ("SubagentLastTool", "subagent_last_tool", "string"),
        ("SubagentFilesWritten", "subagent_files_written", "int"),
        (
            "SubagentMessagesReceived",
            "subagent_messages_received",
            "int",
        ),
        (
            "SubagentLastParentMessage",
            "subagent_last_parent_message",
            "string",
        ),
        ("Step", "step", "string"),
        ("Model", "model", "string"),
        ("PermissionMode", "permission_mode", "string"),
        ("Cwd", "cwd", "string"),
        ("MaxTurns", "max_turns", "int32"),
        ("Tools", "tools", "[]string"),
        ("McpServers", "mcp_servers", "[]string"),
        ("CostUsd", "cost_usd", "float64"),
        ("CostKnown", "cost_known", "bool"),
        ("InputTokens", "input_tokens", "int64"),
        ("OutputTokens", "output_tokens", "int64"),
        ("CacheReadInputTokens", "cache_read_input_tokens", "int64"),
        (
            "CacheCreationInputTokens",
            "cache_creation_input_tokens",
            "int64",
        ),
        (
            "InputTokensIncludeCache",
            "input_tokens_include_cache",
            "bool",
        ),
        (
            "InputTokensIncludeCacheKnown",
            "input_tokens_include_cache_known",
            "bool",
        ),
        ("NumTurns", "num_turns", "int32"),
        ("DurationMs", "duration_ms", "int64"),
        ("StopReason", "stop_reason", "string"),
        ("TokensBefore", "tokens_before", "int32"),
        ("TokensAfter", "tokens_after", "int32"),
        ("HookName", "hook_name", "string"),
        ("Decision", "decision", "string"),
        ("Reason", "reason", "string"),
    ];
    let fields = fixture["fields"].as_array().unwrap();
    assert_eq!(fields.len(), expected_fields.len());
    let mut failures = Vec::new();
    for (field, (name, json_name, go_type)) in fields.iter().zip(expected_fields) {
        assert_eq!(field["name"], name);
        assert_eq!(field["json_name"], json_name);
        assert_eq!(field["go_type"], go_type);
        let cases = field["cases"].as_array().unwrap();
        let expected_count = match go_type {
            "string" | "bool" => 2,
            "int" | "int32" | "int64" => 5,
            "float64" => 18,
            "[]string" | "time.Time" => 3,
            _ => panic!("uncovered Go type {go_type}"),
        };
        assert_eq!(cases.len(), expected_count, "{name}");
        for (index, case) in cases.iter().enumerate() {
            let canonical = case.as_str().unwrap();
            let result = serde_json::from_str::<ContentEvent>(canonical)
                .and_then(|event| to_go_json(&event));
            match result {
                Ok(actual) if actual == canonical.as_bytes() => {}
                Ok(actual) => failures.push(format!(
                    "{name}[{index}]: expected {canonical}, got {}",
                    String::from_utf8(actual).unwrap()
                )),
                Err(error) => failures.push(format!("{name}[{index}]: {error}; input {canonical}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn content_event_rejects_nonfinite_floats_like_go() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/content-event/observations.json"
    ))
    .unwrap();
    let rejections = fixture["nonfinite_rejections"].as_array().unwrap();
    let float_fields: Vec<_> = fixture["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|field| field["go_type"] == "float64")
        .collect();
    assert_eq!(float_fields.len(), 2);
    assert_eq!(rejections.len(), float_fields.len() * 3);
    for field in float_fields {
        for (label, value) in [
            ("NaN", f64::NAN),
            ("+Inf", f64::INFINITY),
            ("-Inf", f64::NEG_INFINITY),
        ] {
            let matching: Vec<_> = rejections
                .iter()
                .filter(|rejection| {
                    rejection["name"] == field["name"] && rejection["value"] == label
                })
                .collect();
            assert_eq!(matching.len(), 1);
            assert_eq!(matching[0]["json_name"], field["json_name"]);
            assert_eq!(
                matching[0]["error"],
                format!("json: unsupported value: {label}")
            );
            let mut event = ContentEvent::default();
            match field["json_name"].as_str().unwrap() {
                "cost_usd" => event.cost_usd = value,
                "subagent_cost_usd" => event.subagent_cost_usd = value,
                name => panic!("uncovered float field {name}"),
            }
            assert!(
                to_go_json(&event).is_err(),
                "{} accepted {label}",
                field["name"]
            );
        }
    }
}
