//! Baseline Messages shaping, separate from credential lookup and HTTP retries.
use crate::auth::AuthMode;
use adk_core::ModelRequest;
use serde_json::{Value, json};

pub const CLAUDE_CODE_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

fn adaptive(model: &str, copilot: bool) -> bool {
    let model = model.trim().to_lowercase();
    let model = model.rsplit('/').next().unwrap_or(&model);
    if !model.contains("claude") {
        return false;
    }
    let mut family = "";
    let mut versions = Vec::new();
    for token in model.split('-') {
        if matches!(token, "fable" | "sonnet" | "opus" | "haiku") {
            if family.is_empty() {
                family = token;
            }
            continue;
        }
        if versions.len() >= 2 {
            continue;
        }
        for part in token.splitn(2, '.') {
            let Ok(n) = part.parse::<u16>() else {
                break;
            };
            if n >= 1000 {
                break;
            }
            versions.push(n);
        }
    }
    let major = versions.first().copied().unwrap_or(0);
    let minor = versions.get(1).copied().unwrap_or(0);
    family == "fable"
        || major >= 5
        || (major == 4
            && if copilot {
                minor >= 6
            } else {
                family == "opus" && minor >= 7
            })
}

/// Apply subscription identity and model-dependent thinking to a serialized
/// Messages request. Raw explicit `thinking` settings take precedence over an
/// effort label. Cache breakpoints are enabled for the baseline Copilot shim.
pub(crate) fn shape(body: &mut Value, request: &ModelRequest, mode: AuthMode) {
    let copilot = mode == AuthMode::CopilotOAuth;
    if mode == AuthMode::AnthropicOAuth {
        body["system"]
            .as_array_mut()
            .expect("Messages system array")
            .insert(0, json!({"type":"text","text":CLAUDE_CODE_IDENTITY}));
    }
    if copilot && !request.settings.contains_key("max_tokens") {
        body["max_tokens"] = 64000.into();
    }
    if let Some(tools) = body["tools"].as_array_mut() {
        for tool in tools.iter_mut() {
            if tool["input_schema"].is_object() && tool["input_schema"].get("properties").is_none()
            {
                tool["input_schema"]["properties"] = json!({});
            }
        }
    }
    let effort = request
        .settings
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let explicit_budget = request
        .settings
        .get("thinking_budget")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    body.as_object_mut()
        .expect("Messages object")
        .remove("reasoning_effort");
    body.as_object_mut().unwrap().remove("thinking_budget");
    if !request.settings.contains_key("thinking") {
        let (mapped, budget) = match effort.as_str() {
            "minimal" => ("low", 1024),
            "low" => ("low", 2048),
            "medium" => ("medium", 4096),
            "high" => ("high", 8192),
            "xhigh" => ("max", 16384),
            "max" => ("max", 24576),
            _ => ("", 0),
        };
        if !mapped.is_empty() || explicit_budget > 0 {
            if adaptive(&request.model, copilot) {
                body["thinking"] = json!({"type":"adaptive","display":"summarized"});
                body["output_config"]["effort"] =
                    if mapped.is_empty() { "medium" } else { mapped }.into();
            } else {
                let budget = if explicit_budget > 0 {
                    explicit_budget
                } else {
                    budget
                };
                let budget = budget.min(
                    body["max_tokens"]
                        .as_u64()
                        .unwrap_or(16384)
                        .saturating_sub(1024),
                );
                if budget >= 1024 {
                    body["thinking"] = json!({"type":"enabled","budget_tokens":budget});
                }
            }
        }
    }
    if copilot {
        for field in ["tools", "system"] {
            if let Some(last) = body[field]
                .as_array_mut()
                .and_then(|items| items.last_mut())
            {
                last["cache_control"] = json!({"type":"ephemeral"});
            }
        }
        let mut count = 0;
        if let Some(messages) = body["messages"].as_array_mut() {
            for message in messages.iter_mut().rev() {
                let Some(blocks) = message["content"].as_array_mut() else {
                    continue;
                };
                if let Some(block) = blocks.iter_mut().rev().find(|block| {
                    matches!(
                        block["type"].as_str(),
                        Some("text" | "tool_result" | "tool_use" | "image" | "document")
                    )
                }) {
                    block["cache_control"] = json!({"type":"ephemeral"});
                    count += 1;
                    if count == 2 {
                        break;
                    }
                }
            }
        }
    }
}

pub(crate) fn beta(model: &str, oauth: bool) -> String {
    let mut values = vec!["claude-code-20250219"];
    if !model.contains("claude-3-") {
        values.push("interleaved-thinking-2025-05-14");
    }
    values.extend([
        "context-management-2025-06-27",
        "prompt-caching-scope-2026-01-05",
    ]);
    if oauth {
        values.push("oauth-2025-04-20");
    }
    values.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{self, Protocol};
    fn request(model: &str, effort: &str) -> ModelRequest {
        ModelRequest {
            model: model.into(),
            instructions: "host instructions".into(),
            input: vec![],
            tools: vec![],
            output_schema: Some(json!({"type":"object"}).try_into().unwrap()),
            output_schema_name: "result".into(),
            output_schema_strict: true,
            settings: [("reasoning_effort".into(), json!(effort))]
                .into_iter()
                .collect(),
        }
    }
    #[test]
    fn model_generations_and_budget_limits_match_reference() {
        for (model, direct, gateway) in [
            ("claude-3-7-sonnet", false, false),
            ("claude-sonnet-4-5-20250929", false, false),
            ("claude-sonnet-4.6", false, true),
            ("claude-opus-4.7", true, true),
            ("anthropic/claude-fable-5", true, true),
            ("claude-sonnet-5", true, true),
        ] {
            assert_eq!(adaptive(model, false), direct);
            assert_eq!(adaptive(model, true), gateway);
        }
        let mut req = request("claude-sonnet-4.5", "max");
        req.settings.insert("max_tokens".into(), json!(4096));
        let body = wire::request(&req, Protocol::Anthropic, false).unwrap();
        assert_eq!(
            body["thinking"],
            json!({"type":"enabled","budget_tokens":3072})
        );
        assert!(body.get("reasoning_effort").is_none());
        req.settings.insert("max_tokens".into(), json!(1024));
        assert!(
            wire::request(&req, Protocol::Anthropic, false)
                .unwrap()
                .get("thinking")
                .is_none()
        );
    }
    #[test]
    fn adaptive_output_schema_subscription_identity_and_cache_are_independent() {
        let req = request("claude-sonnet-4.6", "xhigh");
        let mut body = wire::request(&req, Protocol::Anthropic, false).unwrap();
        shape(&mut body, &req, AuthMode::CopilotOAuth);
        assert_eq!(
            body["thinking"],
            json!({"type":"adaptive","display":"summarized"})
        );
        assert_eq!(body["output_config"]["effort"], "max");
        assert_eq!(
            body["output_format"],
            json!({"type":"json_schema","schema":{"type":"object"}})
        );
        assert_eq!(
            body["system"][0]["cache_control"],
            json!({"type":"ephemeral"})
        );
        let mut oauth = wire::request(&req, Protocol::Anthropic, false).unwrap();
        shape(&mut oauth, &req, AuthMode::AnthropicOAuth);
        assert_eq!(oauth["system"][0]["text"], CLAUDE_CODE_IDENTITY);
        assert_eq!(oauth["system"][1]["text"], "host instructions");
        assert!(oauth["system"][1].get("cache_control").is_none());
    }
}
