//! Runtime bridge for one lifecycle-owned, prepared tool list.
use adk_core::{Context, Error, ErrorCategory, Host, RunError, RunRequest};
use adk_runtime::{AgentConfig, RunOutcome, RunStream, Runner, RunnerConfig};
use adk_tools::bundle::{BundleError, ToolBundle};
use std::sync::Arc;

/// Owns tools for runs, streams and their continuations. Close before runtime shutdown.
/// Existing agent tools are replaced, not appended. Handoffs require independently
/// owned agent lifecycles and are deliberately rejected by this single-bundle bridge.
pub struct ToolRunner {
    runner: Runner,
    bundle: ToolBundle,
}
impl ToolRunner {
    pub fn new(
        mut agent: AgentConfig,
        config: RunnerConfig,
        bundle: ToolBundle,
    ) -> Result<Self, Error> {
        if !agent.handoffs.is_empty() {
            return Err(Error::new(
                ErrorCategory::InvalidInput,
                "tool bundle runner does not accept handoffs",
            ));
        }
        agent.tools = bundle.prepared().tools;
        Ok(Self {
            runner: Runner::new(agent, config)?,
            bundle,
        })
    }

    /// Run limits and tool-use behavior come from the request; tool authorization
    /// comes exclusively from the prepared bundle, including on continuation resume.
    pub async fn run(
        &self,
        context: Context,
        mut request: RunRequest,
        host: Arc<dyn Host>,
    ) -> Result<RunOutcome, RunError> {
        request.policy.tools = self.bundle.prepared().policy;
        self.runner
            .run(self.bundle.context(&context), request, host)
            .await
    }

    pub fn stream(
        &self,
        context: Context,
        mut request: RunRequest,
        host: Arc<dyn Host>,
    ) -> RunStream {
        request.policy.tools = self.bundle.prepared().policy;
        self.runner
            .stream(self.bundle.context(&context), request, host)
    }

    pub async fn close(&mut self) -> Result<(), BundleError> {
        self.bundle.close().await
    }
}
