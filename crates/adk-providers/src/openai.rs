//! OpenAI-shaped request settings shared by Responses and Chat gateways.
use crate::wire::Protocol;
use adk_core::{Error, ModelRequest};
use serde_json::{Value, json};

fn supports_none(model: &str) -> bool {
    let model = model.trim().to_lowercase();
    if model.contains("codex") {
        return false;
    }
    for (offset, _) in model.match_indices("gpt-") {
        if offset != 0 && !matches!(model.as_bytes()[offset - 1], b'/' | b'-') {
            continue;
        }
        let version = &model[offset + 4..];
        let major: String = version.chars().take_while(char::is_ascii_digit).collect();
        let rest = &version[major.len()..];
        let minor: String = rest
            .strip_prefix('.')
            .unwrap_or("")
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let major = major.parse::<u64>().unwrap_or(0);
        let minor = minor.parse::<u64>().unwrap_or(0);
        return major > 5 || (major == 5 && minor >= 1);
    }
    false
}
fn budget_effort(budget: u64) -> &'static str {
    match budget {
        12288.. => "xhigh",
        8192.. => "high",
        4096.. => "medium",
        2048.. => "low",
        _ => "minimal",
    }
}
pub(crate) fn shape(
    body: &mut Value,
    request: &ModelRequest,
    protocol: Protocol,
) -> Result<(), Error> {
    let object = body.as_object_mut().expect("serialized request object");
    let budget = request
        .settings
        .get("thinking_budget")
        .and_then(Value::as_u64)
        .or_else(|| {
            request
                .settings
                .get("thinking")
                .and_then(|value| value["budget_tokens"].as_u64())
        })
        .unwrap_or(0);
    if let Some(fallbacks) = object.remove("model_fallbacks") {
        let fallbacks = fallbacks
            .as_array()
            .ok_or_else(|| crate::invalid("model fallbacks must be an array"))?;
        let mut models = vec![request.model.trim().to_owned()];
        for model in fallbacks {
            let model = model
                .as_str()
                .ok_or_else(|| crate::invalid("fallback model must be a string"))?
                .trim();
            if !model.is_empty() && !models.iter().any(|previous| previous == model) {
                models.push(model.to_owned());
            }
        }
        if protocol == Protocol::Chat && models.len() > 1 {
            object.insert("models".into(), json!(models));
        }
    }
    if let Some(verbosity) = object.remove("text_verbosity") {
        let verbosity = verbosity
            .as_str()
            .ok_or_else(|| crate::invalid("text verbosity must be a string"))?
            .trim()
            .to_lowercase();
        if protocol == Protocol::Responses
            && matches!(verbosity.as_str(), "low" | "medium" | "high")
        {
            object.entry("text").or_insert(json!({}))["verbosity"] = verbosity.into();
        }
    }
    if let Some(threshold) = object.remove("compaction_threshold") {
        let threshold = threshold
            .as_u64()
            .ok_or_else(|| crate::invalid("compaction threshold must be a nonnegative integer"))?;
        if protocol == Protocol::Responses && threshold > 0 {
            object.insert(
                "context_management".into(),
                json!([{"type":"compaction","compact_threshold":threshold}]),
            );
        }
    }
    object.remove("thinking_budget");
    object.remove("thinking");
    let effort = object
        .remove("reasoning_effort")
        .and_then(|v| v.as_str().map(str::to_owned));
    if !object.contains_key("reasoning") {
        if let Some(effort) = effort.filter(|v| !v.trim().is_empty()) {
            let mut effort = effort.trim().to_owned();
            if protocol == Protocol::Responses {
                effort = effort.to_lowercase();
                if effort == "none" && !supports_none(&request.model) {
                    effort = "minimal".into();
                }
            }
            object.insert("reasoning".into(), json!({"effort":effort}));
        } else if budget > 0 {
            object.insert(
                "reasoning".into(),
                if protocol == Protocol::Responses {
                    json!({"effort":budget_effort(budget)})
                } else {
                    json!({"max_tokens":budget})
                },
            );
        }
    }
    match protocol {
        Protocol::Responses => {
            let maximum = object.remove("max_tokens").unwrap_or(json!(16384));
            object.entry("max_output_tokens").or_insert(maximum);
            object.entry("truncation").or_insert(json!("auto"));
            object
                .entry("prompt_cache_retention")
                .or_insert(json!("24h"));
            if let Some(reasoning) = object.get_mut("reasoning") {
                let reasoning = reasoning
                    .as_object_mut()
                    .ok_or_else(|| crate::invalid("reasoning must be an object"))?;
                if !matches!(
                    reasoning.get("effort").and_then(Value::as_str),
                    Some("none" | "minimal")
                ) {
                    reasoning.entry("summary").or_insert(json!("auto"));
                }
                let include = object
                    .entry("include")
                    .or_insert(json!([]))
                    .as_array_mut()
                    .ok_or_else(|| crate::invalid("include must be an array"))?;
                if !include
                    .iter()
                    .any(|value| value == "reasoning.encrypted_content")
                {
                    include.push(json!("reasoning.encrypted_content"));
                }
            }
            if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
                for tool in tools.iter_mut() {
                    tool["strict"] = false.into();
                }
                if !tools.is_empty() {
                    object.entry("parallel_tool_calls").or_insert(json!(true));
                }
            }
        }
        Protocol::Chat => {
            if !object.contains_key("max_completion_tokens") {
                object.entry("max_tokens").or_insert(json!(16384));
            }
        }
        Protocol::Anthropic => unreachable!("Messages has its own shaping"),
    }
    Ok(())
}

pub(crate) fn codex(body: &mut Value) {
    body["store"] = false.into();
    body["stream"] = true.into();
    for field in ["max_output_tokens", "truncation", "prompt_cache_retention"] {
        body.as_object_mut()
            .expect("Responses object")
            .remove(field);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_omits_unsupported_budget_fields_without_losing_continuation() {
        let mut body = json!({"model":"gpt-5.6","stream":false,"store":true,"max_output_tokens":42,"truncation":"auto","prompt_cache_retention":"24h","include":["reasoning.encrypted_content"],"reasoning":{"effort":"max","summary":"auto"}});
        codex(&mut body);
        assert_eq!(
            body,
            json!({"model":"gpt-5.6","stream":true,"store":false,"include":["reasoning.encrypted_content"],"reasoning":{"effort":"max","summary":"auto"}})
        );
    }
}
