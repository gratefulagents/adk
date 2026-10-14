use super::ContentStream;
use adk_codec::dto::ContentEvent;

#[derive(Clone, Debug, Default)]
pub struct SubagentStarted {
    pub task_id: String,
    pub tool_use_id: String,
    pub description: String,
    pub subagent_type: String,
    pub model: String,
    pub prompt: String,
}

/// Completion metrics; the caller supplies the status rather than changing scheduler state.
/// Zero cache counts reproduce the SDK's completion helper without usage data.
#[derive(Clone, Debug, Default)]
pub struct SubagentCompleted {
    pub task_id: String,
    pub status: String,
    pub description: String,
    pub tool_count: i32,
    pub tokens: i64,
    pub duration_ms: i64,
    pub cost_usd: f64,
    pub cost_known: bool,
    pub num_turns: i32,
    pub cache_read_tokens: i64,
    pub cache_create_tokens: i64,
    pub stop_reason: String,
    pub result_text: String,
}

/// An owned projection of the SDK task fields used for status emission.
///
/// This is not an automatic conversion from the native runtime's task snapshot.
/// Counts remain signed; duration is stored signed nanoseconds, not a live clock.
/// Only `completed`, `failed`, and `cancelled` enable terminal result fields.
#[derive(Clone, Debug, Default)]
pub struct SubagentTaskStatus {
    pub task_id: String,
    pub agent_name: String,
    pub status: String,
    pub task_message: String,
    pub message: String,
    pub depends_on: Vec<String>,
    pub waiting_on: Vec<String>,
    pub current_step: String,
    pub last_tool: String,
    pub files_written: i64,
    pub messages_received: i64,
    pub last_parent_message: String,
    pub tool_count: i32,
    pub tokens: i64,
    pub duration_nanos: i64,
    /// The task result, not its separate error field.
    pub result: String,
}

impl ContentStream {
    pub fn emit_subagent_started(&self, started: SubagentStarted) {
        self.emit(ContentEvent {
            kind: "subagent_status".into(),
            task_id: started.task_id,
            tool_use_id: started.tool_use_id,
            status: "started".into(),
            message: started.description,
            agent_name: started.subagent_type.clone(),
            subagent_type: started.subagent_type,
            subagent_model: started.model,
            subagent_prompt: truncate_text(&started.prompt),
            ..Default::default()
        });
    }

    pub fn emit_subagent_completed(&self, completed: SubagentCompleted) {
        self.emit(ContentEvent {
            kind: "subagent_status".into(),
            task_id: completed.task_id,
            status: completed.status,
            message: completed.description,
            subagent_tool_count: completed.tool_count,
            subagent_tokens: completed.tokens,
            subagent_duration_ms: completed.duration_ms,
            subagent_cost_usd: completed.cost_usd,
            subagent_cost_known: completed.cost_known,
            subagent_num_turns: completed.num_turns,
            subagent_cache_read_tokens: completed.cache_read_tokens,
            subagent_cache_create_tokens: completed.cache_create_tokens,
            subagent_stop_reason: completed.stop_reason,
            subagent_result_text: truncate_text(&completed.result_text),
            ..Default::default()
        });
    }

    pub fn emit_subagent_task_status(&self, task: SubagentTaskStatus) {
        let terminal = matches!(task.status.as_str(), "completed" | "failed" | "cancelled");
        let mut event = ContentEvent {
            kind: "subagent_status".into(),
            task_id: task.task_id,
            message: if task.message.is_empty() {
                task.status.clone()
            } else {
                task.message
            },
            status: task.status,
            agent_name: task.agent_name.clone(),
            subagent_type: task.agent_name,
            subagent_prompt: truncate_text(&task.task_message),
            subagent_depends_on: task.depends_on,
            subagent_waiting_on: task.waiting_on,
            subagent_current_step: task.current_step,
            subagent_last_tool: task.last_tool,
            subagent_files_written: task.files_written,
            subagent_messages_received: task.messages_received,
            subagent_last_parent_message: task.last_parent_message,
            subagent_tool_count: task.tool_count,
            subagent_tokens: task.tokens,
            subagent_duration_ms: task.duration_nanos / 1_000_000,
            ..Default::default()
        };
        if terminal {
            event.subagent_stop_reason.clone_from(&event.status);
            event.subagent_result_text = truncate_text(&task.result);
        }
        self.emit(event);
    }
}

fn truncate_text(text: &str) -> String {
    // Pin the SDK's Unicode whitespace set rather than compiler Unicode tables.
    let mut text = text
        .trim_matches(|ch| {
            matches!(ch,
                '\u{9}'..='\u{d}' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}' |
                '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' |
                '\u{205f}' | '\u{3000}'
            )
        })
        .replace('\n', " ");
    if text.chars().take(4097).count() > 4096 {
        let end = text
            .char_indices()
            .nth(4093)
            .expect("more than 4096 scalars")
            .0;
        text.truncate(end);
        text.push_str("...");
    }
    text
}
