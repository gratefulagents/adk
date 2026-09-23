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
    parent: Mutex<(Vec<RunItem>, Vec<ItemProvenance>)>,
    staged_delivery: Mutex<HashSet<String>>,
    parent_task: Option<String>,
}

impl SubagentSession {
    pub fn new(scheduler: SchedulerHandle) -> Self {
        Self {
            scheduler,
            parent: Mutex::new((Vec::new(), Vec::new())),
            staged_delivery: Mutex::new(HashSet::new()),
            parent_task: None,
        }
    }

    pub(crate) fn for_child(control: &ChildControl) -> Self {
        let mut session = Self::new(control.delegation_handle());
        session.parent_task = Some(control.task_id().into());
        session
    }

    pub(crate) fn update_parent(&self, history: &[RunItem], provenance: &[ItemProvenance]) {
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
            .zip(provenance)
            .filter(|(item, _)| match item {
                RunItem::ToolCall { call } => completed.contains(call.id.as_str()),
                _ => true,
            })
            .map(|(item, source)| (item.clone(), source.clone()))
            .unzip();
    }
}

#[derive(Default, Deserialize)]
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

#[derive(Deserialize)]
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

#[derive(Default, Deserialize)]
struct StatusInput {
    #[serde(default)]
    task_ids: Vec<String>,
    #[serde(default)]
    detail: String,
}

#[derive(Default, Deserialize)]
struct WaitInput {
    #[serde(default)]
    task_ids: Vec<String>,
    #[serde(default)]
    wait_for: String,
    #[serde(default)]
    timeout_ms: u64,
}

#[derive(Deserialize)]
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

fn go_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        return "0s".into();
    }
    let (unit, scale, precision) = if nanos < 1_000 {
        ("ns", 1, 0)
    } else if nanos < 1_000_000 {
        ("µs", 1_000, 3)
    } else if nanos < 1_000_000_000 {
        ("ms", 1_000_000, 6)
    } else {
        ("s", 1_000_000_000, 9)
    };
    let fraction = nanos % scale;
    let fraction = if fraction == 0 {
        String::new()
    } else {
        format!(".{fraction:0precision$}")
            .trim_end_matches('0')
            .to_owned()
    };
    if unit == "s" && duration.as_secs() >= 60 {
        let seconds = duration.as_secs();
        let hours = if seconds >= 3600 {
            format!("{}h", seconds / 3600)
        } else {
            String::new()
        };
        format!("{hours}{}m{}{fraction}s", seconds / 60 % 60, seconds % 60)
    } else {
        format!("{}{fraction}{unit}", nanos / scale)
    }
}

fn joined_task(task: &TaskSnapshot) -> Value {
    let mut value = json!({"task_id":task.id,"agent":task.agent_name,"status":task.status});
    if let Some(duration) = task.duration.or_else(|| {
        if task.status.is_terminal() {
            return None;
        }
        task.elapsed().map(|elapsed| {
            Duration::from_millis(
                ((elapsed.as_nanos() + 500_000) / 1_000_000).min(u64::MAX as u128) as u64,
            )
        })
    }) {
        value["duration"] = json!(go_duration(duration));
    }
    if !task.result.is_empty() {
        value["result"] = json!(task.result);
    }
    if let Some(error) = task.error.as_ref().filter(|error| !error.is_empty()) {
        value["error"] = json!(error);
    }
    value
}

fn progress_task(task: &TaskSnapshot) -> Value {
    let mut value = joined_task(task);
    value.as_object_mut().unwrap().remove("result");
    if !task.depends_on.is_empty() {
        value["depends_on"] = json!(task.depends_on);
    }
    if !task.waiting_on.is_empty() {
        value["waiting_on"] = json!(task.waiting_on);
    }
    if task.messages_received != 0 {
        value["messages_received"] = json!(task.messages_received);
    }
    if !task.last_parent_message.is_empty() {
        value["last_parent_message"] = json!(task.last_parent_message);
    }
    if task.status.is_terminal() && !task.result.is_empty() {
        value["result_available"] = json!(true);
    }
    if let Some(activity) = &task.activity {
        if !activity.current_step.is_empty() {
            value["current_step"] = json!(activity.current_step);
        }
        if !activity.last_tool.is_empty() {
            value["last_tool"] = json!(activity.last_tool);
        }
        if !activity.files_written.is_empty() {
            value["files_written"] = json!(activity.files_written.len());
        }
    }
    value
}

fn unique_task_ids(ids: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty() && seen.insert(*id))
        .map(str::to_owned)
        .collect()
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
    let schemas: HashMap<String, schemars::Schema> =
        serde_json::from_str(include_str!("subagent_schema.json"))
            .expect("pinned subagent schemas");
    [
        (ToolKind::Spawn, "subagent", "Delegate a task or keyed DAG; sync waits, background returns task IDs. Results are delivered automatically.", schemas["subagent"].clone()),
        (ToolKind::Status, "subagent_status", "Inspect child summary, activity, results, or dependency graph; results can be re-read at any time.", schemas["subagent_status"].clone()),
        (ToolKind::Wait, "subagent_wait", "Wait for any or all children; timeout leaves children running.", schemas["subagent_wait"].clone()),
        (ToolKind::Control, "subagent_control", "Steer a child with action=message, or cancel it with action=cancel.", schemas["subagent_control"].clone()),
    ].into_iter().map(|(kind, name, description, input_schema)| Arc::new(SubagentTool {
        definition: ToolDefinition { name: name.into(), description: description.into(), input_schema, read_only: matches!(kind, ToolKind::Status), requires_approval: false },
        kind, session: session.clone(), default_agent: default_agent.clone(),
    }) as Arc<dyn Tool>).collect()
}

impl Tool for SubagentTool {
    fn for_access(&self, access: AccessMode) -> Option<Arc<dyn Tool>> {
        let mut definition = self.definition.clone();
        definition.read_only =
            access == AccessMode::ReadOnly || matches!(self.kind, ToolKind::Status);
        Some(Arc::new(Self {
            definition,
            kind: self.kind,
            session: self.session.clone(),
            default_agent: self.default_agent.clone(),
        }))
    }
    fn preserve_result_on_timeout(&self) -> bool {
        matches!(self.kind, ToolKind::Spawn | ToolKind::Wait)
    }
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
            let mut policy = context.policy.clone();
            if self.definition.read_only && !matches!(self.kind, ToolKind::Status) {
                policy.access = AccessMode::ReadOnly;
                policy.allowed_mutating_tools.clear();
            }
            let owned_context = ToolContext {
                operation: context.operation.clone(),
                work_dir: context.work_dir.clone(),
                policy,
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
                    Ok(ToolOutput {
                        content: vec![Content::Text {
                            text: error.info.message,
                        }],
                        is_error: true,
                        should_pause: false,
                    })
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
                read_only: false,
                requires_approval: false,
            },
            agent: agent.into(),
            session,
        }
    }
}
impl Tool for AgentAsTool {
    fn for_access(&self, access: AccessMode) -> Option<Arc<dyn Tool>> {
        let mut definition = self.definition.clone();
        definition.read_only = access == AccessMode::ReadOnly;
        Some(Arc::new(Self {
            definition,
            agent: self.agent.clone(),
            session: self.session.clone(),
        }))
    }
    fn preserve_result_on_timeout(&self) -> bool {
        true
    }
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
            let mut policy = context.policy.clone();
            if self.definition.read_only {
                policy.access = AccessMode::ReadOnly;
                policy.allowed_mutating_tools.clear();
            }
            let owned_context = ToolContext {
                operation: context.operation.clone(),
                work_dir: context.work_dir.clone(),
                policy,
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
        let background = match input.mode.trim().to_ascii_lowercase().as_str() {
            "" | "sync" => false,
            "background" | "async" => true,
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
        let mut summaries = Vec::new();
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
            if task
                .share_parent_context
                .unwrap_or(input.share_parent_context)
            {
                let (history, provenance) = self.parent.lock().unwrap().clone();
                submission.parent_history = Some(history);
                submission.parent_history_provenance = provenance;
            }
            submission.policy.tools = narrowed(policy.clone(), &task.tool_access)?;
            if !single {
                let mut summary =
                    json!({"key":task.key,"task_id":submission.id,"agent":submission.agent_name});
                if !submission.depends_on.is_empty() {
                    summary["depends_on"] = json!(submission.depends_on);
                }
                summaries.push(summary);
            }
            submissions.push(submission);
        }
        let ids = self.scheduler.submit_dag(submissions).await?;
        if background {
            return Ok(output(
                if single {
                    json!({"task_id":ids[0],"status":"pending","agent":agent})
                } else {
                    json!({"tasks":summaries,"task_ids_by_key":keys})
                },
                false,
            ));
        }
        let (tasks, timed_out) = self
            .await_tasks(context, &ids, WaitMode::All, input.timeout_ms)
            .await?;
        let terminal_ids = tasks
            .iter()
            .filter(|task| task.status.is_terminal())
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        self.stage_results(&terminal_ids);
        let failed = tasks
            .iter()
            .any(|task| matches!(task.status, TaskStatus::Failed | TaskStatus::Cancelled));
        let mut response = if single {
            joined_task(&tasks[0])
        } else {
            json!({"tasks":summaries,"task_ids_by_key":keys,"results":tasks.iter().map(joined_task).collect::<Vec<_>>(),"wait_complete":!timed_out})
        };
        if timed_out {
            response["wait_complete"] = json!(false);
            response["timed_out"] = json!(true);
            response["note"] = json!(if single {
                "wait deadline reached; the managed task is still active and its result will be delivered when ready"
            } else {
                "wait deadline reached; active managed tasks continue in the background and their results will be delivered when ready"
            });
        }
        Ok(output(response, failed))
    }
    async fn status(&self, input: StatusInput) -> Result<ToolOutput, Error> {
        let detail = input.detail.trim().to_ascii_lowercase();
        if !matches!(
            detail.as_str(),
            "" | "summary" | "activity" | "results" | "graph"
        ) {
            return Err(invalid(
                "detail must be summary, activity, results, or graph",
            ));
        }
        let mut ids = unique_task_ids(&input.task_ids);
        if detail == "graph"
            || input.task_ids.is_empty()
            || (ids.is_empty() && matches!(detail.as_str(), "results" | "activity"))
        {
            ids = self.scheduler.list().into_iter().map(|t| t.id).collect();
        }
        let tasks = ids
            .iter()
            .map(|id| self.scheduler.status(id, Detail::Full))
            .collect::<Result<Vec<_>, _>>()?;
        let response = match detail.as_str() {
            "graph" => {
                let nodes: Vec<_> = tasks.iter().map(|task| {
                    let mut node = json!({"id":task.id,"agent":task.agent_name,"status":task.status});
                    if !task.depends_on.is_empty() { node["depends_on"] = json!(task.depends_on); }
                    if !task.waiting_on.is_empty() { node["waiting_on"] = json!(task.waiting_on); }
                    if task.messages_received != 0 { node["messages_received"] = json!(task.messages_received); }
                    node
                }).collect();
                let edges: Vec<_> = tasks.iter().flat_map(|task| task.depends_on.iter().map(|dep| json!({"from":dep,"to":task.id}))).collect();
                json!({"nodes":nodes,"edges":if edges.is_empty() { Value::Null } else { json!(edges) }})
            }
            "results" => {
                let mut results = Vec::new();
                let mut active = Vec::new();
                let mut terminal_ids = Vec::new();
                for task in &tasks {
                    if !task.status.is_terminal() {
                        active.push(task.id.clone());
                        continue;
                    }
                    terminal_ids.push(task.id.clone());
                    let mut entry = joined_task(task);
                    let message = task.message.trim().replace('\n', " ");
                    if !message.is_empty() {
                        entry["task"] = json!(if message.chars().count() > 300 {
                            format!("{}...", message.chars().take(297).collect::<String>())
                        } else { message });
                    }
                    results.push(entry);
                }
                self.stage_results(&terminal_ids);
                let mut response = json!({"results":results});
                if !active.is_empty() {
                    response["still_active"] = json!(active);
                    response["note"] = json!("tasks in still_active have no result yet — use subagent_wait to wait for them");
                }
                response
            }
            "activity" => json!(tasks.iter().map(|task| {
                let activity = task.activity.as_ref().map(|activity| {
                    let mut value = json!({"files_read":activity.files_read,"files_written":activity.files_written});
                    if !activity.current_step.is_empty() { value["current_step"] = json!(activity.current_step); }
                    if !activity.current_tool.is_empty() { value["current_tool"] = json!(activity.current_tool); }
                    if !activity.current_tool_input.is_empty() { value["current_tool_input"] = json!(activity.current_tool_input); }
                    if !activity.recent_activity.is_empty() {
                        value["recent_activity"] = json!(activity.recent_activity.iter().map(|entry| {
                            let mut value = json!({"timestamp":entry.timestamp,"tool":entry.tool,"summary":entry.summary});
                            if entry.is_error { value["is_error"] = json!(true); }
                            if entry.duration_ms != 0 { value["duration_ms"] = json!(entry.duration_ms); }
                            value
                        }).collect::<Vec<_>>());
                    }
                    value
                });
                json!({"task_id":task.id,"agent":task.agent_name,"status":task.status,"duration":go_duration(task.duration.unwrap_or_default()),"activity":activity})
            }).collect::<Vec<_>>()),
            _ => {
                let mut summary = json!({"total":tasks.len(),"active":0,"pending":0,"waiting":0,"running":0,"completed":0,"failed":0,"cancelled":0});
                for task in &tasks {
                    if !task.status.is_terminal() {
                        summary["active"] = json!(summary["active"].as_u64().unwrap() + 1);
                    }
                    let status = serde_json::to_value(task.status).unwrap();
                    if let Some(count) = summary.get_mut(status.as_str().unwrap()) {
                        *count = json!(count.as_u64().unwrap() + 1);
                    }
                }
                json!({"summary":summary,"tasks":tasks.iter().map(progress_task).collect::<Vec<_>>()})
            }
        };
        Ok(output(response, false))
    }
    async fn await_tasks(
        &self,
        context: &ToolContext,
        ids: &[String],
        mode: WaitMode,
        timeout_ms: u64,
    ) -> Result<(Vec<TaskSnapshot>, bool), Error> {
        let deadline_timeout = context.operation.deadline.map(|deadline| {
            tokio::time::Instant::from_std(deadline)
                .saturating_duration_since(tokio::time::Instant::now())
        });
        let wait_timeout = timeout(timeout_ms)
            .into_iter()
            .chain(deadline_timeout)
            .min();
        // The scheduler must reacquire the child's execution slot before returning
        // a local timeout; dropping its wait instead cancels the child subtree.
        let result = tokio::select! {
            biased;
            _ = context.operation.cancellation.cancelled() => {
                return Err(Error::new(ErrorCategory::Cancelled, "operation cancelled"));
            }
            result = self.scheduler.wait(ids, mode, wait_timeout) => result,
        };
        let timed_out = match result {
            Ok(_) => false,
            Err(error) if error.info.category == ErrorCategory::DeadlineExceeded => true,
            Err(error) => return Err(error),
        };
        let tasks = ids
            .iter()
            .map(|id| self.scheduler.status(id, Detail::Full))
            .collect::<Result<Vec<_>, _>>()?;
        let timed_out = timed_out && tasks.iter().any(|task| !task.status.is_terminal());
        Ok((tasks, timed_out))
    }
    async fn wait(&self, context: &ToolContext, input: WaitInput) -> Result<ToolOutput, Error> {
        let (mode, wait_for) = match input.wait_for.trim().to_ascii_lowercase().as_str() {
            "" | "all" => (WaitMode::All, "all"),
            "any" => (WaitMode::Any, "any"),
            _ => return Err(invalid("wait_for must be all or any")),
        };
        let mut ids = unique_task_ids(&input.task_ids);
        if ids.is_empty() {
            ids = self.pending_ids();
        }
        if ids.is_empty() {
            return Ok(output(
                json!({"wait_complete":true,"wait_for":wait_for,"note":"no active sub-agent tasks and no undelivered results"}),
                false,
            ));
        }
        let (tasks, timed_out) = self
            .await_tasks(context, &ids, mode, input.timeout_ms)
            .await?;
        let terminal_ids = tasks
            .iter()
            .filter(|task| task.status.is_terminal())
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        let staged = self.stage_results(&terminal_ids);
        let mut finished = Vec::new();
        let mut previous = Vec::new();
        let mut active = Vec::new();
        for task in &tasks {
            if !task.status.is_terminal() {
                active.push(progress_task(task));
            } else if staged.iter().any(|t| t.id == task.id) {
                finished.push(joined_task(task));
            } else {
                let mut entry = joined_task(task);
                entry.as_object_mut().unwrap().remove("result");
                previous.push(entry);
            }
        }
        let mut response = json!({"wait_complete":!timed_out,"wait_for":wait_for});
        let mut notes = Vec::new();
        if !finished.is_empty() {
            response["finished"] = json!(finished);
        }
        if !previous.is_empty() {
            response["previously_delivered"] = json!(previous);
            notes.push("previously_delivered results were already returned earlier and are omitted here — re-read them any time with subagent_status detail=\"results\"".to_owned());
        }
        if !active.is_empty() {
            response["still_active"] = json!(active);
        }
        if timed_out {
            response["timed_out"] = json!(true);
            notes.push(format!("timed out after {}ms; {} task(s) still active — call subagent_wait again to keep waiting", input.timeout_ms, active.len()));
        }
        if !notes.is_empty() {
            response["note"] = json!(notes.join(". "));
        }
        Ok(output(response, false))
    }
    async fn control(
        &self,
        context: &ToolContext,
        input: ControlInput,
    ) -> Result<ToolOutput, Error> {
        match input.action.trim().to_ascii_lowercase().as_str() {
            "message" => {
                if input.message.trim().is_empty() {
                    return Err(invalid("message is required for action=message"));
                }
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
