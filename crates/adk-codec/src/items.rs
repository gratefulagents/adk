use crate::dto::{RunItem, RunItemType, ToolCallData};

pub fn extract_text(items: &[RunItem]) -> String {
    let mut text = String::new();
    for message in items
        .iter()
        .filter(|item| item.kind == RunItemType::MESSAGE)
        .filter_map(|item| item.message.as_ref())
    {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&message.text);
    }
    text
}

pub fn extract_last_text(items: &[RunItem]) -> &str {
    items
        .iter()
        .rev()
        .filter(|item| item.kind == RunItemType::MESSAGE)
        .find_map(|item| item.message.as_ref())
        .map(|message| message.text.as_str())
        .unwrap_or("")
}

pub fn extract_tool_calls(items: &[RunItem]) -> Option<Vec<ToolCallData>> {
    let calls: Vec<_> = items
        .iter()
        .filter(|item| item.kind == RunItemType::TOOL_CALL)
        .filter_map(|item| item.tool_call.clone())
        .collect();
    (!calls.is_empty()).then_some(calls)
}
