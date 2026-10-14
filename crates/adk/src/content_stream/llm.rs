use super::ContentStream;
use adk_codec::dto::ContentEvent;

/// The SDK's six-field LLM attempt input. Use `ContentEvent` for richer metrics.
#[derive(Clone, Debug, Default)]
pub struct LlmAttempt {
    pub attempt: i32,
    pub scope: String,
    pub agent_name: String,
    pub model: String,
    pub task_id: String,
    pub status: String,
}

impl From<LlmAttempt> for ContentEvent {
    fn from(attempt: LlmAttempt) -> Self {
        Self {
            llm_attempt: attempt.attempt,
            attempt_number: attempt.attempt,
            llm_scope: attempt.scope.clone(),
            scope: attempt.scope,
            agent_name: attempt.agent_name,
            model: attempt.model.clone(),
            canonical_model: attempt.model,
            task_id: attempt.task_id,
            status: attempt.status.clone(),
            attempt_status: attempt.status,
            ..Default::default()
        }
    }
}

impl ContentStream {
    /// Emit either a six-field `LlmAttempt` or a full `ContentEvent`.
    /// Only empty legacy aliases are filled; conflicting values remain distinct.
    pub fn emit_llm_attempt(&self, attempt: impl Into<ContentEvent>) {
        let mut event = attempt.into();
        event.kind = "llm_attempt".into();
        if event.llm_attempt == 0 {
            event.llm_attempt = event.attempt_number;
        }
        if event.llm_scope.is_empty() {
            event.llm_scope.clone_from(&event.scope);
        }
        if event.status.is_empty() {
            event.status.clone_from(&event.attempt_status);
        }
        if event.model.is_empty() {
            event.model.clone_from(&event.canonical_model);
        }
        self.emit(event);
    }
}
