//! Source-backed, caller-driven host orchestration. No tasks or polling are started.
//!
//! Typed batches preserve approval journals and authorship across caller-supplied
//! history, ordered persistence and explicit session ownership.
use crate::builder::{ModeSpec, RoleSpec, SessionHandle, SessionState};
use crate::observability::ProgressSnapshot;
use adk_codec::approval::{ApprovalMarker, ApprovalMarkerBoundary, ApprovalPhase};
use adk_core::*;
use adk_runtime::compat::{GoApprovalBoundary, GoApprovalDecision, GoApprovalGate};
use adk_runtime::{
    AgentConfig, CompositeHooks, Observation, RunHooks, RunOutcome, Runner, RunnerConfig,
};
use std::{
    future::Future,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

mod rules;
pub use rules::{CompiledGuardrails, GuardrailRule, compile_guardrail_rules};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub message_id: i64,
    pub token: String,
}
#[derive(Clone, Debug, Default)]
pub struct UserMessage {
    pub id: i64,
    pub content: String,
    pub mode: String,
    pub created_at: Option<std::time::SystemTime>,
    pub images: Vec<adk_codec::dto::ImageAttachment>,
}
#[derive(Clone, Debug, Default)]
pub struct WorkingState {
    pub goal: String,
    pub current_mode: String,
    pub current_step: String,
    pub last_user_message: String,
    pub last_assistant_summary: String,
    pub recent_turn_summaries: Vec<String>,
    pub history_floor_message_id: i64,
    pub last_response_id: String,
    pub data: serde_json::Map<String, serde_json::Value>,
}
impl WorkingState {
    pub fn context(&self) -> String {
        fn snippet(s: &str) -> String {
            let s = s.replace('\n', " ");
            let s = s.trim();
            let mut out: String = s.chars().take(320).collect();
            if s.chars().count() > 320 {
                out.push_str("...");
            }
            out
        }
        let mut lines = Vec::new();
        for (label, value) in [
            ("Current objective", &self.goal),
            ("Mode", &self.current_mode),
            ("Current step", &self.current_step),
            ("Latest user direction", &self.last_user_message),
            ("Latest assistant summary", &self.last_assistant_summary),
        ] {
            if value.is_empty() || (label == "Latest user direction" && value == &self.goal) {
                continue;
            }
            lines.push(format!(
                "{label}: {}",
                if label == "Mode" {
                    value.clone()
                } else {
                    snippet(value)
                }
            ));
        }
        if !self.recent_turn_summaries.is_empty() {
            let start = self.recent_turn_summaries.len().saturating_sub(4);
            lines.push(format!(
                "Recent progress:\n- {}",
                self.recent_turn_summaries[start..]
                    .iter()
                    .map(|s| snippet(s))
                    .collect::<Vec<_>>()
                    .join("\n- ")
            ));
        }
        if lines.is_empty() {
            String::new()
        } else {
            format!("## Durable Working State\n{}", lines.join("\n"))
        }
    }
}

/// Native items plus exact authorship and ordered approval marker sidecars.
/// Empty provenance means Unknown, never the currently executing agent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunBatch {
    pub items: Vec<RunItem>,
    pub provenance: Vec<ItemProvenance>,
    pub markers: Vec<ApprovalMarkerBoundary>,
}
pub trait SessionStore: Send + Sync {
    fn load_messages<'a>(
        &'a self,
        context: &'a Context,
        cursor: &'a Cursor,
        limit: usize,
    ) -> BoxFuture<'a, Result<(Vec<UserMessage>, Cursor), Error>>;
    fn append_run_items<'a>(
        &'a self,
        context: &'a Context,
        batch: &'a RunBatch,
    ) -> BoxFuture<'a, Result<(), Error>>;
    fn working_state<'a>(
        &'a self,
        context: &'a Context,
    ) -> BoxFuture<'a, Result<WorkingState, Error>>;
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PermissionMode {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    DangerFullAccess,
}
impl PermissionMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "workspace-write" => Self::WorkspaceWrite,
            "danger-full-access" => Self::DangerFullAccess,
            _ => Self::ReadOnly,
        }
    }
}
pub trait ConfigSource: Send + Sync {
    fn permission_mode<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<PermissionMode, Error>> {
        Box::pin(async { Ok(PermissionMode::default()) })
    }
    fn mode_directive<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async { Ok(String::new()) })
    }
    fn guardrail_rules<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<Vec<GuardrailRule>, Error>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn mode_snapshot<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<Option<ModeSpec>, Error>> {
        Box::pin(async { Ok(None) })
    }
    /// Available to tool factories; the loop never calls this method.
    fn role_catalog<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<Vec<RoleSpec>, Error>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn handoff_history<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<RunBatch, Error>> {
        Box::pin(async { Ok(RunBatch::default()) })
    }
}
pub trait RunStatusSink: Send + Sync {
    fn publish_progress<'a>(
        &'a self,
        _: &'a Context,
        _: &'a ProgressSnapshot,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn publish_trace_id<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn publish_final<'a>(
        &'a self,
        context: &'a Context,
        result: &'a RunResult,
    ) -> BoxFuture<'a, Result<(), Error>>;
}
pub trait TraceStore: Send + Sync {
    fn run_dir<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async { Ok(String::new()) })
    }
    fn append_category<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn write_file<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        _: &'a [u8],
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn finalize<'a>(
        &'a self,
        context: &'a Context,
        result: &'a RunResult,
    ) -> BoxFuture<'a, Result<(), Error>>;
}
pub trait PlatformToolFactory: Send + Sync {
    fn build_tools<'a>(
        &'a self,
        context: &'a Context,
        base: Vec<Arc<dyn Tool>>,
    ) -> BoxFuture<'a, Result<Vec<Arc<dyn Tool>>, Error>>;
}
#[derive(Clone, Debug)]
pub struct ToolApprovalRequest {
    pub tool_name: String,
    pub input: serde_json::Value,
    pub reason: String,
}
pub trait ApprovalGate: Send + Sync {
    fn approve_tool<'a>(
        &'a self,
        context: &'a Context,
        request: ToolApprovalRequest,
    ) -> BoxFuture<'a, Result<GoApprovalDecision, Error>>;
}
/// Owned state is closed/dropped with the loop. A borrowed handle never closes its owner.
pub enum Session {
    Owned(SessionState),
    Borrowed(SessionHandle),
}
impl Default for Session {
    fn default() -> Self {
        Self::Owned(SessionState::new())
    }
}
impl Session {
    pub fn handle(&self) -> SessionHandle {
        match self {
            Self::Owned(s) => s.handle(),
            Self::Borrowed(s) => s.clone(),
        }
    }
}
pub struct ChatLoopOptions {
    pub agent: AgentConfig,
    pub runner_config: RunnerConfig,
    pub policy: RunPolicy,
    /// None permits the dynamic mode to fill this limit.
    pub max_turns: Option<NonZeroU32>,
    /// Separate sentinels retain source permission defaulting without erasing native policy.
    pub tool_access: Option<AccessMode>,
    pub approval_required: Option<bool>,
    pub additional_instructions: String,
    pub session: Session,
    pub session_store: Option<Arc<dyn SessionStore>>,
    pub status: Option<Arc<dyn RunStatusSink>>,
    pub config_source: Option<Arc<dyn ConfigSource>>,
    pub trace_store: Option<Arc<dyn TraceStore>>,
    pub approval_gate: Option<Arc<dyn ApprovalGate>>,
    pub tool_factory: Option<Arc<dyn PlatformToolFactory>>,
    pub host: Option<Arc<dyn Host>>,
    pub cursor: Cursor,
    pub message_limit: i64,
    pub max_resumes: i64,
}
impl ChatLoopOptions {
    pub fn new(agent: AgentConfig) -> Self {
        Self {
            agent,
            runner_config: RunnerConfig::default(),
            policy: RunPolicy {
                tools: ToolPolicy {
                    access: AccessMode::FullAccess,
                    ..Default::default()
                },
                ..Default::default()
            },
            max_turns: None,
            tool_access: None,
            approval_required: None,
            additional_instructions: String::new(),
            session: Session::default(),
            session_store: None,
            status: None,
            config_source: None,
            trace_store: None,
            approval_gate: None,
            tool_factory: None,
            host: None,
            cursor: Cursor::default(),
            message_limit: 50,
            max_resumes: 12,
        }
    }
}
pub struct ChatLoop {
    options: ChatLoopOptions,
}
impl ChatLoop {
    pub fn new(options: ChatLoopOptions) -> Self {
        Self { options }
    }
    pub fn cursor(&self) -> &Cursor {
        &self.options.cursor
    }
    pub fn session(&self) -> SessionHandle {
        self.options.session.handle()
    }
    pub async fn close(&mut self) -> Result<(), Error> {
        match &mut self.options.session {
            Session::Owned(s) => s.close().await,
            Session::Borrowed(_) => Ok(()),
        }
    }
    pub async fn run(&mut self, context: Context) -> Result<RunOutcome, RunError> {
        let context = self.session().context(&context);
        context.check_active()?;
        let (agent, mut config, policy) = self.prepare(&context).await?;
        let input = self.load(&context).await?;
        let collector = Arc::new(Collector::default());
        let mut hooks = vec![collector.clone() as Arc<dyn RunHooks>];
        if let Some(hook) = config.hooks.take() {
            hooks.push(hook);
        }
        config.hooks = Some(Arc::new(CompositeHooks::new(hooks)));
        let runner = Runner::new(agent, config)?;
        let host = Arc::new(DeferredHost(self.options.host.clone()));
        let request = RunRequest {
            input: input.items,
            input_provenance: input.provenance,
            policy,
        };
        let mut outcome = runner
            .run_with_approval_history(context.clone(), request, host, input.markers)
            .await
            .map_err(|e| RunError::from(e.error))?;
        let boundary = AppendBoundary {
            collector: collector.clone(),
            store: self.options.session_store.clone(),
        };
        let max_resumes = if self.options.max_resumes <= 0 {
            12
        } else {
            self.options.max_resumes as usize
        };
        let mut resumes = 0;
        loop {
            if let Err(error) = boundary.append(&context, &outcome.result).await {
                return Err(partial(error, outcome));
            }
            if outcome.result.pending_approvals.is_empty() {
                break;
            }
            let Some(gate) = &self.options.approval_gate else {
                let mut denied = RunBatch::default();
                for pending in &outcome.result.pending_approvals {
                    denied.markers.push(ApprovalMarkerBoundary {
                        before_item: denied.items.len(),
                        marker: ApprovalMarker::from_call(
                            &pending.call,
                            ApprovalPhase::Denied,
                            None,
                        ),
                    });
                    denied.items.push(RunItem::ToolResult {
                        call_id: pending.call.id.clone(),
                        output: ToolOutput {
                            content: vec![Content::Text {
                                text: "tool call denied: no approval gate configured".into(),
                            }],
                            is_error: true,
                            should_pause: false,
                        },
                    });
                    denied.provenance.push(ItemProvenance::Unattributed);
                }
                outcome.continuation.take();
                outcome.result.new_items.extend_from_slice(&denied.items);
                outcome
                    .result
                    .new_items_provenance
                    .extend_from_slice(&denied.provenance);
                if let Some(store) = &self.options.session_store {
                    if let Err(error) = call(
                        &context,
                        store.append_run_items(&context, &denied),
                        "append denied approval items",
                    )
                    .await
                    {
                        return Err(partial(error, outcome));
                    }
                }
                break;
            };
            if resumes >= max_resumes {
                return Err(partial(
                    Error::new(
                        ErrorCategory::MaxTurns,
                        "too many chat loop resumes after approval interruption",
                    ),
                    outcome,
                ));
            }
            let continuation = outcome
                .continuation
                .take()
                .expect("native pending approval has a continuation");
            collector.approval_phase.store(true, Ordering::SeqCst);
            outcome = match continuation
                .resume_go_gate_with_boundary(&Gate(gate.clone()), &boundary)
                .await
            {
                Ok(outcome) => outcome,
                Err(error) if collector.approval_phase.load(Ordering::SeqCst) => return Err(error),
                Err(error) => return Err(RunError::from(error.error)),
            };
            resumes += 1;
            if outcome.result.status == RunStatus::Paused
                && outcome.result.pending_approvals.is_empty()
            {
                break;
            }
        }
        if let Some(trace) = &self.options.trace_store {
            if let Err(error) = call(
                &context,
                trace.finalize(&context, &outcome.result),
                "finalize trace",
            )
            .await
            {
                return Err(partial(error, outcome));
            }
        }
        if let Some(status) = &self.options.status {
            if let Err(error) = call(
                &context,
                status.publish_final(&context, &outcome.result),
                "publish final result",
            )
            .await
            {
                return Err(partial(error, outcome));
            }
        }
        Ok(outcome)
    }
    async fn prepare(
        &self,
        context: &Context,
    ) -> Result<(AgentConfig, RunnerConfig, RunPolicy), Error> {
        let mut agent = self.options.agent.clone();
        let mut config = self.options.runner_config.clone();
        let mut policy = self.options.policy.clone();
        let mut max_turns = self.options.max_turns;
        let mut instructions = self.options.additional_instructions.clone();
        if let Some(source) = &self.options.config_source {
            let mode = call(
                context,
                source.permission_mode(context),
                "load permission mode",
            )
            .await?;
            config.approve_mutating_tools = self
                .options
                .approval_required
                .unwrap_or(mode != PermissionMode::DangerFullAccess);
            policy.tools.access = self.options.tool_access.unwrap_or(match mode {
                PermissionMode::ReadOnly => AccessMode::ReadOnly,
                _ => AccessMode::FullAccess,
            });
            let directive = call(
                context,
                source.mode_directive(context),
                "load mode directive",
            )
            .await?;
            if !directive.trim().is_empty() {
                instructions = format!("{instructions}\n\n{directive}").trim().into();
            }
            let rules = call(
                context,
                source.guardrail_rules(context),
                "load guardrail rules",
            )
            .await?;
            let guards = compile_guardrail_rules(&rules)?;
            config.tool_input_guardrails.extend(guards.input);
            config.tool_output_guardrails.extend(guards.output);
            if let Some(snapshot) =
                call(context, source.mode_snapshot(context), "load mode snapshot").await?
            {
                if matches!(
                    snapshot.tool_access.trim().to_ascii_lowercase().as_str(),
                    "read-only" | "read_only" | "readonly" | "analysis"
                ) {
                    policy.tools.access = AccessMode::ReadOnly;
                }
                if let Some(constraints) = snapshot.constraints {
                    if max_turns.is_none() {
                        max_turns = constraints.max_turns;
                    }
                    if config.subagent_max_turns.is_none() {
                        config.subagent_max_turns = constraints.subagent_max_turns;
                    }
                }
            }
        } else {
            if let Some(access) = self.options.tool_access {
                policy.tools.access = access;
            }
            if let Some(required) = self.options.approval_required {
                config.approve_mutating_tools = required;
            }
        }
        if let Some(limit) = max_turns {
            policy.max_turns = limit;
        }
        if config.subagent_max_turns.is_none() {
            config.subagent_max_turns = NonZeroU32::new(50);
        }
        if !instructions.is_empty() {
            agent.instructions = if agent.instructions.trim().is_empty() {
                instructions
            } else {
                format!("{}\n\n---\n\n{instructions}", agent.instructions)
            };
        }
        if let Some(store) = &self.options.session_store {
            let state = call(context, store.working_state(context), "load working state").await?;
            if !state.current_step.is_empty() || !state.last_assistant_summary.is_empty() {
                config.working_state_context = state.context();
            }
        }
        if let Some(factory) = &self.options.tool_factory {
            agent.tools = call(
                context,
                factory.build_tools(context, agent.tools),
                "build platform tools",
            )
            .await?;
        }
        if config.subagents.is_none() {
            config.subagents = self.session().subagents().cloned();
        }
        Ok((agent, config, policy))
    }
    async fn load(&mut self, context: &Context) -> Result<RunBatch, Error> {
        let mut input = if let Some(source) = &self.options.config_source {
            call(
                context,
                source.handoff_history(context),
                "load handoff history",
            )
            .await?
        } else {
            RunBatch::default()
        };
        input.provenance = normalize_provenance(input.items.len(), &input.provenance)?;
        if let Some(store) = &self.options.session_store {
            let limit = if self.options.message_limit <= 0 {
                50
            } else {
                self.options.message_limit as usize
            };
            loop {
                let (messages, next) = call(
                    context,
                    store.load_messages(context, &self.options.cursor, limit),
                    "load session messages",
                )
                .await?;
                let count = messages.len();
                let advanced = next != self.options.cursor;
                self.options.cursor = next;
                for message in messages {
                    if message.content.trim().is_empty() && message.images.is_empty() {
                        continue;
                    }
                    let mut content = vec![Content::Text {
                        text: message.content,
                    }];
                    content.extend(message.images.into_iter().map(|image| Content::Attachment {
                        media_type: image.media_type,
                        data: image.data,
                        detail: image.detail,
                    }));
                    input.items.push(RunItem::Message {
                        message: Message {
                            role: Role::User,
                            content,
                        },
                    });
                    input.provenance.push(ItemProvenance::Unattributed);
                }
                if count < limit || !advanced {
                    break;
                }
            }
        }
        Ok(input)
    }
}
async fn call<T>(
    context: &Context,
    future: impl Future<Output = Result<T, Error>>,
    label: &str,
) -> Result<T, Error> {
    context.check_active()?;
    let deadline = async {
        match context.deadline {
            Some(d) => tokio::time::sleep_until(d.into()).await,
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! { biased;
        _ = context.cancellation.cancelled() => Err(Error::new(ErrorCategory::Cancelled, "operation cancelled")),
        _ = deadline => Err(Error::new(ErrorCategory::DeadlineExceeded, "deadline exceeded")),
        result = future => result,
    };
    result.map_err(|e| Error::new(e.info.category, format!("{label}: {e}")).with_source(e))
}
#[derive(Default)]
struct Collector {
    batch: Mutex<RunBatch>,
    offset: Mutex<usize>,
    approval_phase: AtomicBool,
}
impl RunHooks for Collector {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            match observation {
                Observation::CommittedItems { items, markers, .. } => {
                    let mut batch = self.batch.lock().unwrap();
                    let base = batch.items.len();
                    batch.markers.extend(markers.into_iter().map(|mut marker| {
                        marker.before_item += base;
                        marker
                    }));
                    batch.items.extend(items);
                }
                Observation::ModelAttempt { .. } => {
                    self.approval_phase.store(false, Ordering::SeqCst);
                }
                _ => {}
            }
            Ok(())
        })
    }
}
struct AppendBoundary {
    collector: Arc<Collector>,
    store: Option<Arc<dyn SessionStore>>,
}
impl AppendBoundary {
    async fn append(&self, context: &Context, result: &RunResult) -> Result<(), Error> {
        let mut batch = std::mem::take(&mut *self.collector.batch.lock().unwrap());
        {
            let mut offset = self.collector.offset.lock().unwrap();
            let end = *offset + batch.items.len();
            batch.provenance = result.new_items_provenance[*offset..end].to_vec();
            *offset = end;
        }
        if let Some(store) = &self.store {
            call(
                context,
                store.append_run_items(context, &batch),
                "append run items",
            )
            .await?;
        }
        Ok(())
    }
}
impl GoApprovalBoundary for AppendBoundary {
    fn commit<'a>(
        &'a self,
        context: &'a Context,
        result: &'a RunResult,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(self.append(context, result))
    }
}
struct Gate(Arc<dyn ApprovalGate>);
impl GoApprovalGate for Gate {
    fn approve<'a>(
        &'a self,
        context: &'a Context,
        request: &'a ApprovalRequest,
    ) -> BoxFuture<'a, Result<GoApprovalDecision, Error>> {
        self.0.approve_tool(
            context,
            ToolApprovalRequest {
                tool_name: request.call.name.clone(),
                input: request.call.arguments.clone(),
                reason: "tool approval required".into(),
            },
        )
    }
}
struct DeferredHost(Option<Arc<dyn Host>>);
impl Host for DeferredHost {
    fn emit<'a>(
        &'a self,
        context: &'a Context,
        event: RunEvent,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if let Some(host) = &self.0 {
                host.emit(context, event).await?;
            }
            Ok(())
        })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Defer) })
    }
}

#[derive(Debug)]
struct FailedSpills {
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    _spills: Vec<Arc<adk_runtime::output::SpillFile>>,
}
impl std::fmt::Display for FailedSpills {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("retaining failed host run spill files")
    }
}
impl std::error::Error for FailedSpills {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|source| source as _)
    }
}
fn partial(mut error: Error, outcome: RunOutcome) -> RunError {
    if !outcome.spills.is_empty() {
        error.source = Some(Box::new(FailedSpills {
            source: error.source.take(),
            _spills: outcome.spills,
        }));
    }
    RunError::with_partial(error, outcome.result)
}
