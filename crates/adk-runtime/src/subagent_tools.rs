//! Model-facing adapters over an explicitly owned child session.

use crate::runner::Runner;
use crate::subagent::*;
use adk_core::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

/// Shared conversation state, not the task owner. Keep `Scheduler` alive until
/// the host explicitly closes the session with `Scheduler::shutdown`.
pub struct SubagentSession {
    pub scheduler: SchedulerHandle,
    parent: Mutex<Vec<RunItem>>,
    staged_delivery: Mutex<HashSet<String>>,
    parent_task: Option<String>,
}

impl SubagentSession {
    pub fn new(scheduler: SchedulerHandle) -> Self {
        Self {
            scheduler,
            parent: Mutex::new(Vec::new()),
            staged_delivery: Mutex::new(HashSet::new()),
            parent_task: None,
        }
    }

    pub(crate) fn for_child(control: &ChildControl) -> Self {
        let mut session = Self::new(control.delegation_handle());
        session.parent_task = Some(control.task_id().into());
        session
    }

    pub(crate) fn update_parent(&self, history: &[RunItem]) {
        let completed: HashSet<_> = history
            .iter()
            .filter_map(|item| match item {
                RunItem::ToolResult { call_id, .. } | RunItem::Handoff { call_id, .. } => {
                    Some(call_id.as_str())
                }
                _ => None,
            })
            .collect();
        *self.parent.lock().unwrap() = history
            .iter()
            .filter(|item| match item {
                RunItem::ToolCall { call } => completed.contains(call.id.as_str()),
                _ => true,
            })
            .cloned()
            .collect();
    }
}

#[derive(Default, Deserialize, JsonSchema)]
struct SpawnInput {
    #[serde(default)]
    message: String,
    #[serde(default)]
    agent_name: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    tool_access: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    dependency_policy: String,
    include_dependency_results: Option<bool>,
    #[serde(default)]
    share_parent_context: bool,
    #[serde(default)]
    timeout_ms: u64,
    #[serde(default)]
    tasks: Vec<BatchInput>,
}

#[derive(Deserialize, JsonSchema)]
struct BatchInput {
    key: String,
    message: String,
    #[serde(default)]
    agent_name: String,
    #[serde(default)]
    tool_access: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    dependency_policy: String,
    include_dependency_results: Option<bool>,
    share_parent_context: Option<bool>,
}

#[derive(Default, Deserialize, JsonSchema)]
struct StatusInput {
    #[serde(default)]
    task_ids: Vec<String>,
    #[serde(default)]
    detail: String,
}

#[derive(Default, Deserialize, JsonSchema)]
struct WaitInput {
    #[serde(default)]
    task_ids: Vec<String>,
    #[serde(default)]
    wait_for: String,
    #[serde(default)]
    timeout_ms: u64,
}

#[derive(Deserialize, JsonSchema)]
struct ControlInput {
    action: String,
    task_id: String,
    #[serde(default)]
    message: String,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::InvalidInput, message)
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|error| invalid(error.to_string()))
}

fn output(value: impl Serialize, is_error: bool) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text {
            text: serde_json::to_string(&value).expect("serializable tool response"),
        }],
        is_error,
        should_pause: false,
    }
}

fn narrowed(mut policy: ToolPolicy, access: &str) -> Result<ToolPolicy, Error> {
    match access {
        "" | "full" => {}
        "read-only" => {
            policy.access = AccessMode::ReadOnly;
            policy.allowed_mutating_tools.clear();
        }
        _ => return Err(invalid("tool_access must be full or read-only")),
    }
    Ok(policy)
}

fn timeout(milliseconds: u64) -> Option<Duration> {
    (milliseconds != 0).then(|| Duration::from_millis(milliseconds))
}

#[derive(Clone, Copy)]
enum ToolKind {
    Spawn,
    Status,
    Wait,
    Control,
}

struct SubagentTool {
    definition: ToolDefinition,
    kind: ToolKind,
    session: Arc<SubagentSession>,
    default_agent: String,
}

/// All adapters share the same scheduler and delivery ledger.
pub fn build_subagent_task_tools(
    session: Arc<SubagentSession>,
    default_agent: impl Into<String>,
) -> Vec<Arc<dyn Tool>> {
    let default_agent = default_agent.into();
    [
        (ToolKind::Spawn, "subagent", "Delegate a task or keyed DAG; sync waits, background returns task IDs. Results are delivered automatically.", schemars::schema_for!(SpawnInput)),
        (ToolKind::Status, "subagent_status", "Inspect child summary, activity, results, or dependency graph without consuming results.", schemars::schema_for!(StatusInput)),
        (ToolKind::Wait, "subagent_wait", "Wait for any or all children; timeout leaves children running.", schemars::schema_for!(WaitInput)),
        (ToolKind::Control, "subagent_control", "Steer a child with action=message, or cancel it with action=cancel.", schemars::schema_for!(ControlInput)),
    ].into_iter().map(|(kind, name, description, input_schema)| Arc::new(SubagentTool {
        definition: ToolDefinition { name: name.into(), description: description.into(), input_schema, read_only: true, requires_approval: false },
        kind, session: session.clone(), default_agent: default_agent.clone(),
    }) as Arc<dyn Tool>).collect()
}

impl Tool for SubagentTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn is_control_flow(&self) -> bool {
        true
    }
    fn is_delegation(&self) -> bool {
        true
    }
    fn for_delegation(&self, scope: &dyn std::any::Any) -> Option<Arc<dyn Tool>> {
        let session = scope.downcast_ref::<Arc<SubagentSession>>()?.clone();
        Some(Arc::new(Self {
            definition: self.definition.clone(),
            kind: self.kind,
            session,
            default_agent: self.default_agent.clone(),
        }))
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let owned_context = ToolContext {
                operation: context.operation.clone(),
                work_dir: context.work_dir.clone(),
                policy: context.policy.clone(),
                idempotency_key: Some(
                    context
                        .idempotency_key
                        .clone()
                        .unwrap_or_else(|| format!("{}:{}", context.operation.run_id, call.id)),
                ),
            };
            let context = &owned_context;
            let result = match self.kind {
                ToolKind::Spawn => match parse::<SpawnInput>(call.arguments) {
                    Ok(input) => {
                        self.session
                            .spawn(context, input, &self.default_agent)
                            .await
                    }
                    Err(error) => Err(error),
                },
                ToolKind::Status => match parse::<StatusInput>(call.arguments) {
                    Ok(input) => self.session.status(input).await,
                    Err(error) => Err(error),
                },
                ToolKind::Wait => match parse::<WaitInput>(call.arguments) {
                    Ok(input) => self.session.wait(context, input).await,
                    Err(error) => Err(error),
                },
                ToolKind::Control => match parse::<ControlInput>(call.arguments) {
                    Ok(input) => self.session.control(context, input).await,
                    Err(error) => Err(error),
                },
            };
            match result {
                Err(error) if error.info.category == ErrorCategory::InvalidInput => {
                    Ok(output(json!({"error": error.info.message}), true))
                }
                result => result,
            }
        })
    }
}

/// A synchronous specialist tool using the same owned child engine as `subagent`.
pub struct AgentAsTool {
    definition: ToolDefinition,
    agent: String,
    session: Arc<SubagentSession>,
}

#[derive(Deserialize, JsonSchema)]
struct AgentInput {
    message: String,
}

impl AgentAsTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        agent: impl Into<String>,
        session: Arc<SubagentSession>,
    ) -> Self {
        Self {
            definition: ToolDefinition {
                name: name.into(),
                description: description.into(),
                input_schema: schemars::schema_for!(AgentInput),
                read_only: true,
                requires_approval: false,
            },
            agent: agent.into(),
            session,
        }
    }
}
impl Tool for AgentAsTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn is_control_flow(&self) -> bool {
        true
    }
    fn is_delegation(&self) -> bool {
        true
    }
    fn for_delegation(&self, scope: &dyn std::any::Any) -> Option<Arc<dyn Tool>> {
        let session = scope.downcast_ref::<Arc<SubagentSession>>()?.clone();
        Some(Arc::new(Self {
            definition: self.definition.clone(),
            agent: self.agent.clone(),
            session,
        }))
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let owned_context = ToolContext {
                operation: context.operation.clone(),
                work_dir: context.work_dir.clone(),
                policy: context.policy.clone(),
                idempotency_key: Some(
                    context
                        .idempotency_key
                        .clone()
                        .unwrap_or_else(|| format!("{}:{}", context.operation.run_id, call.id)),
                ),
            };
            let context = &owned_context;
            let input: AgentInput = match parse(call.arguments) {
                Ok(input) => input,
                Err(error) => return Ok(output(json!({"error":error.info.message}), true)),
            };
            self.session
                .spawn(
                    context,
                    SpawnInput {
                        message: input.message,
                        ..Default::default()
                    },
                    &self.agent,
                )
                .await
        })
    }
}

impl SubagentSession {
    pub(crate) fn begin_run(&self) {
        self.staged_delivery.lock().unwrap().clear();
    }
    pub(crate) fn delivery_ids(&self) -> Vec<String> {
        self.staged_delivery
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect()
    }
    pub(crate) async fn commit_delivery(&self, ids: &[String]) -> Result<(), Error> {
        if ids.is_empty() {
            return Ok(());
        }
        self.scheduler.collect_ids(ids).await?;
        self.staged_delivery
            .lock()
            .unwrap()
            .retain(|id| !ids.contains(id));
        Ok(())
    }
    fn stage_results(&self, ids: &[String]) -> Vec<TaskSnapshot> {
        let snapshot = self.scheduler.snapshot();
        let visible: HashSet<_> = self
            .scheduler
            .list()
            .into_iter()
            .map(|task| task.id)
            .collect();
        let mut staged = self.staged_delivery.lock().unwrap();
        snapshot
            .records
            .into_iter()
            .filter_map(|record| {
                (visible.contains(&record.task.id)
                    && ids.contains(&record.task.id)
                    && record.task.status.is_terminal()
                    && !record.result_delivered
                    && staged.insert(record.task.id.clone()))
                .then_some(record.task)
            })
            .collect()
    }
    fn pending_ids(&self) -> Vec<String> {
        let visible: HashSet<_> = self
            .scheduler
            .list()
            .into_iter()
            .map(|task| task.id)
            .collect();
        self.scheduler
            .snapshot()
            .records
            .into_iter()
            .filter(|r| {
                r.parent_id == self.parent_task
                    && visible.contains(&r.task.id)
                    && !r.result_delivered
                    && !self.staged_delivery.lock().unwrap().contains(&r.task.id)
            })
            .map(|r| r.task.id)
            .collect()
    }
    pub(crate) async fn collect(&self, context: &Context) -> Result<Vec<RunItem>, Error> {
        context.check_active()?;
        let tasks = self.stage_results(&self.pending_ids());
        Ok(tasks
            .into_iter()
            .map(|task| RunItem::Message {
                message: Message {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: format!(
                            "[SYSTEM] Sub-agent result: {}",
                            serde_json::to_string(&task).unwrap()
                        ),
                    }],
                },
            })
            .collect())
    }
    pub(crate) async fn join(&self, context: &Context) -> Result<Vec<RunItem>, Error> {
        let mut ids = self.pending_ids();
        // A direct child may intentionally end a no-output turn while its own
        // background descendants still run. Final answers must join those too,
        // without stealing their results from their immediate parent.
        for task in self.scheduler.list() {
            if !task.status.is_terminal() && !ids.contains(&task.id) {
                ids.push(task.id);
            }
        }
        if ids.is_empty() {
            return Ok(vec![]);
        }
        if self
            .scheduler
            .list()
            .iter()
            .any(|t| ids.contains(&t.id) && t.status == TaskStatus::Reconciling)
        {
            return Err(Error::new(
                ErrorCategory::Unsupported,
                "child reconciliation required before final answer",
            ));
        }
        crate::runner::bounded(
            context,
            None,
            self.scheduler.wait(&ids, WaitMode::All, None),
        )
        .await?;
        self.collect(context).await
    }
    async fn spawn(
        &self,
        context: &ToolContext,
        input: SpawnInput,
        default_agent: &str,
    ) -> Result<ToolOutput, Error> {
        let background = match input.mode.as_str() {
            "" | "sync" => false,
            "background" => true,
            _ => return Err(invalid("mode must be sync or background")),
        };
        let single = !input.message.trim().is_empty();
        if single != input.tasks.is_empty() {
            return Err(invalid("provide exactly one of message or tasks"));
        }
        let policy = narrowed(context.policy.clone(), &input.tool_access)?;
        let agent = if input.agent_name.is_empty() {
            default_agent
        } else {
            &input.agent_name
        };
        let mut submissions = Vec::new();
        let tasks = if single {
            vec![BatchInput {
                key: String::new(),
                message: input.message,
                agent_name: agent.into(),
                tool_access: String::new(),
                depends_on: input.depends_on,
                dependency_policy: input.dependency_policy,
                include_dependency_results: input.include_dependency_results,
                share_parent_context: None,
            }]
        } else {
            input.tasks
        };
        let mut keys = HashMap::new();
        if !single {
            let prefix = context
                .idempotency_key
                .as_deref()
                .unwrap_or(&context.operation.run_id);
            for task in &tasks {
                if keys
                    .insert(task.key.clone(), format!("{prefix}:{}", task.key))
                    .is_some()
                {
                    return Err(invalid("duplicate batch task key"));
                }
            }
        }
        for task in tasks {
            if !single && task.key.trim().is_empty() {
                return Err(invalid("batch task key is required"));
            }
            let mut submission = Submission::new(
                if task.agent_name.is_empty() {
                    agent
                } else {
                    &task.agent_name
                },
                task.message,
            );
            submission.id = keys.get(&task.key).cloned().unwrap_or_default();
            submission.depends_on = task
                .depends_on
                .into_iter()
                .map(|id| keys.get(&id).cloned().unwrap_or(id))
                .collect();
            submission.dependency_policy = match task.dependency_policy.as_str() {
                "" | "all_success" => DependencyPolicy::AllSuccess,
                "all_terminal" => DependencyPolicy::AllTerminal,
                _ => {
                    return Err(invalid(
                        "dependency_policy must be all_success or all_terminal",
                    ));
                }
            };
            submission.include_dependency_results = task.include_dependency_results.unwrap_or(true);
            submission.parent_history = task
                .share_parent_context
                .unwrap_or(input.share_parent_context)
                .then(|| self.parent.lock().unwrap().clone());
            submission.policy.tools = narrowed(policy.clone(), &task.tool_access)?;
            submissions.push(submission);
        }
        let ids = self.scheduler.submit_dag(submissions).await?;
        if background {
            return Ok(output(
                json!({"task_id": single.then(|| ids[0].clone()), "task_ids": ids, "task_ids_by_key": keys, "status":"pending", "agent": agent}),
                false,
            ));
        }
        self.wait(
            context,
            WaitInput {
                task_ids: ids,
                timeout_ms: input.timeout_ms,
                ..Default::default()
            },
        )
        .await
    }
    async fn status(&self, input: StatusInput) -> Result<ToolOutput, Error> {
        let detail = match input.detail.as_str() {
            "" | "summary" | "graph" => Detail::Summary,
            "activity" => Detail::Activity,
            "results" => Detail::Result,
            _ => {
                return Err(invalid(
                    "detail must be summary, activity, results, or graph",
                ));
            }
        };
        let ids = if input.task_ids.is_empty() {
            self.scheduler.list().into_iter().map(|t| t.id).collect()
        } else {
            input.task_ids
        };
        let tasks = ids
            .iter()
            .map(|id| self.scheduler.status(id, detail))
            .collect::<Result<Vec<_>, _>>()?;
        if input.detail == "graph" {
            let edges: Vec<_> = tasks
                .iter()
                .flat_map(|task| {
                    task.depends_on
                        .iter()
                        .map(|dep| json!({"from":dep,"to":task.id}))
                })
                .collect();
            Ok(output(json!({"nodes":tasks,"edges":edges}), false))
        } else {
            Ok(output(json!({"tasks":tasks}), false))
        }
    }
    async fn wait(&self, context: &ToolContext, input: WaitInput) -> Result<ToolOutput, Error> {
        let mode = match input.wait_for.as_str() {
            "" | "all" => WaitMode::All,
            "any" => WaitMode::Any,
            _ => return Err(invalid("wait_for must be all or any")),
        };
        let ids = if input.task_ids.is_empty() {
            self.pending_ids()
        } else {
            input.task_ids
        };
        if ids.is_empty() {
            return Ok(output(json!({"wait_complete":true,"finished":[]}), false));
        }
        let result = crate::runner::bounded(
            &context.operation,
            None,
            self.scheduler.wait(&ids, mode, timeout(input.timeout_ms)),
        )
        .await;
        let timed_out = match result {
            Ok(_) => false,
            Err(error) if error.info.category == ErrorCategory::DeadlineExceeded => true,
            Err(error) => return Err(error),
        };
        let finished = self.stage_results(&ids);
        let tasks = ids
            .iter()
            .map(|id| self.scheduler.status(id, Detail::Full))
            .collect::<Result<Vec<_>, _>>()?;
        let mut previous = Vec::new();
        let mut active = Vec::new();
        for mut task in tasks {
            if !task.status.is_terminal() {
                active.push(task);
            } else if !finished.iter().any(|t| t.id == task.id) {
                task.result.clear();
                previous.push(task);
            }
        }
        let failed = finished
            .iter()
            .any(|task| matches!(task.status, TaskStatus::Failed | TaskStatus::Cancelled));
        Ok(output(
            json!({"wait_complete":!timed_out,"timed_out":timed_out,"finished":finished,"still_active":active,"previously_delivered":previous}),
            failed,
        ))
    }
    async fn control(
        &self,
        context: &ToolContext,
        input: ControlInput,
    ) -> Result<ToolOutput, Error> {
        match input.action.as_str() {
            "message" => {
                let key = context
                    .idempotency_key
                    .clone()
                    .unwrap_or_else(|| format!("{}:{}", context.operation.run_id, input.message));
                self.scheduler
                    .steer(&input.task_id, &key, &input.message)
                    .await?;
                Ok(output(
                    json!({"task_id":input.task_id,"status":"message_queued"}),
                    false,
                ))
            }
            "cancel" => {
                self.scheduler.cancel(&input.task_id).await?;
                Ok(output(
                    json!({"task_id":input.task_id,"status":"cancellation_requested"}),
                    false,
                ))
            }
            _ => Err(invalid("action must be message or cancel")),
        }
    }
}

/// Registered runners share one child execution implementation across both tool surfaces.
pub struct RunnerChildExecutor {
    runners: HashMap<String, Runner>,
    host: Arc<dyn Host>,
}
impl RunnerChildExecutor {
    pub fn new(runners: HashMap<String, Runner>, host: Arc<dyn Host>) -> Self {
        Self { runners, host }
    }
}
impl ChildExecutor for RunnerChildExecutor {
    fn execute<'a>(
        &'a self,
        invocation: ChildInvocation,
        control: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        Box::pin(async move {
            let runner = self
                .runners
                .get(&invocation.agent_name)
                .ok_or_else(|| invalid("unknown child runner"))?;
            runner
                .run_child(invocation, control, self.host.clone())
                .await
        })
    }
}
