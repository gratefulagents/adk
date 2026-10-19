use super::*;
use adk_core::ModelResponse;

const INSTRUCTIONS: &str = r#"You are compacting the working history of an AI coding agent mid-task. The transcript segment below is about to be deleted from the agent's context; your summary is the ONLY memory of it the agent will keep. Write a dense continuation brief so the agent can resume seamlessly.

Preserve, in this order:
1. Task & intent: what the user asked for, including exact constraints and any follow-up corrections.
2. Findings: what was learned about the codebase/system (key files, symbols, behaviors, root causes), with concrete paths and line references where present.
3. Decisions: design/implementation decisions made and the reasons behind them.
4. Actions & state: files created/edited (paths + what changed), commands run and their outcomes, tests passing/failing, commits made.
5. In-progress work: exactly what was being done last and the planned next steps.
6. Pitfalls: errors hit, dead ends explored, approaches ruled out (so they are not retried).

Rules:
- Be specific: real paths, symbol names, numbers, error strings. Never write vague fillers like "several files were inspected".
- Use terse bullets grouped under the headings above; omit headings with nothing to say.
- Do not mention the summarization process itself. Output only the brief."#;

impl LocalCompactionPlan {
    pub(crate) fn summary_request(&self, model: &str) -> Option<ModelRequest> {
        let transcript = flatten_transcript(self.removed.iter().map(|i| &self.source[*i]), 240_000);
        if transcript.trim().is_empty() {
            return None;
        }
        Some(ModelRequest {
            model: model.into(),
            instructions: INSTRUCTIONS.into(),
            input: vec![RunItem::Message {
                message: Message {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: format!("Transcript segment to summarize:\n\n{transcript}"),
                    }],
                },
            }],
            input_provenance: vec![ItemProvenance::Unattributed],
            tools: Vec::new(),
            output_schema: None,
            output_schema_name: String::new(),
            output_schema_strict: false,
            settings: serde_json::from_value(
                serde_json::json!({"max_tokens": 2048, "reasoning_effort": "low"}),
            )
            .unwrap(),
        })
    }

    pub(crate) fn apply_summary(&mut self, response: &ModelResponse) -> bool {
        let body = summary_body(response);
        if body.is_empty() {
            return false;
        }
        let removed = self
            .removed
            .iter()
            .map(|i| self.source[*i].clone())
            .collect::<Vec<_>>();
        let summary = format!("{SUMMARY_MARKER}\n{}\n{body}", scope(&removed));
        let rebuilt = rebuild_attributed(
            &self.source,
            &self.protected,
            &summary,
            ItemProvenance::Agent {
                name: "context-summary".into(),
            },
        );
        let after = estimate_mixed_tokens(&rebuilt);
        let before = estimate_mixed_tokens(&self.source);
        if after >= before {
            return false;
        }
        let (history, markers, history_provenance) = split_history(rebuilt);
        self.outcome.history = history;
        self.outcome.markers = markers;
        self.outcome.history_provenance = history_provenance;
        self.outcome.after_tokens = self.outcome.before_tokens - before + after;
        true
    }
}

fn summary_body(response: &ModelResponse) -> String {
    let body = response
        .items
        .iter()
        .filter_map(|item| match item {
            RunItem::Message { message } | RunItem::PhasedMessage { message, .. } => {
                let text = content_text(&message.content);
                let text = text.trim();
                (!text.is_empty()).then(|| text.to_owned())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let body = body
        .trim()
        .strip_prefix(SUMMARY_MARKER)
        .unwrap_or(body.trim())
        .trim();
    body.to_owned()
}

fn truncate_transcript(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.len() <= max {
        return text.into();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} …[truncated]", &text[..end])
}

fn flatten_transcript<'a>(items: impl Iterator<Item = &'a HistoryItem>, max: usize) -> String {
    let mut transcript = String::new();
    for item in items {
        match item {
            HistoryItem::Native(
                RunItem::Message { message } | RunItem::PhasedMessage { message, .. },
                source,
            ) => {
                let text = content_text(&message.content);
                if text.trim().is_empty() {
                    continue;
                }
                let prior_summary = text.trim().starts_with(SUMMARY_MARKER)
                    && matches!(source, ItemProvenance::Agent { name } if name == "context-summary");
                let role = match source {
                    ItemProvenance::Agent { .. } => "assistant",
                    ItemProvenance::Unattributed => "user",
                    ItemProvenance::Unknown if message.role == Role::Assistant => "assistant",
                    ItemProvenance::Unknown => "user",
                };
                let limit = if prior_summary { 16_000 } else { 2_000 };
                transcript.push_str(&format!(
                    "[{role}] {}\n\n",
                    truncate_transcript(&text, limit)
                ));
            }
            HistoryItem::Native(RunItem::Reasoning { reasoning }, _)
                if !reasoning.text.trim().is_empty() =>
            {
                transcript.push_str(&format!(
                    "[thinking] {}\n\n",
                    truncate_transcript(&reasoning.text, 1_500)
                ));
            }
            HistoryItem::Native(RunItem::ToolCall { call }, _) => {
                transcript.push_str(&format!(
                    "[tool_call] {} {}\n",
                    call.name,
                    truncate_transcript(&arguments(call), 300)
                ));
            }
            HistoryItem::Native(RunItem::ToolResult { output, .. }, _) => {
                let (label, limit) = if output.is_error {
                    ("tool_error", 1_000)
                } else {
                    ("tool_result", 700)
                };
                transcript.push_str(&format!(
                    "[{label}] {}\n\n",
                    truncate_transcript(&content_text(&output.content), limit)
                ));
            }
            _ => {}
        }
    }
    if max > 0 && transcript.len() > max {
        let mut head = max / 3;
        while !transcript.is_char_boundary(head) {
            head -= 1;
        }
        let mut tail = transcript.len() - (max - max / 3);
        while !transcript.is_char_boundary(tail) {
            tail += 1;
        }
        transcript = format!(
            "{}\n\n[... transcript truncated ...]\n\n{}",
            &transcript[..head],
            &transcript[tail..]
        );
    }
    transcript
}

#[cfg(test)]
mod tests;
