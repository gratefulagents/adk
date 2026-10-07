use adk_core::{BoxFuture, Error, Host};
use adk_runtime::{
    Runner, RunnerChildExecutor, RunnerConfig,
    subagent::{ChildControl, ChildExecutor, ChildInvocation, ChildOutcome},
};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

#[derive(Default)]
pub(super) struct PendingExecutor(OnceLock<RunnerChildExecutor>);

impl PendingExecutor {
    pub(super) fn initialize(&self, runners: HashMap<String, Runner>, host: Arc<dyn Host>) {
        assert!(self.0.set(RunnerChildExecutor::new(runners, host)).is_ok());
    }
}

impl ChildExecutor for PendingExecutor {
    fn execute<'a>(
        &'a self,
        invocation: ChildInvocation,
        control: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        self.0
            .get()
            .expect("child runners initialized before exposing scheduler")
            .execute(invocation, control)
    }
}

// Children own their input admission; retaining the parent session would also create an Arc cycle.
pub(super) fn runner_config(parent: &RunnerConfig) -> RunnerConfig {
    RunnerConfig {
        work_dir: parent.work_dir.clone(),
        output: parent.output.clone(),
        retry: parent.retry.clone(),
        error_handler: parent.error_handler.clone(),
        generation_observer: parent.generation_observer.clone(),
        limits: parent.limits.clone(),
        cost_estimator: parent.cost_estimator.clone(),
        model_idle_timeout: parent.model_idle_timeout,
        validate_tool_arguments: parent.validate_tool_arguments,
        tool_input_guardrails: parent.tool_input_guardrails.clone(),
        tool_output_guardrails: parent.tool_output_guardrails.clone(),
        approve_mutating_tools: parent.approve_mutating_tools,
        consecutive_tool_error_limit: parent.consecutive_tool_error_limit,
        subagent_max_turns: parent.subagent_max_turns,
        hooks: parent.hooks.clone(),
        compaction: parent.compaction.clone(),
        local_compaction: parent.local_compaction,
        compaction_model_defaults: parent.compaction_model_defaults,
        compaction_model_resolver: parent.compaction_model_resolver.clone(),
        ..Default::default()
    }
}
