use super::{AgentMeta, HookEvent, HookResult, ProgressTracker, state::truncate};
use crate::tracewriter::{Session, Span, SpanData, Subagent};
use adk_codec::dto::Usage;

fn session_summary(number: i32, cost_usd: f64, num_turns: i64) -> String {
    let cost = if cost_usd.is_nan() {
        "NaN".into()
    } else if cost_usd == f64::INFINITY {
        "+Inf".into()
    } else if cost_usd == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        format!("{cost_usd:.4}")
    };
    format!("Session {number} complete: ${cost}, {num_turns} turns")
}

impl ProgressTracker {
    pub fn record_system_init(
        &self,
        _model: &str,
        _cwd: &str,
        _tools: &[String],
        _max_turns: i64,
        _mcp_servers: &[String],
        _permission_mode: &str,
    ) {
        let mut state = self.lock();
        state.snapshot.current_step = "starting".into();
        state.event("step_change", "Agent session initialized".into(), false);
    }

    pub fn record_assistant_text(&self, text: &str) {
        let mut state = self.lock();
        state.last_assistant_text = text.into();
        state.snapshot.last_activity = truncate(text, 120);
    }

    pub fn record_assistant_thinking(&self, thinking: &str) {
        self.lock().last_assistant_thinking = thinking.into();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_tool_use(
        &self,
        tool_name: &str,
        input_summary: &str,
        _tool_use_id: &str,
        _turn: i64,
        _raw_input: &str,
        _agent_name: &str,
        _parent_call_id: &str,
    ) {
        let mut tracker = Some(self.clone());
        let mut local = true;
        while let Some(current) = tracker {
            let mut state = current.lock();
            state.snapshot.tool_call_count = state.snapshot.tool_call_count.wrapping_add(1);
            state.session_tool_call_count = state.session_tool_call_count.wrapping_add(1);
            if local {
                state.snapshot.last_activity = format!("Tool: {tool_name}");
                state.infer_step(tool_name, input_summary);
                local = false;
            }
            tracker = state.parent.clone();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_tool_result(
        &self,
        _tool_use_id: &str,
        tool_name: &str,
        output: &str,
        is_error: bool,
        _duration_ms: i64,
        _agent_name: &str,
        _parent_call_id: &str,
    ) {
        let status = if is_error { "failed" } else { "completed" };
        let mut summary = format!("Tool {tool_name} {status}");
        if !output.is_empty() {
            summary.push_str(": ");
            summary.push_str(&truncate(output, 120));
        }
        let mut state = self.lock();
        state.snapshot.last_activity = summary.clone();
        state.event("tool_result", summary, false);
    }

    pub fn record_lifecycle_event(&self, event_type: &str, message: &str) {
        let mut state = self.lock();
        if !message.is_empty() {
            state.snapshot.last_activity = truncate(message, 120);
        }
        state.event(
            if event_type.is_empty() {
                "lifecycle"
            } else {
                event_type
            },
            message.into(),
            false,
        );
    }

    pub fn record_hook_decision(&self, _event: &HookEvent, _result: &HookResult) {}

    pub fn record_llm_usage(&self, model: &str, cost_usd: f64, usage: &Usage) {
        let mut tracker = Some(self.clone());
        while let Some(current) = tracker {
            let mut state = current.lock();
            state.session_cost_usd += cost_usd;
            let snapshot = &mut state.snapshot;
            snapshot.cost_usd += cost_usd;
            snapshot.input_tokens = snapshot.input_tokens.wrapping_add(usage.input_tokens);
            snapshot.output_tokens = snapshot.output_tokens.wrapping_add(usage.output_tokens);
            snapshot.cache_read_input_tokens = snapshot
                .cache_read_input_tokens
                .wrapping_add(usage.cache_read_tokens);
            snapshot.cache_creation_input_tokens = snapshot
                .cache_creation_input_tokens
                .wrapping_add(usage.cache_create_tokens);
            let entry = snapshot
                .model_usage
                .get_or_insert_default()
                .entry(model.into())
                .or_default();
            entry.input_tokens = entry.input_tokens.wrapping_add(usage.input_tokens);
            entry.output_tokens = entry.output_tokens.wrapping_add(usage.output_tokens);
            entry.cache_read_input_tokens = entry
                .cache_read_input_tokens
                .wrapping_add(usage.cache_read_tokens);
            entry.cache_creation_input_tokens = entry
                .cache_creation_input_tokens
                .wrapping_add(usage.cache_create_tokens);
            tracker = state.parent.clone();
        }
    }

    pub fn record_session_complete(
        &self,
        model: &str,
        cost_usd: f64,
        num_turns: i64,
        usage: &Usage,
        duration_ms: i64,
        stop_reason: &str,
    ) {
        {
            let mut state = self.lock();
            let number = state.snapshot.session_number;
            state.event(
                "session_complete",
                session_summary(number, cost_usd, num_turns),
                true,
            );
            state.instant_span(
                "session",
                SpanData::Session(Session {
                    model: model.into(),
                    cost_usd,
                    num_turns,
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_read_input_tokens: usage.cache_read_tokens,
                    cache_creation_input_tokens: usage.cache_create_tokens,
                    stop_reason: stop_reason.into(),
                    duration_ms,
                }),
            );
        }
        self.dispatch();
    }

    /// The runtime adapter's external span owner still needs the status event.
    pub(super) fn record_session_complete_event(&self, cost_usd: f64, num_turns: i64) {
        let mut state = self.lock();
        let number = state.snapshot.session_number;
        state.event(
            "session_complete",
            session_summary(number, cost_usd, num_turns),
            true,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_subagent_started(
        &self,
        task_id: &str,
        _tool_use_id: &str,
        description: &str,
        subagent_type: &str,
        model: &str,
        isolation: &str,
        prompt: &str,
    ) {
        {
            let mut state = self.lock();
            state.snapshot.agent_count = state.snapshot.agent_count.wrapping_add(1);
            state.task_agents.insert(
                task_id.into(),
                AgentMeta {
                    description: description.into(),
                    prompt: prompt.into(),
                    subagent_type: subagent_type.into(),
                    model: model.into(),
                    isolation: isolation.into(),
                },
            );
            state.event(
                "agent_spawn",
                format!("Spawned agent [{subagent_type}]: {description}"),
                false,
            );
            if state.processor.is_some() {
                let span = Span::new(
                    format!("subagent.{subagent_type}"),
                    &state.root_span_id,
                    Some(SpanData::Subagent(Box::new(Subagent {
                        task_id: task_id.into(),
                        subagent_type: subagent_type.into(),
                        description: description.into(),
                        model: model.into(),
                        status: "initializing".into(),
                        ..Default::default()
                    }))),
                );
                state.notify(span.clone(), true);
                state.subagent_spans.insert(task_id.into(), span);
            }
        }
        self.dispatch();
    }

    pub fn record_subagent_progress(
        &self,
        task_id: &str,
        tool_count: i32,
        total_tokens: i64,
        _duration_ms: i64,
        last_tool_name: &str,
    ) {
        let mut summary =
            format!("Subagent {task_id} progress: {tool_count} tools, {total_tokens} tokens");
        if !last_tool_name.is_empty() {
            summary.push_str(&format!(", last tool {last_tool_name}"));
        }
        let mut state = self.lock();
        state.snapshot.last_activity = summary.clone();
        state.event("subagent_progress", summary, false);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_subagent_completed(
        &self,
        task_id: &str,
        status: &str,
        summary: &str,
        cost_usd: f64,
        num_turns: i64,
        usage: &Usage,
        stop_reason: &str,
        files_read: &[String],
        files_written: &[String],
    ) {
        {
            let mut state = self.lock();
            let meta = state.task_agents.get(task_id).cloned().unwrap_or_default();
            state.snapshot.last_activity = truncate(summary, 120);
            // Usage is already forwarded live; completion metrics belong only to the span.
            if state.processor.is_some() {
                let mut data = Subagent {
                    task_id: task_id.into(),
                    subagent_type: meta.subagent_type,
                    description: meta.description,
                    model: meta.model,
                    status: status.into(),
                    cost_usd,
                    num_turns,
                    total_tokens: usage.total_tokens(),
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_read_tokens: usage.cache_read_tokens,
                    cache_creation_tokens: usage.cache_create_tokens,
                    stop_reason: stop_reason.into(),
                    isolation: meta.isolation,
                    prompt: meta.prompt,
                    result_text: truncate(summary, 4096),
                    files_read: files_read.into(),
                    files_written: files_written.into(),
                    ..Default::default()
                };
                let mut span = match state.subagent_spans.remove(task_id) {
                    Some(span) => span,
                    None => {
                        let span = Span::new(
                            format!("subagent.{}", data.subagent_type),
                            &state.root_span_id,
                            Some(SpanData::Subagent(Box::new(data.clone()))),
                        );
                        state.notify(span.clone(), true);
                        span
                    }
                };
                data.duration_ms = span.duration_ms();
                span.data = Some(SpanData::Subagent(Box::new(data)));
                span.finish();
                state.notify(span, false);
            }
            if matches!(status, "completed" | "failed" | "stopped") {
                state.task_agents.remove(task_id);
            }
        }
        self.dispatch();
    }

    pub fn record_api_retry(
        &self,
        error_code: &str,
        retry_after_ms: i64,
        attempt: i32,
        max_retries: i32,
    ) {
        {
            let mut state = self.lock();
            state.snapshot.api_retries = state.snapshot.api_retries.wrapping_add(1);
            state.instant_span(
                "api.retry",
                SpanData::Retry {
                    error_code: error_code.into(),
                    retry_after_ms,
                    attempt: i64::from(attempt),
                    max_retries: i64::from(max_retries),
                },
            );
        }
        self.dispatch();
    }

    pub fn record_compact_boundary(&self, tokens_before: i64, tokens_after: i64, _trigger: &str) {
        self.lock().instant_span(
            "compaction",
            SpanData::Compaction {
                tokens_before,
                tokens_after,
            },
        );
        self.dispatch();
    }
}
