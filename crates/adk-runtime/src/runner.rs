//! Owned execution state. Snapshots are observations, not replayable checkpoints.
//! Dropping a run future or pull stream drops all in-flight provider/tool futures.
//! Providers and tools must honor the core no-detached-work contract; cancellation
//! cannot undo effects already performed. Read-only tools share batch execution;
//! mutations are exclusive. Approval/handoff boundaries are resolved first. A first
//! handoff preempts sibling calls, which receive explicit not-executed results.
//!
//! Normal runs use `Model::complete`; pull streams use `StreamingModel::stream`
//! when supplied, or only completion events for an explicit complete-only binding.
//! Model retries/fallbacks stop at the first visible provider event. Fallbacks
//! stay active per agent until three successful calls trigger a primary reprobe.
//! Token/cost limits use reported consumption and can only stop after a response
//! crosses the limit; provider-side hard generation caps belong in model settings.
//! Compactors must report any successful provider usage/cost, not assume zero.
//!
//! Approval continuations are in-process, single-use owners, not serializable
//! checkpoints. DurableHook is fail-closed boundary observation, not crash replay.
//! Failed events are best effort when the host, deadline or cancellation prevents
//! delivery; `RunError` remains authoritative and retains partial history/spills.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use adk_codec::approval::ApprovalMarkerBoundary;
use adk_core::*;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::compaction::{
    EstimateCalibration, LocalCompactionPolicy, REQUEST_SAFETY_BUFFER, compact_with_approvals,
    estimate_history_tokens, estimate_history_tokens_with_approvals,
    estimate_request_overhead_tokens, finalize_local_history_with_approvals, output_reserve_tokens,
};
use crate::output::{OutputPolicy, SpillFile};

#[derive(Clone)]
pub enum ModelBinding {
    Complete {
        name: String,
        model: Arc<dyn Model>,
    },
    Streaming {
        name: String,
        model: Arc<dyn StreamingModel>,
    },
}

impl ModelBinding {
    pub fn complete(name: impl Into<String>, model: Arc<dyn Model>) -> Self {
        Self::Complete {
            name: name.into(),
            model,
        }
    }
    pub fn streaming(name: impl Into<String>, model: Arc<dyn StreamingModel>) -> Self {
        Self::Streaming {
            name: name.into(),
            model,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Complete { name, .. } | Self::Streaming { name, .. } => name,
        }
    }
}

pub struct Handoff {
    pub definition: ToolDefinition,
    pub target: Arc<AgentConfig>,
}

pub trait OutputParser: Send + Sync {
    fn parse(&self, raw: &str) -> Result<Value, Error>;
}

pub struct AgentConfig {
    pub name: String,
    pub instructions: String,
    pub model: ModelBinding,
    pub fallbacks: Vec<ModelBinding>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub handoffs: Vec<Handoff>,
    pub output_schema: Option<schemars::Schema>,
    pub output_schema_name: String,
    pub output_schema_strict: bool,
    pub output_parser: Option<Arc<dyn OutputParser>>,
    pub settings: Map<String, Value>,
    pub hooks: Option<Arc<dyn RunHooks>>,
}

impl AgentConfig {
    pub fn new(name: impl Into<String>, model: ModelBinding) -> Self {
        Self {
            name: name.into(),
            instructions: String::new(),
            model,
            fallbacks: vec![],
            tools: vec![],
            handoffs: vec![],
            output_schema: None,
            output_schema_name: "final_output".into(),
            output_schema_strict: true,
            output_parser: None,
            settings: Map::new(),
            hooks: None,
        }
    }
}

pub enum ModelErrorAction {
    Retry,
    Continue,
    Abort,
}

pub trait ModelErrorHandler: Send + Sync {
    fn handle(&self, agent: &str, turn: u32, error: &Error) -> ModelErrorAction;
}

/// Policy retries supplement provider advice before any visible model event.
/// Tools and host callbacks are never retried.
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub retryable: fn(&Error) -> bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 0,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            retryable: |e| e.info.category == ErrorCategory::Provider,
        }
    }
}

#[derive(Default)]
pub struct Limits {
    pub max_tokens: Option<u64>,
    /// Monetary units are selected by the host's CostEstimator.
    pub max_cost: Option<f64>,
}

pub trait CostEstimator: Send + Sync {
    fn cost(&self, model: &str, usage: &Usage) -> f64;
}

#[derive(Debug, Clone)]
pub enum Observation {
    TextDelta {
        delta: String,
    },
    CommittedItems {
        items: Vec<RunItem>,
        agents: Vec<Option<adk_codec::dto::AgentRef>>,
        markers: Vec<ApprovalMarkerBoundary>,
    },
    ApprovalMarker {
        agent: Option<String>,
        call: ToolCall,
        decision: ApprovalDecision,
        reason: Option<String>,
        new_items_before: usize,
        history_before: usize,
    },
    HistoryReplaced {
        before: Vec<RunItem>,
        after: Vec<RunItem>,
    },
    ApprovalHistoryReplaced {
        before: Vec<RunItem>,
        after: Vec<RunItem>,
        markers: Vec<ApprovalMarkerBoundary>,
    },
    AgentStarted {
        agent: String,
    },
    ModelAccepted {
        agent: String,
        response: ModelResponse,
    },
    ToolStarted {
        agent: String,
        call: ToolCall,
    },
    AgentEnded {
        agent: String,
        output: Value,
    },
    ModelAttempt {
        agent: String,
        model: String,
        attempt: u32,
    },
    Retry {
        model: String,
        delay: Duration,
    },
    Fallback {
        from: String,
        to: String,
    },
    RawToolOutput {
        call: ToolCall,
        output: ToolOutput,
    },
    Handoff {
        from: String,
        to: String,
    },
    CompactionStarted {
        context_tokens: u64,
        target_tokens: u64,
    },
    CompactionFailed {
        error: ErrorInfo,
    },
    Compacted {
        before_items: usize,
        after_items: usize,
        context_tokens: u64,
    },
    OutputValidationFailed {
        message: String,
    },
    Usage {
        usage: Usage,
        cost: f64,
    },
}

/// Awaited and fail-closed. RawToolOutput precedes truncation/trust wrapping;
/// hooks must not forward sensitive raw data to untrusted telemetry sinks.
pub trait RunHooks: Send + Sync {
    fn observe<'a>(
        &'a self,
        context: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>>;
}

pub struct CompactionRequest {
    pub agent: String,
    pub model: String,
    pub history: Vec<RunItem>,
    pub context_tokens: u64,
    pub target_tokens: u64,
}

pub struct CompactedHistory {
    pub history: Vec<RunItem>,
    pub context_tokens: u64,
    /// Report provider compaction usage; local compaction uses zero counters.
    pub usage: Usage,
    /// In the same units as CostEstimator; local compaction costs zero.
    pub cost: f64,
}

pub trait Compactor: Send + Sync {
    fn compact<'a>(
        &'a self,
        context: &'a Context,
        request: CompactionRequest,
    ) -> BoxFuture<'a, Result<CompactedHistory, Error>>;
}

pub struct CompactionConfig {
    pub trigger_tokens: u64,
    pub target_tokens: u64,
    pub compactor: Arc<dyn Compactor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    Started,
    ModelPrepared,
    ModelCompleted,
    ToolPrepared,
    ToolCompleted,
    ApprovalPending,
    Handoff,
    Paused,
    Completed,
}

/// Durable adapters must reconcile prepared effects after a crash. This hook
/// alone does not provide serialization, restoration, or exactly-once effects.
pub trait DurableHook: Send + Sync {
    fn checkpoint<'a>(
        &'a self,
        context: &'a Context,
        boundary: Boundary,
        snapshot: &'a RunResult,
        pending: Option<&'a ToolCall>,
    ) -> BoxFuture<'a, Result<(), Error>>;
}

pub trait TurnContext: Send + Sync {
    fn context<'a>(
        &'a self,
        context: &'a Context,
        snapshot: &'a RunResult,
        turn: u32,
    ) -> BoxFuture<'a, Result<Vec<RunItem>, Error>>;
}

/// None accepts the answer; Some(feedback) asks the same engine to continue.
pub trait StopGate: Send + Sync {
    fn check<'a>(
        &'a self,
        context: &'a Context,
        output: &'a Value,
    ) -> BoxFuture<'a, Result<Option<String>, Error>>;
}
pub struct RunnerConfig {
    pub work_dir: PathBuf,
    pub output: OutputPolicy,
    pub retry: RetryPolicy,
    pub error_handler: Option<Arc<dyn ModelErrorHandler>>,
    pub limits: Limits,
    pub cost_estimator: Option<Arc<dyn CostEstimator>>,
    /// None disables the model inactivity timeout. Host backpressure is excluded.
    pub model_idle_timeout: Option<Duration>,
    /// Stable instructions preceding the agent's instructions in every request.
    pub cache_prefix: String,
    /// Hashed with the namespace (run ID by default) before reaching providers.
    pub prompt_cache_key: Option<String>,
    pub prompt_cache_namespace: Option<String>,
    /// StopAfterTool executes the whole batch; by default it returns no output.
    pub return_tool_output: bool,
    /// Optional native guardrail; Go leaves argument decoding/validation to each tool.
    pub validate_tool_arguments: bool,
    pub approve_mutating_tools: bool,
    pub consecutive_tool_error_limit: Option<usize>,
    pub stop_gate: Option<Arc<dyn StopGate>>,
    pub stop_gate_max_blocks: usize,
    pub turn_context: Option<Arc<dyn TurnContext>>,
    /// Request-only context, never persisted in history or compaction input.
    pub transient_context: Vec<RunItem>,
    pub hooks: Option<Arc<dyn RunHooks>>,
    pub durable: Option<Arc<dyn DurableHook>>,
    pub compaction: Option<CompactionConfig>,
    /// Baseline local fallback; independent of the optional provider/custom compactor.
    pub local_compaction: LocalCompactionPolicy,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            work_dir: PathBuf::from("."),
            output: OutputPolicy::default(),
            retry: RetryPolicy::default(),
            error_handler: None,
            limits: Limits::default(),
            cost_estimator: None,
            model_idle_timeout: Some(Duration::from_secs(300)),
            cache_prefix: String::new(),
            prompt_cache_key: None,
            prompt_cache_namespace: None,
            return_tool_output: false,
            validate_tool_arguments: false,
            approve_mutating_tools: false,
            consecutive_tool_error_limit: Some(3),
            stop_gate: None,
            stop_gate_max_blocks: 8,
            turn_context: None,
            transient_context: vec![],
            hooks: None,
            durable: None,
            compaction: None,
            local_compaction: LocalCompactionPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct Runner {
    initial: Arc<AgentConfig>,
    config: Arc<RunnerConfig>,
}

pub struct RunOutcome {
    pub result: RunResult,
    /// Keeps any temporary spill paths referenced by history alive.
    pub spills: Vec<Arc<SpillFile>>,
    pub continuation: Option<Continuation>,
}

/// A single-use capability carrying the actual tool cursor, budgets and agent.
/// It cannot be cloned or reconstructed from RunResult.
pub struct Continuation {
    engine: Engine,
}

impl Continuation {
    /// Resume a single pending approval, or a tool-requested pause with None.
    /// Multiple pending approvals require call-ID decisions via resume_batch.
    pub async fn resume(self, decision: Option<ApprovalDecision>) -> Result<RunOutcome, RunError> {
        self.prepare(decision)?.drive().await
    }
    pub fn stream(self, decision: Option<ApprovalDecision>) -> Result<RunStream, RunError> {
        Ok(RunStream::new(self.prepare(decision)?))
    }
    pub async fn resume_batch(
        self,
        decisions: Vec<(String, ApprovalDecision)>,
    ) -> Result<RunOutcome, RunError> {
        self.prepare_batch(decisions)?.drive().await
    }
    /// Go ChatLoop boundary: resolve and execute each pending call before asking
    /// the next gate, then start a fresh invocation budget without replaying effects.
    pub async fn resume_go_gate(
        mut self,
        gate: &dyn crate::compat::GoApprovalGate,
    ) -> Result<RunOutcome, RunError> {
        self.engine.result.status = RunStatus::Incomplete;
        self.engine.tools_prepared = true;
        self.engine.streaming = false;
        self.engine.sender.take();
        while let Some(call) = self.engine.calls.front().cloned() {
            let Some(request) = self
                .engine
                .result
                .pending_approvals
                .iter()
                .find(|r| r.call.id == call.id)
                .cloned()
            else {
                return Err(self
                    .engine
                    .fail(Error::new(
                        ErrorCategory::InvalidInput,
                        "Go gate resume requires pending approval calls",
                    ))
                    .await);
            };
            let decision = match bounded(
                &self.engine.context,
                None,
                gate.approve(&self.engine.context, &request),
            )
            .await
            {
                Ok(decision) => decision,
                Err(error) => return Err(self.engine.fail(error).await),
            };
            let approval = if decision.approved {
                ApprovalDecision::Approve
            } else {
                ApprovalDecision::Deny
            };
            if !decision.approved && !decision.reason.trim().is_empty() {
                self.engine
                    .denial_reasons
                    .insert(call.id.clone(), decision.reason.trim().into());
            }
            self.engine.approvals.insert(call.id.clone(), approval);
            if let Err(error) = self.engine.tool(call).await {
                return Err(self.engine.fail(error).await);
            }
        }
        self.engine.turns = 0;
        self.engine.policy.max_turns = self.engine.base_turn_limit;
        self.engine.stop_gate_blocks = 0;
        self.engine.consecutive_tool_errors = 0;
        self.engine.tool_error_escalated = false;
        self.engine.fallbacks.clear();
        self.engine.calibration = EstimateCalibration::default();
        self.engine.tool_final = None;
        self.engine.drive().await
    }
    pub fn stream_batch(
        self,
        decisions: Vec<(String, ApprovalDecision)>,
    ) -> Result<RunStream, RunError> {
        Ok(RunStream::new(self.prepare_batch(decisions)?))
    }
    fn prepare(self, decision: Option<ApprovalDecision>) -> Result<Engine, RunError> {
        let decisions = match (self.engine.result.pending_approvals.as_slice(), decision) {
            ([], None) => vec![],
            ([request], Some(decision)) => vec![(request.call.id.clone(), decision)],
            _ => return Err(self.invalid_decisions()),
        };
        self.prepare_batch(decisions)
    }
    fn invalid_decisions(mut self) -> RunError {
        let mut error = Error::new(
            ErrorCategory::InvalidInput,
            "resume decisions must match every pending approval by call ID exactly once",
        );
        if !self.engine.spills.is_empty() {
            error.source = Some(Box::new(FailedSpills {
                source: None,
                _spills: self.engine.spills,
            }));
        }
        self.engine.result.status = RunStatus::Incomplete;
        RunError::with_partial(error, self.engine.result)
    }
    fn prepare_batch(
        mut self,
        decisions: Vec<(String, ApprovalDecision)>,
    ) -> Result<Engine, RunError> {
        let mut seen = HashSet::new();
        if decisions.len() != self.engine.result.pending_approvals.len()
            || decisions.iter().any(|(id, _)| {
                !seen.insert(id.clone())
                    || !self
                        .engine
                        .result
                        .pending_approvals
                        .iter()
                        .any(|request| &request.call.id == id)
            })
        {
            return Err(self.invalid_decisions());
        }
        self.engine.streaming = false;
        self.engine.approvals = decisions.into_iter().collect();
        self.engine.tools_prepared = false;
        self.engine.result.status = RunStatus::Incomplete;
        Ok(self.engine)
    }
}

/// Lazy, owned pull stream with a one-event channel and no spawned tasks.
/// No work runs between pulls; dropping it drops the producer and provider stream.
/// `finish` drains events (also delivered to Host) and returns the final outcome.
pub struct RunStream {
    producer: Option<BoxFuture<'static, Result<RunOutcome, RunError>>>,
    receiver: mpsc::Receiver<RunEvent>,
    outcome: Option<Result<RunOutcome, RunError>>,
}

impl RunStream {
    fn new(mut engine: Engine) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        engine.sender = Some(sender);
        engine.streaming = true;
        Self {
            producer: Some(Box::pin(engine.drive())),
            receiver,
            outcome: None,
        }
    }
    pub async fn next(&mut self) -> Option<RunEvent> {
        loop {
            if let Ok(event) = self.receiver.try_recv() {
                return Some(event);
            }
            let producer = self.producer.as_mut()?;
            tokio::select! {
                biased;
                event = self.receiver.recv() => { if event.is_some() { return event; } }
                outcome = producer => { self.outcome = Some(outcome); self.producer = None; }
            }
        }
    }
    pub async fn finish(mut self) -> Result<RunOutcome, RunError> {
        while self.next().await.is_some() {}
        self.outcome.take().expect("producer completed")
    }
}

impl Runner {
    pub fn new(agent: AgentConfig, mut config: RunnerConfig) -> Result<Self, Error> {
        validate_agent(&agent, &mut HashSet::new())?;
        config.output.work_dir = Some(config.work_dir.clone());
        config.local_compaction = config.local_compaction.normalized();
        if config.stop_gate_max_blocks == 0 {
            config.stop_gate_max_blocks = 8;
        }
        if let Some(max) = config.limits.max_cost {
            if !max.is_finite() || max < 0.0 || config.cost_estimator.is_none() {
                return Err(Error::new(
                    ErrorCategory::InvalidInput,
                    "cost limit requires a finite nonnegative value and an estimator",
                ));
            }
        }
        if let Some(c) = &config.compaction {
            if c.target_tokens >= c.trigger_tokens || c.target_tokens == 0 {
                return Err(Error::new(
                    ErrorCategory::InvalidInput,
                    "compaction requires 0 < target < trigger",
                ));
            }
        }
        Ok(Self {
            initial: Arc::new(agent),
            config: Arc::new(config),
        })
    }
    pub async fn run(
        &self,
        context: Context,
        request: RunRequest,
        host: Arc<dyn Host>,
    ) -> Result<RunOutcome, RunError> {
        self.engine(context, request, host).drive().await
    }
    pub fn stream(&self, context: Context, request: RunRequest, host: Arc<dyn Host>) -> RunStream {
        RunStream::new(self.engine(context, request, host))
    }
    fn engine(&self, context: Context, request: RunRequest, host: Arc<dyn Host>) -> Engine {
        Engine {
            agent: self.initial.clone(),
            config: self.config.clone(),
            context,
            host,
            base_turn_limit: request.policy.max_turns,
            policy: request.policy,
            phase: Phase::Start,
            calls: VecDeque::new(),
            approvals: HashMap::new(),
            denial_reasons: HashMap::new(),
            deferred_calls: VecDeque::new(),
            pending_marker_events: Vec::new(),
            approval_journal: crate::compat::ApprovalJournal::default(),
            tools_prepared: false,
            tool_turn_start: None,
            consecutive_tool_errors: 0,
            tool_error_escalated: false,
            stop_gate_blocks: 0,
            turns: 0,
            committed_cursor: 0,
            committed_markers: 0,
            calibration: EstimateCalibration::default(),
            fallbacks: HashMap::new(),
            cost: 0.0,
            tool_pause: false,
            tool_final: None,
            streaming: false,
            sender: None,
            spills: vec![],
            result: RunResult {
                status: RunStatus::Incomplete,
                final_output: None,
                new_items: vec![],
                history: request.input,
                responses: vec![],
                usage: Usage::default(),
                pending_approvals: vec![],
                last_agent: Some(self.initial.name.clone()),
            },
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Start,
    Model,
    Tools,
    Finish,
}

struct ExecutedTool {
    raw: ToolOutput,
    hook_error: Option<Error>,
}

struct Engine {
    agent: Arc<AgentConfig>,
    config: Arc<RunnerConfig>,
    context: Context,
    host: Arc<dyn Host>,
    policy: RunPolicy,
    base_turn_limit: std::num::NonZeroU32,
    phase: Phase,
    calls: VecDeque<ToolCall>,
    approvals: HashMap<String, ApprovalDecision>,
    denial_reasons: HashMap<String, String>,
    deferred_calls: VecDeque<ToolCall>,
    pending_marker_events: Vec<Observation>,
    approval_journal: crate::compat::ApprovalJournal,
    tools_prepared: bool,
    tool_turn_start: Option<usize>,
    consecutive_tool_errors: usize,
    tool_error_escalated: bool,
    stop_gate_blocks: usize,
    turns: u32,
    committed_cursor: usize,
    committed_markers: usize,
    calibration: EstimateCalibration,
    fallbacks: HashMap<usize, (usize, u32)>,
    cost: f64,
    tool_pause: bool,
    tool_final: Option<String>,
    streaming: bool,
    sender: Option<mpsc::Sender<RunEvent>>,
    spills: Vec<Arc<SpillFile>>,
    result: RunResult,
}

fn validate_agent(agent: &AgentConfig, seen: &mut HashSet<usize>) -> Result<(), Error> {
    if !seen.insert(agent as *const AgentConfig as usize) {
        return Ok(());
    }
    let mut names = HashSet::new();
    for definition in agent
        .tools
        .iter()
        .map(|t| t.definition())
        .chain(agent.handoffs.iter().map(|h| &h.definition))
    {
        if !names.insert(&definition.name) {
            return Err(Error::new(
                ErrorCategory::InvalidInput,
                format!("duplicate tool or handoff: {}", definition.name),
            ));
        }
        compile_schema(&definition.input_schema)?;
    }
    if let Some(schema) = &agent.output_schema {
        compile_schema(schema)?;
    }
    for handoff in &agent.handoffs {
        validate_agent(&handoff.target, seen)?;
    }
    Ok(())
}

fn compile_schema(schema: &schemars::Schema) -> Result<jsonschema::Validator, Error> {
    jsonschema::validator_for(schema.as_value()).map_err(|e| {
        Error::new(
            ErrorCategory::InvalidInput,
            format!("invalid JSON schema: {e}"),
        )
    })
}

#[derive(Debug)]
struct OperationTimeout;
impl std::fmt::Display for OperationTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("local operation timeout")
    }
}
impl std::error::Error for OperationTimeout {}

async fn bounded<T>(
    context: &Context,
    timeout: Option<Duration>,
    future: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    context.check_active()?;
    let deadline = context.deadline;
    tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => Err(Error::new(ErrorCategory::Cancelled, "operation cancelled")),
        _ = async { match deadline { Some(d) => tokio::time::sleep_until(d.into()).await, None => std::future::pending().await } } => Err(Error::new(ErrorCategory::DeadlineExceeded, "deadline exceeded")),
        _ = async { match timeout { Some(d) => tokio::time::sleep(d).await, None => std::future::pending().await } } => Err(Error::new(ErrorCategory::DeadlineExceeded, "operation inactivity/timeout limit exceeded").with_source(OperationTimeout)),
        result = future => result,
    }
}

fn go_duration(duration: Duration) -> String {
    let ns = duration.as_nanos();
    if ns == 0 {
        return "0s".into();
    }
    if ns < 1_000_000_000 {
        let (scale, digits, unit) = if ns < 1_000 {
            (1, 0, "ns")
        } else if ns < 1_000_000 {
            (1_000, 3, "µs")
        } else {
            (1_000_000, 6, "ms")
        };
        let fraction = format!("{:0width$}", ns % scale, width = digits);
        let fraction = fraction.trim_end_matches('0');
        return if fraction.is_empty() {
            format!("{}{unit}", ns / scale)
        } else {
            format!("{}.{fraction}{unit}", ns / scale)
        };
    }
    let seconds = duration.as_secs();
    let mut result = String::new();
    if seconds >= 3600 {
        result.push_str(&format!("{}h", seconds / 3600));
    }
    if seconds >= 60 {
        result.push_str(&format!("{}m", seconds / 60 % 60));
    }
    result.push_str(&(seconds % 60).to_string());
    let fraction = format!("{:09}", duration.subsec_nanos());
    let fraction = fraction.trim_end_matches('0');
    if !fraction.is_empty() {
        result.push('.');
        result.push_str(fraction);
    }
    result.push('s');
    result
}

fn text(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|c| {
            if let Content::Text { text } = c {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

impl Engine {
    async fn emit(&self, event: RunEvent) -> Result<(), Error> {
        if let RunEvent::Model {
            event: ModelEvent::TextDelta { delta },
        } = &event
        {
            self.observe(Observation::TextDelta {
                delta: delta.clone(),
            })
            .await?;
        }
        bounded(
            &self.context,
            None,
            self.host.emit(&self.context, event.clone()),
        )
        .await?;
        if let Some(sender) = &self.sender {
            bounded(&self.context, None, async {
                sender
                    .send(event)
                    .await
                    .map_err(|_| Error::new(ErrorCategory::Cancelled, "event consumer dropped"))
            })
            .await?;
        }
        Ok(())
    }
    async fn observe(&self, observation: Observation) -> Result<(), Error> {
        self.approval_journal
            .observe(&self.context, observation.clone())
            .await?;
        let hooks = if matches!(
            observation,
            Observation::AgentEnded { .. } | Observation::Handoff { .. }
        ) {
            [&self.agent.hooks, &self.config.hooks]
        } else {
            [&self.config.hooks, &self.agent.hooks]
        };
        for hooks in hooks.into_iter().flatten() {
            bounded(
                &self.context,
                None,
                hooks.observe(&self.context, observation.clone()),
            )
            .await?;
        }
        Ok(())
    }
    async fn checkpoint(
        &self,
        boundary: Boundary,
        pending: Option<&ToolCall>,
    ) -> Result<(), Error> {
        if let Some(hook) = &self.config.durable {
            bounded(
                &self.context,
                None,
                hook.checkpoint(&self.context, boundary, &self.result, pending),
            )
            .await?;
        }
        Ok(())
    }
    fn append(&mut self, item: RunItem) {
        self.result.history.push(item.clone());
        self.result.new_items.push(item);
    }
    async fn publish_committed(&mut self) -> Result<(), Error> {
        let entries = self.approval_journal.entries();
        if self.committed_cursor == self.result.new_items.len()
            && self.committed_markers == entries.len()
        {
            return Ok(());
        }
        let items = self.result.new_items[self.committed_cursor..].to_vec();
        let agents = items
            .iter()
            .map(|item| {
                let no_agent = match item {
                    RunItem::Message { message } => message.role == Role::User,
                    RunItem::ToolResult { call_id, .. } => entries.iter().any(|entry| {
                        entry.marker.phase == adk_codec::approval::ApprovalPhase::Denied
                            && entry.marker.agent.is_none()
                            && entry.marker.data.call_id == *call_id
                    }),
                    _ => false,
                };
                (!no_agent).then(|| adk_codec::dto::AgentRef {
                    name: self.agent.name.clone(),
                })
            })
            .collect();
        let mut markers = entries[self.committed_markers..]
            .iter()
            .map(|entry| {
                let before_item = entry
                    .new_items_before
                    .checked_sub(self.committed_cursor)
                    .ok_or_else(|| {
                        Error::new(
                            ErrorCategory::Internal,
                            "late approval marker behind committed cursor",
                        )
                    })?;
                Ok(ApprovalMarkerBoundary {
                    before_item,
                    marker: entry.marker.clone(),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        markers.sort_by_key(|marker| marker.before_item);
        self.observe(Observation::CommittedItems {
            items,
            agents,
            markers,
        })
        .await?;
        self.committed_cursor = self.result.new_items.len();
        self.committed_markers = entries.len();
        Ok(())
    }
    async fn drive(mut self) -> Result<RunOutcome, RunError> {
        let execution = async {
            let status = self.advance().await?;
            self.publish_committed().await?;
            self.result.status = status;
            self.checkpoint(
                if status == RunStatus::Paused {
                    Boundary::Paused
                } else {
                    Boundary::Completed
                },
                None,
            )
            .await?;
            self.emit(RunEvent::Finished {
                result: self.result.clone(),
            })
            .await?;
            Ok::<_, Error>(status)
        }
        .await;
        match execution {
            Ok(RunStatus::Paused) => {
                self.sender.take();
                Ok(RunOutcome {
                    result: self.result.clone(),
                    spills: self.spills.clone(),
                    continuation: Some(Continuation { engine: self }),
                })
            }
            Ok(_) => Ok(RunOutcome {
                result: self.result,
                spills: self.spills,
                continuation: None,
            }),
            Err(error) => Err(self.fail(error).await),
        }
    }
    async fn fail(mut self, mut error: Error) -> RunError {
        self.result.status = RunStatus::Incomplete;
        self.result.final_output = None;
        // Preserve the original failure even when the failed-event sink fails.
        let _ = self
            .emit(RunEvent::Failed {
                error: error.info.clone(),
            })
            .await;
        if !self.spills.is_empty() {
            error.source = Some(Box::new(FailedSpills {
                source: error.source.take(),
                _spills: self.spills,
            }));
        }
        RunError::with_partial(error, self.result)
    }

    async fn advance(&mut self) -> Result<RunStatus, Error> {
        loop {
            self.context.check_active()?;
            match self.phase {
                Phase::Start => {
                    validate_history_pairs(&self.result.history)?;
                    self.checkpoint(Boundary::Started, None).await?;
                    self.emit(RunEvent::Started {
                        agent: self.agent.name.clone(),
                    })
                    .await?;
                    self.phase = Phase::Model;
                }
                Phase::Model => self.model_turn().await?,
                Phase::Tools => {
                    if !self.tools_prepared {
                        self.prepare_tools().await?;
                        self.tools_prepared = true;
                    }
                    if self.parallel_batch_ready() {
                        self.tool_batch().await?;
                    } else if let Some(call) = self.calls.front().cloned() {
                        if self.tool(call).await? {
                            return Ok(RunStatus::Paused);
                        }
                    } else if !self.deferred_calls.is_empty() {
                        self.calls = std::mem::take(&mut self.deferred_calls);
                        for event in std::mem::take(&mut self.pending_marker_events) {
                            self.observe(event).await?;
                        }
                        self.settle_tool_turn();
                        for request in &self.result.pending_approvals {
                            self.checkpoint(Boundary::ApprovalPending, Some(&request.call))
                                .await?;
                        }
                        return Ok(RunStatus::Paused);
                    } else if self.policy.tool_use == ToolUseBehavior::StopAfterTool
                        && self.tool_final.is_some()
                    {
                        self.settle_tool_turn();
                        if self.config.return_tool_output {
                            let output = self.tool_final.take().unwrap();
                            self.result.final_output = Some(self.validate_output(output).await?);
                        }
                        self.phase = Phase::Finish;
                    } else {
                        self.settle_tool_turn();
                        self.phase = Phase::Model;
                        if std::mem::take(&mut self.tool_pause) {
                            return Ok(RunStatus::Paused);
                        }
                    }
                }
                Phase::Finish => return Ok(RunStatus::Completed),
            }
        }
    }
    fn check_budget(&self) -> Result<(), Error> {
        let tokens = self
            .result
            .usage
            .input_tokens
            .saturating_add(self.result.usage.output_tokens);
        if self
            .config
            .limits
            .max_tokens
            .is_some_and(|max| tokens >= max)
        {
            return Err(Error::new(
                ErrorCategory::Guardrail,
                "run token budget exhausted",
            ));
        }
        if self
            .config
            .limits
            .max_cost
            .is_some_and(|max| self.cost >= max)
        {
            return Err(Error::new(
                ErrorCategory::Guardrail,
                "run cost budget exhausted",
            ));
        }
        Ok(())
    }
    async fn compact(&mut self) -> Result<(), Error> {
        let Some(config) = &self.config.compaction else {
            return Ok(());
        };
        let Some(tokens) = self.result.usage.context_tokens else {
            return Ok(());
        };
        if tokens < config.trigger_tokens {
            return Ok(());
        }
        let before_items = self.result.history.len();
        let request = CompactionRequest {
            agent: self.agent.name.clone(),
            model: self
                .fallbacks
                .get(&(Arc::as_ptr(&self.agent) as usize))
                .map_or_else(
                    || self.agent.model.name(),
                    |(index, _)| self.agent.fallbacks[index - 1].name(),
                )
                .into(),
            history: self.result.history.clone(),
            context_tokens: tokens,
            target_tokens: config.target_tokens,
        };
        self.observe(Observation::CompactionStarted {
            context_tokens: tokens,
            target_tokens: config.target_tokens,
        })
        .await?;
        let compacted = match bounded(
            &self.context,
            self.config.model_idle_timeout,
            config.compactor.compact(&self.context, request),
        )
        .await
        {
            Ok(compacted) => compacted,
            Err(error) => {
                self.context.check_active()?;
                self.observe(Observation::CompactionFailed {
                    error: error.info.clone(),
                })
                .await?;
                if matches!(
                    error.info.category,
                    ErrorCategory::Provider | ErrorCategory::Unsupported
                ) || error
                    .source
                    .as_deref()
                    .is_some_and(|source| source.is::<OperationTimeout>())
                {
                    // Request preparation still runs the baseline local planner after this failure.
                    return Ok(());
                }
                return Err(error);
            }
        };
        self.account_usage(&compacted.usage, compacted.cost)?;
        self.observe(Observation::Usage {
            usage: self.result.usage.clone(),
            cost: self.cost,
        })
        .await?;
        validate_history_pairs(&compacted.history)?;
        self.observe(Observation::HistoryReplaced {
            before: self.result.history.clone(),
            after: compacted.history.clone(),
        })
        .await?;
        self.result.history = compacted.history;
        self.result.usage.context_tokens = Some(compacted.context_tokens);
        self.observe(Observation::Compacted {
            before_items,
            after_items: self.result.history.len(),
            context_tokens: compacted.context_tokens,
        })
        .await
    }
    async fn compact_local(
        &mut self,
        request: &mut ModelRequest,
        forced: bool,
    ) -> Result<bool, Error> {
        let mut policy = self.config.local_compaction;
        if policy == LocalCompactionPolicy::default() {
            policy = LocalCompactionPolicy::for_model(&request.model);
        }
        let mut policy = self.calibration.apply(policy);
        let markers = self
            .approval_journal
            .history_markers()
            .map_err(|error| Error::new(ErrorCategory::Internal, error.to_string()))?;
        let transient = &request.input[self.result.history.len()..];
        let overhead =
            estimate_request_overhead_tokens(request) + estimate_history_tokens(transient);
        let history_estimate =
            estimate_history_tokens_with_approvals(&self.result.history, &markers);
        if forced {
            policy.trigger_tokens = 1;
            policy.target_tokens = policy.target_tokens.min((history_estimate / 2).max(1));
        }
        let before = history_estimate + overhead;
        if !policy.enabled || before <= policy.trigger_tokens {
            return Ok(false);
        }
        self.observe(Observation::CompactionStarted {
            context_tokens: before,
            target_tokens: policy.target_tokens,
        })
        .await?;
        let outcome = compact_with_approvals(
            &self.result.history,
            &markers,
            policy,
            if forced {
                0
            } else {
                overhead.min(i64::MAX as u64) as i64
            },
        );
        if !outcome.changed {
            if !matches!(outcome.reason, "disabled" | "below-threshold") {
                self.observe(Observation::CompactionFailed {
                    error: ErrorInfo {
                        category: ErrorCategory::ModelBehavior,
                        message: outcome.reason.into(),
                    },
                })
                .await?;
            }
            return Ok(false);
        }
        let (history, markers) = finalize_local_history_with_approvals(
            &outcome.history,
            &outcome.markers,
            &self.result.history,
            &markers,
        );
        validate_history_pairs(&history)?;
        let before_items = self.result.history.len();
        let context_tokens = estimate_history_tokens_with_approvals(&history, &markers) + overhead;
        let mut input = history.clone();
        input.extend_from_slice(transient);
        request.input = input;
        self.observe(Observation::ApprovalHistoryReplaced {
            before: self.result.history.clone(),
            after: history.clone(),
            markers,
        })
        .await?;
        self.result.history = history;
        self.result.usage.context_tokens = Some(context_tokens);
        self.observe(Observation::Compacted {
            before_items,
            after_items: self.result.history.len(),
            context_tokens,
        })
        .await?;
        Ok(true)
    }
    fn settle_tool_turn(&mut self) {
        let Some(start) = self.tool_turn_start.take() else {
            return;
        };
        let Some(limit) = self
            .config
            .consecutive_tool_error_limit
            .filter(|limit| *limit > 0)
        else {
            return;
        };
        let errors: Vec<_> = self.result.new_items[start..]
            .iter()
            .filter_map(|item| match item {
                RunItem::ToolResult { output, .. } => Some(output.is_error),
                _ => None,
            })
            .collect();
        if errors.is_empty() {
            return;
        }
        if errors.iter().all(|error| *error) {
            self.consecutive_tool_errors = self.consecutive_tool_errors.saturating_add(1);
        } else {
            self.consecutive_tool_errors = 0;
            self.tool_error_escalated = false;
        }
        if self.consecutive_tool_errors >= limit && !self.tool_error_escalated {
            self.tool_error_escalated = true;
            self.append(RunItem::Message { message: Message { role: Role::User, content: vec![Content::Text { text: format!("[SYSTEM] Your last {} tool turns all failed. Stop repeating the same approach. Re-read the error messages above carefully, then either: (1) try a fundamentally different approach or tool, (2) inspect the environment to understand why the calls fail, or (3) if the task is genuinely blocked, report the blocker and what you tried instead of retrying.", self.consecutive_tool_errors) }] } });
        }
    }
    async fn model_turn(&mut self) -> Result<(), Error> {
        self.settle_tool_turn();
        self.publish_committed().await?;
        if self.turns >= self.policy.max_turns.get() {
            return Err(Error::new(
                ErrorCategory::MaxTurns,
                "maximum model turns exceeded",
            ));
        }
        self.check_budget()?;
        self.compact().await?;
        self.check_budget()?;
        let mut input = self.result.history.clone();
        input.extend(self.config.transient_context.clone());
        if let Some(hints) = &self.config.turn_context {
            input.extend(
                bounded(
                    &self.context,
                    None,
                    hints.context(&self.context, &self.result, self.turns + 1),
                )
                .await?,
            );
        }
        let tools = self
            .agent
            .tools
            .iter()
            .map(|t| t.definition())
            .chain(self.agent.handoffs.iter().map(|h| &h.definition))
            .filter(|d| self.policy.tools.decision(d) != ToolDecision::Deny)
            .cloned()
            .collect();
        let mut instructions = if self.config.cache_prefix.is_empty() {
            self.agent.instructions.clone()
        } else {
            format!("{}\n{}", self.config.cache_prefix, self.agent.instructions)
        };
        if let Some(schema) = &self.agent.output_schema {
            if !instructions.trim().is_empty() {
                instructions.push_str("\n\n---\n\n");
            }
            let name = self.agent.output_schema_name.trim();
            let name = if name.is_empty() {
                "final_output"
            } else {
                name
            };
            let strict = if self.agent.output_schema_strict {
                "\nStrict mode: do not include prose, markdown fences, or fields outside the schema."
            } else {
                ""
            };
            instructions.push_str(&format!("<structured_output>\nWhen producing a final answer, return JSON only.\nOutput schema name: {name}{strict}\nJSON schema:\n{}\n</structured_output>", schema.as_value()));
        }
        let mut settings = self.agent.settings.clone();
        if let Some(key) = &self.config.prompt_cache_key {
            let namespace = self
                .config
                .prompt_cache_namespace
                .as_deref()
                .unwrap_or(&self.context.run_id);
            let mut hash = Sha256::new();
            hash.update(namespace);
            hash.update([0]);
            hash.update(key);
            settings.insert(
                "prompt_cache_key".into(),
                Value::String(format!("{:x}", hash.finalize())),
            );
        }
        let mut request = ModelRequest {
            model: self
                .fallbacks
                .get(&(Arc::as_ptr(&self.agent) as usize))
                .map_or_else(
                    || self.agent.model.name(),
                    |(index, _)| self.agent.fallbacks[index - 1].name(),
                )
                .into(),
            instructions,
            input,
            tools,
            output_schema: self.agent.output_schema.clone(),
            output_schema_name: self.agent.output_schema_name.clone(),
            output_schema_strict: self.agent.output_schema_strict,
            settings,
        };
        self.compact_local(&mut request, false).await?;
        self.checkpoint(Boundary::ModelPrepared, None).await?;
        let (response, model, streamed) = self.model_response(request).await?;
        self.observe(Observation::ModelAccepted {
            agent: self.agent.name.clone(),
            response: response.clone(),
        })
        .await?;
        self.record_response(response.clone(), &model)?;
        self.publish_committed().await?;
        if !streamed {
            self.emit(RunEvent::Model {
                event: ModelEvent::Complete {
                    response: response.clone(),
                },
            })
            .await?;
        }
        self.checkpoint(Boundary::ModelCompleted, None).await?;
        self.observe(Observation::Usage {
            usage: self.result.usage.clone(),
            cost: self.cost,
        })
        .await?;
        self.check_budget()?;
        let mut ids = HashSet::new();
        for item in &response.items {
            match item {
                RunItem::ToolCall { call } => {
                    if call.id.is_empty() || !ids.insert(call.id.clone()) || self.result.history[..self.result.history.len() - response.items.len()].iter().any(|i| matches!(i, RunItem::ToolCall { call: prior } if prior.id == call.id)) {
                        return Err(Error::new(ErrorCategory::ModelBehavior, "duplicate tool call ID"));
                    }
                    self.calls.push_back(call.clone());
                }
                RunItem::Message { message } if message.role == Role::Assistant => {}
                _ => {
                    return Err(Error::new(
                        ErrorCategory::ModelBehavior,
                        "model returned a non-assistant item",
                    ));
                }
            }
        }
        if let Some(index) = self.calls.iter().position(|call| {
            self.agent
                .handoffs
                .iter()
                .any(|h| h.definition.name == call.name)
        }) {
            let handoff = self.calls.remove(index).unwrap();
            self.calls.push_front(handoff);
        }
        if !self.calls.is_empty() {
            self.stop_gate_blocks = 0;
            self.tool_turn_start = Some(self.result.new_items.len());
            self.tool_pause = false;
            self.tool_final = None;
            self.tools_prepared = false;
            self.phase = Phase::Tools;
        } else if response.end_turn == Some(false) {
            self.stop_gate_blocks = 0;
            self.phase = Phase::Model;
        } else {
            let output = response
                .items
                .iter()
                .rev()
                .filter_map(|i| match i {
                    RunItem::Message { message } => Some(text(&message.content)),
                    _ => None,
                })
                .find(|t| !t.is_empty())
                .unwrap_or_default();
            let output = self.validate_output(output).await?;
            if let Some(gate) = &self.config.stop_gate {
                let has_tools = self
                    .agent
                    .tools
                    .iter()
                    .any(|tool| self.tool_decision(tool.definition()) != ToolDecision::Deny)
                    || !self.agent.handoffs.is_empty();
                if has_tools && self.stop_gate_blocks < self.config.stop_gate_max_blocks.max(1) {
                    if let Some(mut feedback) =
                        bounded(&self.context, None, gate.check(&self.context, &output)).await?
                    {
                        self.stop_gate_blocks += 1;
                        if feedback.trim().is_empty() {
                            feedback =
                                "the finalization check failed; continue working until it passes"
                                    .into();
                        }
                        self.append(RunItem::Message { message: Message { role: Role::User, content: vec![Content::Text { text: format!("[SYSTEM] Final answer blocked by the completion gate ({}/{}):\n{}", self.stop_gate_blocks, self.config.stop_gate_max_blocks.max(1), feedback) }] } });
                        if self.turns >= self.policy.max_turns.get() {
                            self.policy.max_turns = self.policy.max_turns.saturating_add(1);
                        }
                        self.phase = Phase::Model;
                        return Ok(());
                    }
                    self.stop_gate_blocks = 0;
                }
            }
            self.result.final_output = Some(output.clone());
            self.observe(Observation::AgentEnded {
                agent: self.agent.name.clone(),
                output,
            })
            .await?;
            self.phase = Phase::Finish;
        }
        Ok(())
    }
    async fn validate_output(&self, output: String) -> Result<Value, Error> {
        if let Some(schema) = &self.agent.output_schema {
            if let Some(parser) = &self.agent.output_parser {
                return match parser.parse(&output) {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        self.observe(Observation::OutputValidationFailed {
                            message: error.to_string(),
                        })
                        .await?;
                        Ok(Value::String(output))
                    }
                };
            }
            match serde_json::from_str::<Value>(&output) {
                Ok(value) => {
                    if !compile_schema(schema)?.is_valid(&value) {
                        self.observe(Observation::OutputValidationFailed {
                            message: "output does not match its JSON schema".into(),
                        })
                        .await?;
                    }
                    Ok(value)
                }
                Err(error) => {
                    self.observe(Observation::OutputValidationFailed {
                        message: format!("output is not JSON: {error}"),
                    })
                    .await?;
                    Ok(Value::String(output))
                }
            }
        } else {
            Ok(Value::String(output))
        }
    }
    fn account_usage(&mut self, used: &Usage, cost: f64) -> Result<(), Error> {
        let usage = &mut self.result.usage;
        usage.input_tokens = usage.input_tokens.saturating_add(used.input_tokens);
        usage.output_tokens = usage.output_tokens.saturating_add(used.output_tokens);
        usage.cache_read_tokens = usage
            .cache_read_tokens
            .saturating_add(used.cache_read_tokens);
        usage.cache_creation_tokens = usage
            .cache_creation_tokens
            .saturating_add(used.cache_creation_tokens);
        usage.context_tokens = used.context_tokens;
        if !cost.is_finite() || cost < 0.0 || !(self.cost + cost).is_finite() {
            return Err(Error::new(
                ErrorCategory::InvalidInput,
                "invalid provider cost",
            ));
        }
        self.cost += cost;
        Ok(())
    }
    fn record_response(&mut self, response: ModelResponse, model: &str) -> Result<(), Error> {
        for item in &response.items {
            self.append(item.clone());
        }
        let cost = self
            .config
            .cost_estimator
            .as_ref()
            .map(|c| c.cost(model, &response.usage))
            .unwrap_or(0.0);
        let accounting = self.account_usage(&response.usage, cost);
        self.result.responses.push(response);
        accounting
    }
    async fn model_response(
        &mut self,
        mut request: ModelRequest,
    ) -> Result<(ModelResponse, String, bool), Error> {
        let candidates: Vec<_> = std::iter::once(self.agent.model.clone())
            .chain(self.agent.fallbacks.clone())
            .collect();
        let agent_key = Arc::as_ptr(&self.agent) as usize;
        let start = self.fallbacks.get(&agent_key).map_or(0, |state| state.0);
        let mut attempt = 0;
        for (index, binding) in candidates.iter().enumerate().skip(start) {
            loop {
                if self.turns >= self.policy.max_turns.get() {
                    return Err(Error::new(
                        ErrorCategory::MaxTurns,
                        "maximum model turns exceeded",
                    ));
                }
                self.turns += 1;
                self.observe(Observation::AgentStarted {
                    agent: self.agent.name.clone(),
                })
                .await?;
                self.observe(Observation::ModelAttempt {
                    agent: self.agent.name.clone(),
                    model: binding.name().into(),
                    attempt,
                })
                .await?;
                if request.model != binding.name() {
                    request.model = binding.name().into();
                    self.compact_local(&mut request, false).await?;
                }
                let markers = self
                    .approval_journal
                    .history_markers()
                    .map_err(|error| Error::new(ErrorCategory::Internal, error.to_string()))?;
                let estimated_prompt =
                    estimate_history_tokens_with_approvals(&request.input, &markers)
                        + estimate_request_overhead_tokens(&request)
                        - output_reserve_tokens(&request)
                        - REQUEST_SAFETY_BUFFER;
                match self.model_attempt(binding, request.clone()).await {
                    Ok(response) => {
                        self.calibration.observe_estimate(
                            response
                                .usage
                                .context_tokens
                                .unwrap_or(response.usage.input_tokens),
                            estimated_prompt,
                        );
                        if let Some((_, successes)) = self.fallbacks.get_mut(&agent_key) {
                            *successes += 1;
                            if *successes == 3 {
                                self.fallbacks.remove(&agent_key);
                            }
                        }
                        return Ok((
                            response,
                            binding.name().into(),
                            self.streaming && matches!(binding, ModelBinding::Streaming { .. }),
                        ));
                    }
                    Err((error, committed)) => {
                        self.context.check_active()?;
                        let idle = error
                            .source
                            .as_deref()
                            .is_some_and(|source| source.is::<OperationTimeout>());
                        if committed
                            || error.info.category == ErrorCategory::Cancelled
                            || (error.info.category == ErrorCategory::DeadlineExceeded && !idle)
                        {
                            return Err(error);
                        }
                        let message = error.info.message.to_lowercase();
                        if (message.contains("context_length_exceeded")
                            || message.contains("exceeds the context window"))
                            && self.compact_local(&mut request, true).await?
                        {
                            continue;
                        }
                        attempt += 1;
                        let advice = match binding {
                            ModelBinding::Complete { model, .. } => model.retry_advice(&error),
                            ModelBinding::Streaming { model, .. } => model.retry_advice(&error),
                        };
                        if let Some(next) = candidates.get(index + 1).filter(|_| {
                            error.info.category != ErrorCategory::ModelBehavior
                                && (idle
                                    || advice.as_ref().is_some_and(|advice| {
                                        let reason = advice.reason.trim().to_ascii_lowercase();
                                        advice.should_retry
                                            && (matches!(
                                                reason.as_str(),
                                                "429" | "402" | "503" | "529"
                                            ) || [
                                                "rate_limit",
                                                "too_many_requests",
                                                "too many requests",
                                                "overloaded",
                                                "quota",
                                                "billing",
                                                "subscription",
                                                "credit",
                                                "limit_exceeded",
                                                "exhausted",
                                            ]
                                            .iter()
                                            .any(|part| reason.contains(part)))
                                    }))
                        }) {
                            self.fallbacks.insert(agent_key, (index + 1, 0));
                            self.observe(Observation::Fallback {
                                from: binding.name().into(),
                                to: next.name().into(),
                            })
                            .await?;
                            break;
                        }
                        if let Some(handler) = &self.config.error_handler {
                            match handler.handle(&self.agent.name, self.turns - 1, &error) {
                                ModelErrorAction::Retry | ModelErrorAction::Continue => continue,
                                ModelErrorAction::Abort => return Err(error),
                            }
                        }
                        let policy_retry = attempt <= self.config.retry.max_retries
                            && (idle || (self.config.retry.retryable)(&error))
                            && !advice.as_ref().is_some_and(|advice| {
                                !advice.should_retry && !advice.reason.trim().is_empty()
                            });
                        let advised_retry =
                            advice.as_ref().is_some_and(|advice| advice.should_retry)
                                && attempt <= 10;
                        if !policy_retry && !advised_retry {
                            return Err(error);
                        }
                        let policy_delay = if attempt == 1 {
                            self.config.retry.initial_delay
                        } else {
                            self.config
                                .retry
                                .initial_delay
                                .saturating_mul(2u32.saturating_pow(attempt - 1))
                                .min(if self.config.retry.max_delay.is_zero() {
                                    Duration::from_secs(30)
                                } else {
                                    self.config.retry.max_delay
                                })
                        };
                        let advised_delay = advice
                            .as_ref()
                            .map_or(Duration::ZERO, |advice| advice.retry_after);
                        let delay = if policy_retry {
                            policy_delay.max(advised_delay)
                        } else if !advised_delay.is_zero() {
                            advised_delay
                        } else if !self.config.retry.initial_delay.is_zero() {
                            policy_delay
                        } else {
                            Duration::from_secs(1)
                                .saturating_mul(2u32.saturating_pow(attempt - 1))
                                .min(Duration::from_secs(30))
                        }
                        .min(Duration::from_secs(300));
                        self.observe(Observation::Retry {
                            model: binding.name().into(),
                            delay,
                        })
                        .await?;
                        bounded(&self.context, None, async {
                            tokio::time::sleep(delay).await;
                            Ok(())
                        })
                        .await?;
                    }
                }
            }
        }
        unreachable!("primary model is always present")
    }
    async fn model_attempt(
        &mut self,
        binding: &ModelBinding,
        request: ModelRequest,
    ) -> Result<ModelResponse, (Error, bool)> {
        let timeout = self.config.model_idle_timeout;
        match binding {
            ModelBinding::Complete { model, .. } => bounded(
                &self.context,
                timeout,
                model.complete(&self.context, request),
            )
            .await
            .map_err(|e| (e, false)),
            ModelBinding::Streaming { model, .. } if !self.streaming => bounded(
                &self.context,
                timeout,
                model.complete(&self.context, request),
            )
            .await
            .map_err(|e| (e, false)),
            ModelBinding::Streaming { model, .. } => {
                let context = self.context.clone();
                let mut stream = bounded(&context, timeout, model.stream(&context, request))
                    .await
                    .map_err(|e| (e, false))?;
                let mut complete = None;
                let mut items = vec![];
                let mut delta_text = String::new();
                let mut committed = false;
                let result = loop {
                    match bounded(&context, timeout, stream.next()).await {
                        Ok(Some(event)) => {
                            if complete.is_some() {
                                break Err(Error::new(
                                    ErrorCategory::ModelBehavior,
                                    "model event after Complete",
                                ));
                            }
                            committed = true;
                            match &event {
                                ModelEvent::Complete { response } => {
                                    complete = Some(response.clone())
                                }
                                ModelEvent::ItemDone { item } => {
                                    if let RunItem::Message { message } = item {
                                        let finished_text = text(&message.content);
                                        if !finished_text.is_empty() && finished_text == delta_text
                                        {
                                            delta_text.clear();
                                        }
                                    }
                                    items.push(item.clone());
                                }
                                ModelEvent::TextDelta { delta } => delta_text.push_str(delta),
                                _ => {}
                            }
                            if let Err(error) = self.emit(RunEvent::Model { event }).await {
                                break Err(error);
                            }
                        }
                        Ok(None) if complete.is_some() => break Ok(complete.take().unwrap()),
                        Ok(None) => {
                            break Err(Error::new(
                                ErrorCategory::ModelBehavior,
                                "model stream ended without Complete",
                            ));
                        }
                        Err(error) => break Err(error),
                    }
                };
                match result {
                    Ok(response) => Ok(response),
                    Err(error) => {
                        if let Some(response) = complete {
                            let _ = self.record_response(response, binding.name());
                        } else {
                            if !delta_text.is_empty() {
                                items.push(RunItem::Message {
                                    message: Message {
                                        role: Role::Assistant,
                                        content: vec![Content::Text { text: delta_text }],
                                    },
                                });
                            }
                            for item in items {
                                self.append(item);
                            }
                        }
                        Err((error, committed))
                    }
                }
            }
        }
    }
    fn tool_decision(&self, definition: &ToolDefinition) -> ToolDecision {
        let decision = self.policy.tools.decision(definition);
        if decision == ToolDecision::Allow
            && self.config.approve_mutating_tools
            && !definition.read_only
            && self
                .agent
                .tools
                .iter()
                .any(|tool| tool.definition().name == definition.name && !tool.is_control_flow())
        {
            ToolDecision::RequireApproval
        } else {
            decision
        }
    }
    async fn prepare_tools(&mut self) -> Result<(), Error> {
        if self.calls.front().is_some_and(|call| {
            self.agent
                .handoffs
                .iter()
                .any(|handoff| handoff.definition.name == call.name)
        }) {
            return Ok(());
        }
        let mut ready = VecDeque::new();
        self.result.pending_approvals.clear();
        while let Some(call) = self.calls.pop_front() {
            let tool = self
                .agent
                .tools
                .iter()
                .find(|tool| tool.definition().name == call.name);
            if let Some(tool) = tool {
                let definition = tool.definition();
                if self.tool_decision(definition) == ToolDecision::RequireApproval {
                    if self.config.validate_tool_arguments
                        && !compile_schema(&definition.input_schema)?.is_valid(&call.arguments)
                    {
                        return Err(Error::new(
                            ErrorCategory::ModelBehavior,
                            format!("invalid arguments for {}", call.name),
                        ));
                    }
                    let request = ApprovalRequest {
                        call: call.clone(),
                        reason: "tool policy requires approval".into(),
                    };
                    let approval = if let Some(approval) = self.approvals.remove(&call.id) {
                        approval
                    } else {
                        self.emit(RunEvent::ApprovalRequired {
                            request: request.clone(),
                        })
                        .await?;
                        bounded(
                            &self.context,
                            None,
                            self.host.approve(&self.context, request.clone()),
                        )
                        .await?
                    };
                    if approval == ApprovalDecision::Defer {
                        self.pending_marker_events
                            .push(Observation::ApprovalMarker {
                                agent: Some(self.agent.name.clone()),
                                call: call.clone(),
                                decision: approval,
                                reason: None,
                                new_items_before: self.result.new_items.len() + ready.len(),
                                history_before: self.result.history.len() + ready.len(),
                            });
                        self.result.pending_approvals.push(request);
                        self.deferred_calls.push_back(call);
                        continue;
                    }
                    self.approvals.insert(call.id.clone(), approval);
                }
            }
            ready.push_back(call);
        }
        self.calls = ready;
        Ok(())
    }
    fn parallel_batch_ready(&self) -> bool {
        self.calls.len() > 1
            && self.calls.iter().all(|call| {
                self.agent.tools.iter().any(|tool| {
                    let definition = tool.definition();
                    definition.name == call.name
                        && self.tool_decision(definition) == ToolDecision::Allow
                        && (!self.config.validate_tool_arguments
                            || compile_schema(&definition.input_schema)
                                .is_ok_and(|schema| schema.is_valid(&call.arguments)))
                })
            })
    }
    async fn tool_batch(&mut self) -> Result<(), Error> {
        let calls: Vec<_> = self.calls.iter().cloned().collect();
        for call in &calls {
            self.checkpoint(Boundary::ToolPrepared, Some(call)).await?;
        }
        let mutation_lock = tokio::sync::RwLock::new(());
        let outputs = futures_util::future::join_all(calls.iter().map(|call| async {
            let tool = self
                .agent
                .tools
                .iter()
                .find(|tool| tool.definition().name == call.name)
                .unwrap();
            if tool.definition().read_only {
                let _guard = mutation_lock.read().await;
                self.execute_tool(tool.as_ref(), call).await
            } else {
                let _guard = mutation_lock.write().await;
                self.execute_tool(tool.as_ref(), call).await
            }
        }))
        .await;
        let mut failure = None;
        for (call, output) in calls.into_iter().zip(outputs) {
            let result = match output {
                Ok(raw) => self.finish_tool(call, raw).await,
                Err(error) => {
                    self.calls.pop_front();
                    Err(error)
                }
            };
            if let Err(error) = result {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    async fn tool(&mut self, call: ToolCall) -> Result<bool, Error> {
        let agent = self.agent.clone();
        let tool = agent
            .tools
            .iter()
            .find(|t| t.definition().name == call.name);
        let handoff = agent
            .handoffs
            .iter()
            .find(|h| h.definition.name == call.name);
        let definition = tool
            .map(|t| t.definition())
            .or_else(|| handoff.map(|h| &h.definition));
        if definition
            .is_none_or(|definition| self.policy.tools.decision(definition) == ToolDecision::Deny)
        {
            let output = ToolOutput {
                content: vec![Content::Text {
                    text: format!("unknown tool: {}", call.name),
                }],
                is_error: true,
                should_pause: false,
            };
            self.finish_tool(
                call,
                ExecutedTool {
                    raw: output,
                    hook_error: None,
                },
            )
            .await?;
            return Ok(false);
        }
        let definition = definition.unwrap();
        let decision = self.tool_decision(definition);
        if self.config.validate_tool_arguments
            && !compile_schema(&definition.input_schema)?.is_valid(&call.arguments)
        {
            return Err(Error::new(
                ErrorCategory::ModelBehavior,
                format!("invalid arguments for {}", call.name),
            ));
        }
        if decision == ToolDecision::RequireApproval {
            let request = ApprovalRequest {
                call: call.clone(),
                reason: "tool policy requires approval".into(),
            };
            let approval = if let Some(approval) = self.approvals.remove(&call.id) {
                approval
            } else {
                self.emit(RunEvent::ApprovalRequired {
                    request: request.clone(),
                })
                .await?;
                bounded(
                    &self.context,
                    None,
                    self.host.approve(&self.context, request.clone()),
                )
                .await?
            };
            self.observe(Observation::ApprovalMarker {
                agent: None,
                call: call.clone(),
                decision: approval,
                reason: self.denial_reasons.get(&call.id).cloned(),
                new_items_before: self.result.new_items.len(),
                history_before: self.result.history.len(),
            })
            .await?;
            match approval {
                ApprovalDecision::Approve => {
                    self.result
                        .pending_approvals
                        .retain(|request| request.call.id != call.id);
                }
                ApprovalDecision::Deny => {
                    self.result
                        .pending_approvals
                        .retain(|request| request.call.id != call.id);
                    self.calls.pop_front();
                    let output = ToolOutput {
                        content: vec![Content::Text {
                            text: self
                                .denial_reasons
                                .remove(&call.id)
                                .unwrap_or_else(|| "tool call denied by host approval gate".into()),
                        }],
                        is_error: true,
                        should_pause: false,
                    };
                    self.append(RunItem::ToolResult {
                        call_id: call.id.clone(),
                        output: output.clone(),
                    });
                    self.emit(RunEvent::ToolFinished {
                        call_id: call.id,
                        output,
                    })
                    .await?;
                    return Ok(false);
                }
                ApprovalDecision::Defer => {
                    self.result.pending_approvals = vec![request];
                    self.checkpoint(Boundary::ApprovalPending, Some(&call))
                        .await?;
                    return Ok(true);
                }
            }
        }
        self.checkpoint(Boundary::ToolPrepared, Some(&call)).await?;
        if let Some(handoff) = handoff {
            let pending: HashSet<_> = self.calls.drain(..).map(|call| call.id).collect();
            let ordered: Vec<_> = self
                .result
                .responses
                .last()
                .unwrap()
                .items
                .iter()
                .filter_map(|item| match item {
                    RunItem::ToolCall { call } if pending.contains(&call.id) => Some(call.clone()),
                    _ => None,
                })
                .collect();
            for pending in ordered {
                if pending.id == call.id {
                    self.append(RunItem::Handoff {
                        call_id: pending.id,
                        agent: handoff.target.name.clone(),
                    });
                } else {
                    self.append(RunItem::ToolResult { call_id: pending.id, output: ToolOutput {
                        content: vec![Content::Text { text: format!("not executed: the conversation was handed off to {} in this turn", handoff.target.name) }],
                        is_error: true,
                        should_pause: false,
                    } });
                }
            }
            self.publish_committed().await?;
            let from = self.agent.name.clone();
            self.agent = handoff.target.clone();
            self.result.last_agent = Some(self.agent.name.clone());
            self.observe(Observation::Handoff {
                from,
                to: self.agent.name.clone(),
            })
            .await?;
            self.checkpoint(Boundary::Handoff, None).await?;
            self.phase = Phase::Model;
            return Ok(false);
        }
        let tool = tool.expect("tool or handoff resolved");
        let raw = self.execute_tool(tool.as_ref(), &call).await?;
        self.finish_tool(call, raw).await?;
        Ok(false)
    }
    async fn execute_tool(&self, tool: &dyn Tool, call: &ToolCall) -> Result<ExecutedTool, Error> {
        self.observe(Observation::ToolStarted {
            agent: self.agent.name.clone(),
            call: call.clone(),
        })
        .await?;
        self.emit(RunEvent::ToolStarted { call: call.clone() })
            .await?;
        let mut operation = self.context.clone();
        let timeout = self.policy.tools.timeout.or_else(|| tool.timeout());
        if let Some(timeout) = timeout {
            if let Some(deadline) = Instant::now().checked_add(timeout) {
                operation.deadline = Some(operation.deadline.map_or(deadline, |d| d.min(deadline)));
            }
        }
        let context = ToolContext {
            operation,
            work_dir: self.config.work_dir.clone(),
            policy: self.policy.tools.clone(),
            idempotency_key: Some(format!(
                "{}:{}:{}",
                self.context.run_id, self.turns, call.id
            )),
        };
        let raw = match bounded(
            &context.operation,
            timeout,
            tool.execute(&context, call.clone()),
        )
        .await
        {
            Ok(output) => output,
            Err(error) => {
                // A local tool budget is model-visible; a dead parent still terminates the run.
                self.context.check_active()?;
                let message = if let Some(timeout) = timeout.filter(|_| {
                    context
                        .operation
                        .deadline
                        .is_some_and(|d| Instant::now() >= d)
                }) {
                    format!(
                        "tool {:?} timed out after {}",
                        call.name,
                        go_duration(timeout)
                    )
                } else {
                    error.info.message
                };
                ToolOutput {
                    content: vec![Content::Text { text: message }],
                    is_error: true,
                    should_pause: false,
                }
            }
        };
        let hook_error = self
            .observe(Observation::RawToolOutput {
                call: call.clone(),
                output: raw.clone(),
            })
            .await
            .err();
        Ok(ExecutedTool { raw, hook_error })
    }
    async fn finish_tool(&mut self, call: ToolCall, executed: ExecutedTool) -> Result<(), Error> {
        self.calls.pop_front();
        self.approvals.remove(&call.id);
        let raw = executed.raw;
        self.tool_pause |= raw.should_pause;
        if let Some(error) = executed.hook_error {
            self.append(RunItem::ToolResult {
                call_id: call.id,
                output: withheld_output(),
            });
            return Err(error);
        }
        let processed = match self
            .config
            .output
            .process(raw, self.policy.tools.access != AccessMode::ReadOnly)
        {
            Ok(processed) => processed,
            Err(error) => {
                self.append(RunItem::ToolResult {
                    call_id: call.id,
                    output: withheld_output(),
                });
                return Err(error);
            }
        };
        if let Some(spill) = processed.spill {
            self.spills.push(Arc::new(spill));
        }
        if self.tool_final.is_none() {
            self.tool_final = Some(text(&processed.item_output.content));
        }
        self.result.history.push(RunItem::ToolResult {
            call_id: call.id.clone(),
            output: processed.output,
        });
        self.result.new_items.push(RunItem::ToolResult {
            call_id: call.id.clone(),
            output: processed.item_output.clone(),
        });
        self.checkpoint(Boundary::ToolCompleted, Some(&call))
            .await?;
        self.emit(RunEvent::ToolFinished {
            call_id: call.id,
            output: processed.item_output,
        })
        .await?;
        Ok(())
    }
}

fn withheld_output() -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text {
            text: "Tool completed; output withheld because post-processing failed.".into(),
        }],
        is_error: true,
        should_pause: false,
    }
}

fn validate_history_pairs(history: &[RunItem]) -> Result<(), Error> {
    let mut pending = HashSet::new();
    for item in history {
        match item {
            RunItem::ToolCall { call } => {
                if !pending.insert(call.id.clone()) {
                    return Err(Error::new(
                        ErrorCategory::InvalidInput,
                        "duplicate compacted call",
                    ));
                }
            }
            RunItem::ToolResult { call_id, .. } | RunItem::Handoff { call_id, .. } => {
                if !pending.remove(call_id) {
                    return Err(Error::new(
                        ErrorCategory::InvalidInput,
                        "orphan compacted tool result",
                    ));
                }
            }
            _ => {}
        }
    }
    if !pending.is_empty() {
        return Err(Error::new(
            ErrorCategory::InvalidInput,
            "compaction left unresolved tool calls",
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct FailedSpills {
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    _spills: Vec<Arc<SpillFile>>,
}
impl std::fmt::Display for FailedSpills {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("partial result spill ownership")
    }
}
impl std::error::Error for FailedSpills {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|s| s as &(dyn std::error::Error + 'static))
    }
}

/// Minimal history owner. Every success, pause, or failure replaces history with
/// the engine snapshot; it never reconstructs history as input + new_items.
#[derive(Default)]
pub struct Conversation {
    pub history: Vec<RunItem>,
    spills: Vec<Arc<SpillFile>>,
    paused: bool,
}
impl Conversation {
    pub async fn run(
        &mut self,
        runner: &Runner,
        context: Context,
        input: Vec<RunItem>,
        policy: RunPolicy,
        host: Arc<dyn Host>,
    ) -> Result<RunOutcome, RunError> {
        if self.paused {
            return Err(Error::new(ErrorCategory::InvalidInput, "resume the continuation and accept its outcome before running the conversation again").into());
        }
        self.history.extend(input);
        let result = runner
            .run(
                context,
                RunRequest {
                    input: self.history.clone(),
                    policy,
                },
                host,
            )
            .await;
        match result {
            Ok(outcome) => {
                self.history = outcome.result.history.clone();
                self.paused = outcome.result.status == RunStatus::Paused;
                self.spills.extend(outcome.spills.iter().cloned());
                Ok(outcome)
            }
            Err(error) => {
                self.accept_error(&error);
                Err(error)
            }
        }
    }
    /// Adopt a failed resume as well as its spill ownership; subsequent runs
    /// still reject history with unresolved calls rather than replaying effects.
    pub fn accept_error(&mut self, error: &RunError) {
        if let Some(partial) = &error.partial {
            self.history = partial.history.clone();
        }
        if let Some(spills) = error
            .error
            .source
            .as_ref()
            .and_then(|s| s.downcast_ref::<FailedSpills>())
        {
            self.spills.extend(spills._spills.iter().cloned());
        }
        self.paused = false;
    }
    /// Use after resuming an owned continuation to update conversation history.
    pub fn accept(&mut self, outcome: &mut RunOutcome) {
        self.history = outcome.result.history.clone();
        self.paused = outcome.result.status == RunStatus::Paused;
        self.spills.extend(outcome.spills.iter().cloned());
    }
}
