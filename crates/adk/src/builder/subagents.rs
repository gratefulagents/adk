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
    pub(super) fn initialize(
        &self,
        runners: HashMap<String, Runner>,
        host: Arc<dyn Host>,
        progress: Option<Arc<adk_runtime::subagent_tools::ChildProgressRegistry>>,
    ) {
        let mut executor = RunnerChildExecutor::new(runners, host);
        if let Some(progress) = progress {
            executor = executor.with_progress(progress);
        }
        assert!(self.0.set(executor).is_ok());
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
        handoff_history: parent.handoff_history,
        compaction_model_defaults: parent.compaction_model_defaults,
        compaction_model_resolver: parent.compaction_model_resolver.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_runtime::compaction::HandoffHistoryPolicy;

    #[test]
    fn children_preserve_resolved_handoff_history_policy() {
        for policy in [
            HandoffHistoryPolicy::default(),
            HandoffHistoryPolicy {
                enabled: true,
                ..Default::default()
            },
            HandoffHistoryPolicy {
                enabled: true,
                max_tokens: 300,
                target_tokens: 120,
                preserve_recent_items: 3,
                summary_bullet_limit: 2,
            },
            HandoffHistoryPolicy {
                enabled: false,
                max_tokens: 0,
                target_tokens: 0,
                preserve_recent_items: 0,
                summary_bullet_limit: 0,
            },
        ] {
            let parent = RunnerConfig {
                handoff_history: policy,
                ..Default::default()
            };
            let child = runner_config(&parent);
            assert_eq!(child.handoff_history, policy);
            assert_eq!(parent.handoff_history, policy);
        }
    }
}
