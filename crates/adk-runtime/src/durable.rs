//! Crash recovery is deliberately narrower than in-process continuation.
//! Stores must atomically commit each checkpoint under a fenced lease and CAS.
//! Host event delivery is observational, not exactly-once. Tools are conservatively
//! non-replayable; an interrupted dispatch always requires operator reconciliation.

use super::*;
use adk_durable::{AttemptId, Effect, EffectClassification, EffectState, RunId, StepId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Go's runner checkpoint envelope (schema 1), with a versioned Rust continuation.
/// Go snapshots lack the execution policy and complete budget/continuation state;
/// decoding one is supported, but resuming it requires explicit migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerCheckpoint {
    pub schema_version: u32,
    pub run_id: String,
    pub attempt_id: String,
    pub step_id: String,
    pub sequence: u64,
    pub boundary: String,
    pub agent_name: String,
    #[serde(default)]
    #[serde(deserialize_with = "null_history")]
    pub history: Vec<adk_codec::dto::RunItemSnapshot>,
    #[serde(default)]
    pub interruptions: Option<Value>,
    pub usage: adk_codec::dto::Usage,
    #[serde(default)]
    pub children: Option<Value>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeCheckpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
}

fn null_history<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<adk_codec::dto::RunItemSnapshot>, D::Error> {
    Ok(Option::<Vec<_>>::deserialize(d)?.unwrap_or_default())
}

impl RunnerCheckpoint {
    pub fn execution_boundary(&self) -> &str {
        self.runtime
            .as_ref()
            .map_or(&self.boundary, |state| &state.boundary)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let value: Value = serde_json::from_slice(bytes).map_err(invalid)?;
        if value.get("schema_version").and_then(Value::as_u64) != Some(1) {
            return Err(unsupported("unknown runner checkpoint schema"));
        }
        let checkpoint: Self = serde_json::from_value(value).map_err(invalid)?;
        if checkpoint
            .runtime
            .as_ref()
            .is_some_and(|state| state.version != 1)
        {
            return Err(unsupported("unknown runtime continuation schema"));
        }
        Ok(checkpoint)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCheckpoint {
    version: u32,
    boundary: String,
    started_at: DateTime<Utc>,
    deadline_at: Option<DateTime<Utc>>,
    fingerprint: String,
    result: RunResult,
    policy: RunPolicy,
    #[serde(default)]
    base_turn_limit: Option<std::num::NonZeroU32>,
    #[serde(default)]
    stop_gate_blocks: usize,
    phase: Phase,
    calls: VecDeque<ToolCall>,
    turns: u32,
    cost: f64,
    tool_calls: u64,
    tool_pause: bool,
    tool_final: Option<String>,
    tool_turn_start: Option<usize>,
    consecutive_tool_errors: usize,
    tool_error_escalated: bool,
    #[serde(default)]
    applied_child_messages: HashSet<String>,
    #[serde(default)]
    child_deliveries: Vec<String>,
    approval_journal_present: bool,
    #[serde(default)]
    approval_journal: Vec<crate::compat::ApprovalJournalEntry>,
}
impl RuntimeCheckpoint {
    pub fn wall_time_ms(&self, now: DateTime<Utc>) -> i64 {
        (now - self.started_at).num_milliseconds().max(0)
    }
    pub fn turns(&self) -> u32 {
        self.turns
    }
    pub fn cost(&self) -> f64 {
        self.cost
    }
    pub fn tool_calls(&self) -> u64 {
        self.tool_calls
    }
    pub fn child_deliveries(&self) -> Vec<String> {
        self.child_deliveries.clone()
    }
    pub fn applied_child_messages(&self) -> Vec<String> {
        self.applied_child_messages.iter().cloned().collect()
    }
    pub fn result(&self) -> &RunResult {
        &self.result
    }
}

/// Acknowledging a write means the entire checkpoint is durable. Implementations
/// must reject stale writers (lease fencing + revision CAS), not last-write-win.
/// An ambiguous write failure must not be retried under the same session.
pub trait CheckpointStore: Send + Sync {
    fn persist<'a>(
        &'a self,
        context: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>>;
}

/// Host-owned scheduler state. Restore must not dispatch children; active records
/// arrive as reconciling and need explicit child-worker/operator resolution.
/// Implementations must preserve delivery, security and steering-message state.
pub trait ChildCheckpointOwner: Send + Sync {
    fn restore<'a>(
        &'a self,
        context: &'a Context,
        checkpoint: Value,
    ) -> BoxFuture<'a, Result<(), Error>>;
    fn checkpoint<'a>(&'a self, context: &'a Context) -> BoxFuture<'a, Result<Value, Error>>;
}

pub struct DurableRun {
    pub store: Arc<dyn CheckpointStore>,
    /// New attempts must have a distinct ID; effects retain their original key.
    pub attempt_id: String,
    pub resume: Option<RunnerCheckpoint>,
    pub children: Option<Arc<dyn ChildCheckpointOwner>>,
}
impl DurableRun {
    pub fn new(store: Arc<dyn CheckpointStore>) -> Self {
        Self {
            store,
            attempt_id: AttemptId::new().to_string(),
            resume: None,
            children: None,
        }
    }
}

pub(super) struct DurableState {
    children: Option<Arc<dyn ChildCheckpointOwner>>,
    child_checkpoint: Option<Value>,
    store: Arc<dyn CheckpointStore>,
    started_at: DateTime<Utc>,
    deadline_at: Option<DateTime<Utc>>,
    attempt_id: String,
    sequence: u64,
    step_id: String,
    fingerprint: String,
    pub(super) effect: Option<Effect>,
    tool_calls: u64,
}

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::new(ErrorCategory::InvalidInput, error.to_string())
}
fn unsupported(message: &str) -> Error {
    Error::new(ErrorCategory::Unsupported, message)
}

fn reconcile_children(mut children: Value) -> Result<Value, Error> {
    if children.is_null() {
        return Ok(children);
    }
    if children.get("records").is_some_and(Value::is_null) {
        return Ok(children);
    }
    let records = children
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("invalid child records"))?;
    let mut ids = HashSet::new();
    for record in records {
        let task = record
            .get_mut("task")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid("missing child task"))?;
        let id = task
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| invalid("missing child task ID"))?;
        if !ids.insert(id.to_owned()) {
            return Err(invalid("duplicate child task ID"));
        }
        match task.get("status").and_then(Value::as_str) {
            Some("completed" | "failed" | "cancelled") => {}
            Some("pending" | "waiting" | "running" | "reconciling") => {
                task.insert("status".into(), Value::String("reconciling".into()));
                task.insert("error".into(), Value::String("sub-agent runtime restarted while this task was active; durable reconciliation is required".into()));
                task.remove("waiting_on");
            }
            _ => return Err(unsupported("unknown child task status")),
        }
    }
    Ok(children)
}

fn has_active_children(children: Option<&Value>) -> bool {
    let Some(children) = children.filter(|v| !v.is_null()) else {
        return false;
    };
    let Some(records) = children.get("records") else {
        return true;
    };
    if records.is_null() {
        return false;
    }
    records.as_array().is_none_or(|records| {
        records.iter().any(|record| {
            !matches!(
                record.pointer("/task/status").and_then(Value::as_str),
                Some("completed" | "failed" | "cancelled")
            )
        })
    })
}

impl Runner {
    /// Run or recover a durable invocation. Recovery never accepts extra input.
    /// Deferred approvals never authorize dispatch merely by reopening a checkpoint.
    pub async fn run_durable(
        &self,
        context: Context,
        request: RunRequest,
        host: Arc<dyn Host>,
        durable: DurableRun,
    ) -> Result<RunOutcome, RunError> {
        self.drive_durable(context, request, host, durable, None, None)
            .await
    }

    /// Lazy durable streaming; dropping the stream leaves dispatched effects unresolved.
    pub fn stream_durable(
        &self,
        context: Context,
        request: RunRequest,
        host: Arc<dyn Host>,
        durable: DurableRun,
    ) -> RunStream {
        let runner = self.clone();
        let (sender, receiver) = mpsc::channel(1);
        RunStream {
            producer: Some(Box::pin(async move {
                runner
                    .drive_durable(context, request, host, durable, Some(sender), None)
                    .await
            })),
            receiver,
            outcome: None,
        }
    }

    pub(super) async fn drive_durable(
        &self,
        context: Context,
        request: RunRequest,
        host: Arc<dyn Host>,
        durable: DurableRun,
        sender: Option<mpsc::Sender<RunEvent>>,
        child_control: Option<crate::subagent::ChildControl>,
    ) -> Result<RunOutcome, RunError> {
        if context.run_id.is_empty() || durable.attempt_id.is_empty() {
            return Err(invalid("durable run and attempt IDs must be nonempty").into());
        }
        if child_control.is_none()
            && self
                .config
                .subagents
                .as_ref()
                .is_some_and(|session| !session.scheduler.is_durable())
        {
            return Err(unsupported(
                "native durable child sessions require a durable scheduler store",
            )
            .into());
        }
        let fingerprint = self.durable_fingerprint()?;
        let mut engine = self.engine(context, request, host);
        engine.streaming = sender.is_some() || child_control.is_some();
        engine.child_control = child_control;
        engine.sender = sender;
        let now = Utc::now();
        let deadline_at = engine.context.deadline.map(|deadline| {
            now + chrono::Duration::from_std(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or(chrono::Duration::MAX)
        });
        let mut state = DurableState {
            children: durable.children.or_else(|| {
                self.config
                    .subagents
                    .as_ref()
                    .filter(|_| engine.child_control.is_none())
                    .map(|session| {
                        Arc::new(session.scheduler.clone()) as Arc<dyn ChildCheckpointOwner>
                    })
            }),
            child_checkpoint: None,
            started_at: now,
            deadline_at,
            store: durable.store,
            attempt_id: durable.attempt_id,
            sequence: 0,
            step_id: StepId::new().to_string(),
            fingerprint,
            effect: None,
            tool_calls: 0,
        };
        if let Some(checkpoint) = durable.resume {
            if checkpoint.schema_version != 1 {
                return Err(unsupported("unknown runner checkpoint schema").into());
            }
            if checkpoint.run_id != engine.context.run_id
                || checkpoint.attempt_id == state.attempt_id
            {
                return Err(
                    invalid("recovery requires the same run ID and a new attempt ID").into(),
                );
            }
            if has_active_children(checkpoint.children.as_ref()) && state.children.is_none() {
                return Err(
                    unsupported("active child checkpoints require a scheduler owner").into(),
                );
            }
            if let Some(children) = checkpoint.children.clone() {
                reconcile_children(children)?;
            }
            state.child_checkpoint = checkpoint.children.clone();
            if checkpoint
                .interruptions
                .as_ref()
                .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|v| !v.is_empty()))
            {
                return Err(
                    unsupported("approval interruptions require explicit reconciliation").into(),
                );
            }
            if !engine.result.history.is_empty() {
                return Err(invalid("recovery does not accept additional input").into());
            }
            let execution_boundary = checkpoint.execution_boundary().to_owned();
            let mut saved = checkpoint.runtime.ok_or_else(|| unsupported(
                "Go checkpoint requires migration: missing policy, cumulative turns/cost and exact continuation"))?;
            if saved.version != 1 {
                return Err(unsupported("unknown runtime continuation schema").into());
            }
            if engine.child_control.is_some() {
                let previous = crate::subagent::SecurityBaseline {
                    tools: saved.policy.tools.clone(),
                    ..Default::default()
                };
                let current = crate::subagent::SecurityBaseline {
                    tools: engine.policy.tools.clone(),
                    ..Default::default()
                };
                if !previous.allows_resume_under(&current) {
                    return Err(unsupported("child recovery cannot weaken the tool policy").into());
                }
                saved.policy.tools = engine.policy.tools.clone();
            }
            let mut original_policy = saved.policy.clone();
            original_policy.max_turns = saved.base_turn_limit.unwrap_or(saved.policy.max_turns);
            if (self.config.stop_gate.is_none() && saved.stop_gate_blocks != 0)
                || (self.config.stop_gate.is_none()
                    && self.config.subagents.is_none()
                    && saved.policy.max_turns != original_policy.max_turns)
                || saved.policy.max_turns < original_policy.max_turns
                || saved.stop_gate_blocks > self.config.stop_gate_max_blocks
                || saved.fingerprint != state.fingerprint
                || original_policy != engine.policy
            {
                return Err(
                    unsupported("durable agent configuration or security policy changed").into(),
                );
            }
            if checkpoint.sequence == 0
                || checkpoint.step_id.is_empty()
                || saved.result.last_agent.as_deref() != Some(checkpoint.agent_name.as_str())
            {
                return Err(invalid("inconsistent checkpoint identity").into());
            }
            let mut pending = HashMap::new();
            let mut seen_calls = HashSet::new();
            for item in &saved.result.history {
                match item {
                    RunItem::ToolCall { call } => {
                        if !seen_calls.insert(&call.id) {
                            return Err(invalid("duplicate persisted tool call").into());
                        }
                        pending.insert(&call.id, call);
                    }
                    RunItem::ToolResult { call_id, .. } | RunItem::Handoff { call_id, .. } => {
                        if pending.remove(call_id).is_none() {
                            return Err(invalid("orphan persisted tool result").into());
                        }
                    }
                    _ => {}
                }
            }
            if pending.len() != saved.calls.len()
                || saved
                    .calls
                    .iter()
                    .any(|call| pending.get(&call.id).is_none_or(|saved| *saved != call))
                || (!matches!(saved.phase, Phase::Tools) && !pending.is_empty())
            {
                return Err(
                    invalid("persisted tool queue does not match unresolved history").into(),
                );
            }
            if !saved.cost.is_finite() || saved.cost < 0.0 {
                return Err(invalid("invalid durable cost counter").into());
            }
            if checkpoint.effect.as_ref().is_some_and(|effect| {
                matches!(
                    effect.state,
                    EffectState::Dispatched | EffectState::OutcomeUnknown
                )
            }) {
                return Err(unsupported(
                    "operator_resolution: dispatched effect has an unknown outcome; never replayed",
                )
                .into());
            }
            if !matches!(
                execution_boundary.as_str(),
                "run_started"
                    | "model_prepared"
                    | "model_completed"
                    | "tool_prepared"
                    | "tool_completed"
                    | "handoff_completed"
                    | "run_completed"
                    | "paused"
                    | "child_changed"
                    | "approval_pending"
            ) {
                return Err(
                    unsupported("checkpoint boundary requires explicit reconciliation").into(),
                );
            }
            if saved.approval_journal_present
                && saved.approval_journal.is_empty()
                && execution_boundary != "run_completed"
            {
                return Err(unsupported(
                    "approval journal recovery requires explicit reconciliation",
                )
                .into());
            }
            let mut agents = vec![self.initial.clone()];
            let mut seen = HashSet::new();
            let mut found = None;
            while let Some(agent) = agents.pop() {
                if !seen.insert(Arc::as_ptr(&agent) as usize) {
                    continue;
                }
                if agent.name == checkpoint.agent_name {
                    found = Some(agent.clone());
                }
                agents.extend(agent.handoffs.iter().map(|h| h.target.clone()));
            }
            engine.agent =
                found.ok_or_else(|| unsupported("checkpoint agent is not registered"))?;
            engine.approval_journal = crate::compat::ApprovalJournal::restore(
                saved.approval_journal,
                saved.result.history.len(),
                saved.result.new_items.len(),
            )
            .map_err(invalid)?;
            engine.result = saved.result;
            engine.phase = if execution_boundary == "run_started" {
                Phase::Model
            } else {
                saved.phase
            };
            engine.calls = saved.calls;
            for entry in engine.approval_journal.entries() {
                let call =
                    adk_codec::approval::approval_call(&entry.marker.data).map_err(invalid)?;
                if engine.calls.iter().any(|pending| *pending == call) {
                    use adk_codec::approval::ApprovalPhase;
                    match entry.marker.phase {
                        ApprovalPhase::Approved => {
                            engine.approvals.insert(call.id, ApprovalDecision::Approve);
                        }
                        ApprovalPhase::Denied => {
                            engine.approvals.insert(call.id, ApprovalDecision::Deny);
                        }
                        ApprovalPhase::Pending => {
                            engine.approvals.remove(&call.id);
                        }
                    }
                }
            }
            engine.policy = saved.policy;
            engine.base_turn_limit = original_policy.max_turns;
            engine.stop_gate_blocks = saved.stop_gate_blocks;
            engine.turns = saved.turns;
            engine.applied_child_messages = saved.applied_child_messages;
            engine.cost = saved.cost;
            engine.tool_pause = saved.tool_pause;
            engine.tool_final = saved.tool_final;
            engine.tool_turn_start = saved.tool_turn_start;
            engine.consecutive_tool_errors = saved.consecutive_tool_errors;
            engine.tool_error_escalated = saved.tool_error_escalated;
            engine.committed_cursor = engine.result.new_items.len();
            engine.committed_markers = engine.approval_journal.entries().len();
            state.started_at = saved.started_at;
            state.deadline_at = match (state.deadline_at, saved.deadline_at) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            if let Some(deadline) = state.deadline_at {
                let remaining = (deadline - Utc::now()).to_std().unwrap_or_default();
                engine.context.deadline = Some(Instant::now() + remaining);
            }
            state.sequence = checkpoint.sequence;
            state.step_id = checkpoint.step_id;
            state.effect = checkpoint.effect;
            state.tool_calls = saved.tool_calls;
            if let (Some(owner), Some(children)) = (&state.children, &state.child_checkpoint) {
                let children = reconcile_children(children.clone())?;
                bounded(
                    &engine.context,
                    None,
                    owner.restore(&engine.context, children),
                )
                .await?;
            }
            if engine.child_control.is_some()
                && let Some(session) = &self.config.subagents
            {
                session.commit_delivery(&saved.child_deliveries).await?;
            }
            if matches!(execution_boundary.as_str(), "approval_pending" | "paused")
                && !engine.result.pending_approvals.is_empty()
            {
                engine.result.status = RunStatus::Paused;
                engine.durable_state = Some(state);
                engine.sender = None;
                return Ok(RunOutcome {
                    result: engine.result.clone(),
                    spills: vec![],
                    continuation: Some(Continuation { engine }),
                });
            }
            if execution_boundary == "run_completed" {
                if engine.result.status != RunStatus::Completed {
                    return Err(invalid("completed checkpoint has an incomplete result").into());
                }
                return Ok(RunOutcome {
                    result: engine.result,
                    spills: vec![],
                    continuation: None,
                });
            }
        }
        engine.durable_state = Some(state);
        engine.drive().await
    }

    fn durable_fingerprint(&self) -> Result<String, Error> {
        let config = &self.config;
        if config.compaction.is_some()
            || config.turn_context.is_some()
            || config
                .stop_gate
                .as_ref()
                .is_some_and(|gate| gate.durable_key().is_none_or(str::is_empty))
            || config
                .hooks
                .as_ref()
                .is_some_and(|hooks| !hooks.durable_observer())
            || config.durable.is_some()
        {
            return Err(unsupported(
                "durable execution does not support custom compaction, turn context, replay-unsafe stop gates or hooks",
            ));
        }
        let mut agents = vec![self.initial.clone()];
        let mut seen = HashSet::new();
        let mut names = HashSet::new();
        let mut catalog = vec![];
        while let Some(agent) = agents.pop() {
            if !seen.insert(Arc::as_ptr(&agent) as usize) {
                continue;
            }
            if !names.insert(agent.name.clone())
                || agent.output_parser.is_some()
                || agent
                    .hooks
                    .as_ref()
                    .is_some_and(|hooks| !hooks.durable_observer())
            {
                return Err(unsupported(
                    "durable agents require unique names and no custom parsers or hooks",
                ));
            }
            catalog.push(serde_json::json!({
                "name": agent.name, "instructions": agent.instructions, "model": agent.model.name(),
                "fallbacks": agent.fallbacks.iter().map(ModelBinding::name).collect::<Vec<_>>(),
                "settings": agent.settings, "schema": agent.output_schema,
                "schema_name": agent.output_schema_name, "strict": agent.output_schema_strict,
                "tools": agent.tools.iter().map(|t| t.definition()).collect::<Vec<_>>(),
                "handoffs": agent.handoffs.iter().map(|h| (&h.definition, &h.target.name)).collect::<Vec<_>>()
            }));
            agents.extend(agent.handoffs.iter().map(|h| h.target.clone()));
        }
        let mut baseline = serde_json::json!({
            "catalog": catalog, "work_dir": config.work_dir, "max_tokens": config.limits.max_tokens,
            "max_cost": config.limits.max_cost, "output_cap": config.output.max_bytes,
            "untrusted": config.output.untrusted, "validate": config.validate_tool_arguments,
            "approve_mutating": config.approve_mutating_tools, "cache_prefix": config.cache_prefix,
            "transient_context": config.transient_context, "return_tool_output": config.return_tool_output,
            "tool_error_limit": config.consecutive_tool_error_limit,
        });
        if config.subagents.is_some() {
            baseline["subagents"] = serde_json::json!({"version": 1});
        }
        if let Some(gate) = &config.stop_gate {
            baseline["stop_gate"] = serde_json::json!({"key":gate.durable_key(), "max_blocks":config.stop_gate_max_blocks});
        }
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&baseline).map_err(invalid)?)
        ))
    }
}

impl Engine {
    pub(super) async fn persist_boundary(
        &mut self,
        boundary: Boundary,
        pending: Option<&ToolCall>,
    ) -> Result<(), Error> {
        let Some(state) = &mut self.durable_state else {
            return Ok(());
        };
        if let Some(owner) = &state.children {
            let children = bounded(&self.context, None, owner.checkpoint(&self.context)).await?;
            reconcile_children(children.clone())?;
            state.child_checkpoint = Some(children);
        }
        let delivery_ids = self
            .config
            .subagents
            .as_ref()
            .map(|session| session.delivery_ids())
            .unwrap_or_default();
        if let Some(records) = state
            .child_checkpoint
            .as_mut()
            .and_then(|children| children.get_mut("records"))
            .and_then(Value::as_array_mut)
        {
            for record in records {
                if record
                    .pointer("/task/id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| delivery_ids.iter().any(|pending| pending == id))
                {
                    record["result_delivered"] = Value::Bool(true);
                }
            }
        }
        let name = match boundary {
            Boundary::Started => "run_started",
            Boundary::ModelPrepared => "model_prepared",
            Boundary::ModelDispatched => "model_dispatched",
            Boundary::ModelCompleted => "model_completed",
            Boundary::ToolPrepared => "tool_prepared",
            Boundary::ToolDispatched => "tool_dispatched",
            Boundary::ToolCompleted => "tool_completed",
            Boundary::ApprovalPending => "approval_pending",
            Boundary::Handoff => "handoff_completed",
            Boundary::Paused => "paused",
            Boundary::Completed => "run_completed",
        };
        if matches!(boundary, Boundary::ModelPrepared | Boundary::ToolPrepared)
            && !state
                .effect
                .as_ref()
                .is_some_and(|e| e.state == EffectState::Prepared)
        {
            state.effect = Some(Effect::new(
                &RunId::from(self.context.run_id.clone()),
                EffectClassification::NonReplayable,
                Utc::now(),
            ));
            state.step_id = StepId::new().to_string();
        }
        if matches!(
            boundary,
            Boundary::ModelDispatched | Boundary::ToolDispatched
        ) {
            let effect = state
                .effect
                .as_mut()
                .ok_or_else(|| invalid("dispatch without prepared intent"))?;
            adk_durable::transition_effect(effect, EffectState::Dispatched, Utc::now())
                .map_err(invalid)?;
            if matches!(boundary, Boundary::ToolDispatched) {
                state.tool_calls = state
                    .tool_calls
                    .checked_add(1)
                    .ok_or_else(|| invalid("tool counter overflow"))?;
            }
        }
        if matches!(
            boundary,
            Boundary::ModelCompleted | Boundary::ToolCompleted | Boundary::Handoff
        ) {
            if let Some(effect) = &mut state.effect {
                // Handoffs are local transitions, not dispatched external effects.
                if effect.state == EffectState::Prepared && matches!(boundary, Boundary::Handoff) {
                    adk_durable::transition_effect(effect, EffectState::Dispatched, Utc::now())
                        .map_err(invalid)?;
                }
                if effect.state == EffectState::Dispatched {
                    adk_durable::transition_effect(effect, EffectState::Succeeded, Utc::now())
                        .map_err(invalid)?;
                    effect.outcome = Some(serde_json::json!({"boundary": name, "call": pending}));
                }
            }
        }
        let agent = adk_codec::dto::AgentRef {
            name: self.agent.name.clone(),
        };
        let mut history = self
            .result
            .history
            .iter()
            .map(|item| {
                let provenance = match item {
                    RunItem::Message { message } | RunItem::PhasedMessage { message, .. }
                        if message.role == Role::User =>
                    {
                        None
                    }
                    _ => Some(&agent),
                };
                let projected;
                let item = match item {
                    RunItem::Handoff { call_id, agent } => {
                        projected = RunItem::ToolResult {
                            call_id: call_id.clone(),
                            output: ToolOutput {
                                content: vec![Content::Text {
                                    text: format!("Handing off to {agent}"),
                                }],
                                is_error: false,
                                should_pause: false,
                            },
                        };
                        &projected
                    }
                    RunItem::ToolResult { call_id, output } if output.should_pause => {
                        let mut output = output.clone();
                        output.should_pause = false;
                        projected = RunItem::ToolResult {
                            call_id: call_id.clone(),
                            output,
                        };
                        &projected
                    }
                    _ => item,
                };
                adk_codec::approval::encode_item(item, provenance).map_err(invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (offset, marker) in self
            .approval_journal
            .history_markers()
            .map_err(invalid)?
            .into_iter()
            .enumerate()
        {
            let index = marker
                .before_item
                .checked_add(offset)
                .filter(|i| *i <= history.len())
                .ok_or_else(|| invalid("approval marker outside history"))?;
            history.insert(index, marker.marker.to_wire().map_err(invalid)?);
        }
        let count = |n: u64| i64::try_from(n).map_err(invalid);
        let sequence = state
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("checkpoint sequence overflow"))?;
        let checkpoint = RunnerCheckpoint {
            schema_version: 1,
            run_id: self.context.run_id.clone(),
            attempt_id: state.attempt_id.clone(),
            step_id: state.step_id.clone(),
            sequence,
            // Baseline Go rejects model_completed, but accepts unknown boundaries.
            // Keep every nonterminal Rust continuation behind that refusal gate.
            boundary: if matches!(boundary, Boundary::Completed) {
                "run_completed"
            } else {
                "model_completed"
            }
            .into(),
            agent_name: self.agent.name.clone(),
            history: adk_codec::snapshot_items(&history),
            interruptions: None,
            children: state.child_checkpoint.clone(),
            usage: adk_codec::dto::Usage {
                requests: i64::from(self.turns),
                input_tokens: count(self.result.usage.input_tokens)?,
                output_tokens: count(self.result.usage.output_tokens)?,
                cache_read_tokens: count(self.result.usage.cache_read_tokens)?,
                cache_create_tokens: count(self.result.usage.cache_creation_tokens)?,
            },
            created_at: Utc::now(),
            effect: state.effect.clone(),
            runtime: Some(RuntimeCheckpoint {
                version: 1,
                boundary: name.into(),
                started_at: state.started_at,
                deadline_at: state.deadline_at,
                fingerprint: state.fingerprint.clone(),
                result: self.result.clone(),
                policy: self.policy.clone(),
                base_turn_limit: Some(self.base_turn_limit),
                stop_gate_blocks: self.stop_gate_blocks,
                phase: self.phase,
                calls: self.calls.clone(),
                turns: self.turns,
                cost: self.cost,
                tool_calls: state.tool_calls,
                tool_pause: self.tool_pause,
                tool_final: self.tool_final.clone(),
                tool_turn_start: self.tool_turn_start,
                consecutive_tool_errors: self.consecutive_tool_errors,
                tool_error_escalated: self.tool_error_escalated,
                applied_child_messages: self.applied_child_messages.clone(),
                child_deliveries: delivery_ids.clone(),
                approval_journal_present: !self.approval_journal.entries().is_empty(),
                approval_journal: self.approval_journal.entries(),
            }),
        };
        bounded(
            &self.context,
            None,
            state.store.persist(&self.context, &checkpoint),
        )
        .await?;
        state.sequence = sequence;
        if let Some(session) = &self.config.subagents {
            session.commit_delivery(&delivery_ids).await?;
        }
        Ok(())
    }
}

/// Verified host state required to migrate a baseline Go checkpoint. These are
/// cumulative run totals, not deltas. The host must verify the original agent and
/// security policy match this runner and preserve the original absolute deadline.
pub struct GoRecovery {
    pub policy: RunPolicy,
    /// Required when a stop gate is configured: Go does not persist these values.
    pub stop_gate_blocks: Option<usize>,
    pub effective_max_turns: Option<std::num::NonZeroU32>,
    pub turns: u32,
    pub usage: Usage,
    pub cost: f64,
    pub tool_calls: u64,
    pub started_at: DateTime<Utc>,
    pub deadline_at: Option<DateTime<Utc>>,
    /// Go checkpoints do not persist a parsed final result; terminal migration
    /// requires the host to supply its verified result rather than invent one.
    pub final_output: Option<Value>,
}

impl Runner {
    pub fn migrate_go_checkpoint(
        &self,
        mut checkpoint: RunnerCheckpoint,
        recovery: GoRecovery,
    ) -> Result<RunnerCheckpoint, Error> {
        if checkpoint.schema_version != 1 || checkpoint.runtime.is_some() {
            return Err(unsupported(
                "migration requires a baseline Go schema-1 checkpoint",
            ));
        }
        if !matches!(
            checkpoint.boundary.as_str(),
            "run_started"
                | "run_completed"
                | "tool_completed"
                | "handoff_completed"
                | "paused"
                | "child_changed"
        ) {
            return Err(unsupported(
                "Go boundary requires operator reconciliation; prepared does not prove undispatched",
            ));
        }
        if checkpoint.effect.is_some()
            || checkpoint
                .interruptions
                .as_ref()
                .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|v| !v.is_empty()))
        {
            return Err(unsupported(
                "Go child, effect or approval state requires reconciliation",
            ));
        }
        if let Some(children) = checkpoint.children.clone().filter(|v| !v.is_null()) {
            reconcile_children(children)?;
        }
        let count = |n: i64| u64::try_from(n).map_err(invalid);
        if u64::from(recovery.turns) < count(checkpoint.usage.requests)?
            || recovery.usage.input_tokens < count(checkpoint.usage.input_tokens)?
            || recovery.usage.output_tokens < count(checkpoint.usage.output_tokens)?
            || recovery.usage.cache_read_tokens < count(checkpoint.usage.cache_read_tokens)?
            || recovery.usage.cache_creation_tokens < count(checkpoint.usage.cache_create_tokens)?
            || !recovery.cost.is_finite()
            || recovery.cost < 0.0
        {
            return Err(invalid(
                "verified counters cannot decrease Go checkpoint totals",
            ));
        }
        let mut history = vec![];
        let mut approval_journal = vec![];
        for item in &checkpoint.history {
            use adk_codec::dto::{RunItemType, SnapshotType};
            let mut wire = adk_codec::dto::RunItem {
                agent: (!item.agent_name.is_empty()).then(|| adk_codec::dto::AgentRef {
                    name: item.agent_name.clone(),
                }),
                ..Default::default()
            };
            match item.kind {
                SnapshotType::Message => {
                    wire.kind = RunItemType(0);
                    wire.message = Some(adk_codec::dto::MessageOutput {
                        text: item.message_text.clone(),
                        phase: item.message_phase.clone(),
                        images: item.message_images.clone(),
                    });
                }
                SnapshotType::ToolCall => {
                    wire.kind = RunItemType(1);
                    let call = item
                        .tool_call
                        .as_ref()
                        .ok_or_else(|| invalid("missing Go tool call"))?;
                    wire.tool_call = Some(adk_codec::dto::ToolCallData {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        input: call.input.clone(),
                    });
                }
                SnapshotType::ToolOutput => {
                    wire.kind = RunItemType(2);
                    wire.tool_output = item.tool_output.clone();
                }
                SnapshotType::Reasoning => {
                    wire.kind = RunItemType(5);
                    wire.reasoning = Some(
                        serde_json::from_value(
                            serde_json::to_value(
                                item.reasoning
                                    .as_ref()
                                    .ok_or_else(|| invalid("missing Go reasoning payload"))?,
                            )
                            .map_err(invalid)?,
                        )
                        .map_err(invalid)?,
                    );
                }
                SnapshotType::ToolApproval => {
                    let approval = item
                        .tool_approval
                        .as_ref()
                        .ok_or_else(|| invalid("missing Go approval payload"))?;
                    let data = adk_codec::dto::ToolApprovalData {
                        tool_name: approval.tool_name.clone(),
                        input: approval.input.clone(),
                        call_id: approval.call_id.clone(),
                        approved: approval.approved,
                    };
                    approval_journal.push(crate::compat::ApprovalJournalEntry {
                        marker: adk_codec::approval::ApprovalMarker {
                            phase: if data.approved {
                                adk_codec::approval::ApprovalPhase::Approved
                            } else {
                                adk_codec::approval::ApprovalPhase::Pending
                            },
                            data,
                            agent: wire.agent.clone(),
                        },
                        new_items_before: 0,
                        history_before: Some(history.len()),
                        reason: None,
                    });
                    continue;
                }
                SnapshotType::Compaction => {
                    wire.kind = RunItemType(7);
                    wire.compaction = Some(
                        serde_json::from_value(
                            serde_json::to_value(
                                item.compaction
                                    .as_ref()
                                    .ok_or_else(|| invalid("missing Go compaction payload"))?,
                            )
                            .map_err(invalid)?,
                        )
                        .map_err(invalid)?,
                    );
                }
                _ => {
                    return Err(unsupported(
                        "Go approval/handoff/unknown history requires explicit migration",
                    ));
                }
            }
            history.push(adk_codec::approval::decode_item(&wire).map_err(invalid)?);
        }
        validate_history_pairs(&history)?;
        let completed = checkpoint.boundary == "run_completed";
        if completed && recovery.final_output.is_none() {
            return Err(invalid(
                "terminal Go migration requires verified final output",
            ));
        }
        let base_turn_limit = recovery.policy.max_turns;
        let mut policy = recovery.policy;
        let stop_gate_blocks = if self.config.stop_gate.is_some() {
            let blocks = recovery
                .stop_gate_blocks
                .ok_or_else(|| invalid("Go stop gate migration requires verified block count"))?;
            policy.max_turns = recovery.effective_max_turns.ok_or_else(|| {
                invalid("Go stop gate migration requires verified effective turn limit")
            })?;
            if blocks > self.config.stop_gate_max_blocks || policy.max_turns < base_turn_limit {
                return Err(invalid("invalid verified Go stop gate state"));
            }
            blocks
        } else {
            0
        };
        checkpoint.runtime = Some(RuntimeCheckpoint {
            version: 1,
            boundary: checkpoint.boundary.clone(),
            fingerprint: self.durable_fingerprint()?,
            started_at: recovery.started_at,
            deadline_at: recovery.deadline_at,
            result: RunResult {
                status: if completed {
                    RunStatus::Completed
                } else {
                    RunStatus::Incomplete
                },
                final_output: recovery.final_output,
                new_items: vec![],
                history,
                responses: vec![],
                usage: recovery.usage,
                pending_approvals: vec![],
                last_agent: Some(checkpoint.agent_name.clone()),
            },
            base_turn_limit: Some(base_turn_limit),
            stop_gate_blocks,
            policy,
            phase: if completed {
                Phase::Finish
            } else {
                Phase::Model
            },
            calls: VecDeque::new(),
            turns: recovery.turns,
            cost: recovery.cost,
            tool_calls: recovery.tool_calls,
            tool_pause: false,
            tool_final: None,
            tool_turn_start: None,
            consecutive_tool_errors: 0,
            tool_error_escalated: false,
            applied_child_messages: HashSet::new(),
            child_deliveries: Vec::new(),
            approval_journal_present: !approval_journal.is_empty(),
            approval_journal,
        });
        if !completed {
            checkpoint.boundary = "model_completed".into();
        }
        Ok(checkpoint)
    }
}

struct StoredSession {
    lease: adk_durable::Lease,
    snapshot: adk_durable::RunSnapshot,
    sequence: u64,
    poisoned: bool,
}

/// Adapter for the synchronous RunStore contract. Writes run inline (and may
/// block); no background task outlives the runner. Hosts using these stores should
/// dedicate an executor thread to the run. Lease acquisition/renewal/release are
/// host-owned; an expired lease or any failed write permanently closes this adapter.
pub struct StoredCheckpointStore {
    store: Arc<dyn adk_durable::RunStore>,
    session: std::sync::Mutex<StoredSession>,
}
impl StoredCheckpointStore {
    pub fn open(
        store: Arc<dyn adk_durable::RunStore>,
        lease: adk_durable::Lease,
    ) -> Result<Self, Error> {
        let (snapshot, _) = store
            .load(&lease.tenant_id, &lease.run_id)
            .map_err(persistence_error)?;
        if snapshot.cancellation.is_some() || !snapshot.child_runs.is_empty() {
            return Err(unsupported(
                "stored cancellation/child runs require reconciliation",
            ));
        }
        if snapshot.effects.iter().any(|e| {
            matches!(
                e.state,
                EffectState::Dispatched | EffectState::OutcomeUnknown
            )
        }) {
            return Err(unsupported(
                "operator_resolution: stored effect outcome is unknown",
            ));
        }
        let sequence = snapshot
            .state
            .as_ref()
            .map(|value| {
                RunnerCheckpoint::decode(&serde_json::to_vec(value).map_err(invalid)?)
                    .map(|checkpoint| checkpoint.sequence)
            })
            .transpose()?
            .unwrap_or(0);
        Ok(Self {
            store,
            session: std::sync::Mutex::new(StoredSession {
                lease,
                snapshot,
                sequence,
                poisoned: false,
            }),
        })
    }

    pub fn checkpoint(&self) -> Result<Option<RunnerCheckpoint>, Error> {
        let session = self
            .session
            .lock()
            .map_err(|_| persistence_error("poisoned persistence lock"))?;
        session
            .snapshot
            .state
            .as_ref()
            .map(|value| RunnerCheckpoint::decode(&serde_json::to_vec(value).map_err(invalid)?))
            .transpose()
    }
}
fn persistence_error(error: impl std::fmt::Display) -> Error {
    Error::new(ErrorCategory::Host, format!("durable persistence: {error}"))
}
impl CheckpointStore for StoredCheckpointStore {
    fn persist<'a>(
        &'a self,
        context: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            context.check_active()?;
            let mut session = self
                .session
                .lock()
                .map_err(|_| persistence_error("poisoned persistence lock"))?;
            if session.poisoned {
                return Err(persistence_error(
                    "session closed after persistence failure",
                ));
            }
            session.poisoned = true;
            if checkpoint.run_id != context.run_id
                || checkpoint.run_id != session.snapshot.run_id.to_string()
                || checkpoint.sequence
                    != session
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| invalid("sequence overflow"))?
            {
                return Err(persistence_error(
                    "run identity or checkpoint sequence conflict",
                ));
            }
            let runtime = checkpoint
                .runtime
                .as_ref()
                .ok_or_else(|| invalid("missing runtime checkpoint"))?;
            let mut next = session.snapshot.clone();
            next.revision = next
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("revision overflow"))?;
            next.updated_at = checkpoint.created_at;
            next.state = Some(serde_json::to_value(checkpoint).map_err(invalid)?);
            next.status = if checkpoint.execution_boundary() == "run_completed" {
                adk_durable::RunStatus::Succeeded
            } else {
                adk_durable::RunStatus::Running
            };
            if let Some(effect) = &checkpoint.effect {
                if let Some(previous) = next.effects.iter_mut().find(|e| e.id == effect.id) {
                    *previous = effect.clone();
                } else {
                    next.effects.push(effect.clone());
                }
            }
            let count = |n: u64| i64::try_from(n).map_err(invalid);
            let cost_micros = (runtime.cost * 1_000_000.0).ceil();
            if !cost_micros.is_finite() || cost_micros < 0.0 || cost_micros >= i64::MAX as f64 {
                return Err(invalid("durable cost counter overflow"));
            }
            let budget = adk_durable::BudgetCounters {
                input_tokens: count(runtime.result.usage.input_tokens)?,
                output_tokens: count(runtime.result.usage.output_tokens)?,
                tool_calls: count(runtime.tool_calls)?,
                cost_micros: cost_micros as i64,
                wall_time_ms: runtime
                    .wall_time_ms(checkpoint.created_at)
                    .max(next.cumulative_budget.wall_time_ms),
            };
            let prior = &next.cumulative_budget;
            if budget.input_tokens < prior.input_tokens
                || budget.output_tokens < prior.output_tokens
                || budget.tool_calls < prior.tool_calls
                || budget.cost_micros < prior.cost_micros
            {
                return Err(invalid("durable counters cannot decrease"));
            }
            next.cumulative_budget = budget;
            let event = adk_durable::Event {
                event_type: checkpoint.execution_boundary().into(),
                classification: next.classification,
                payload: next.state.clone(),
                ..Default::default()
            };
            let expected_state = next.state.clone();
            let committed = self
                .store
                .append(&session.lease, session.snapshot.revision, vec![event], next)
                .map_err(persistence_error)?;
            // Redaction may intentionally remove private data, but must not
            // acknowledge an unusable executable checkpoint and then dispatch.
            if committed.state != expected_state {
                return Err(persistence_error(
                    "redaction changed executable continuation",
                ));
            }
            session.snapshot = committed;
            session.sequence = checkpoint.sequence;
            context.check_active()?;
            session.poisoned = false;
            Ok(())
        })
    }
}
