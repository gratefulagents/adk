//! Trusted construction and ownership for a frozen tool selection.
//!
//! Keep the bundle alive while using its prepared handles. Drop cancels work;
//! call `close().await` before shutting down the executor to await process cleanup.
use crate::{BuildError, Config, PreparedTools, Registry, select, shell};
use adk_core::{
    AccessMode, ApprovalPolicy, BoxFuture, Cancellation, Context, Error, ErrorCategory, Tool,
    ToolCall, ToolContext, ToolDecision, ToolDefinition, ToolOutput, ToolPolicy,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Weak},
};
use tokio::sync::watch;

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Sandbox(#[from] adk_sandbox::Error),
    #[error("tool teardown failed: {0}")]
    Teardown(String),
    #[error("tool bundle is closed")]
    Closed,
}

/// Host dependencies are explicit; no executable, store or credential discovery.
/// Inject Git, plan, skills and memory implementations through `implementations`.
/// Use the typed shell/LSP inputs for resources whose teardown this owner manages.
pub struct BundleBuilder {
    config: Config,
    implementations: Vec<Arc<dyn Tool>>,
    extra_tools: Vec<Arc<dyn Tool>>,
    composed_tools: Vec<Arc<dyn Tool>>,
    shell: Option<adk_sandbox::Config>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    lsp: Option<crate::lsp::Config>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    browser: Option<crate::browser::Config>,
}

impl BundleBuilder {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            implementations: Vec::new(),
            extra_tools: Vec::new(),
            composed_tools: Vec::new(),
            shell: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            lsp: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            browser: None,
        }
    }

    pub fn implementations(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        self.implementations.extend(tools);
        self
    }

    pub fn extra_tools(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        self.extra_tools.extend(tools);
        self
    }

    /// Host-composed integrations selected independently of the ExtraTools feature.
    pub fn composed_tools(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        self.composed_tools.extend(tools);
        self
    }

    pub fn shell(mut self, sandbox: adk_sandbox::Config) -> Self {
        self.shell = Some(sandbox);
        self
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn lsp(mut self, config: crate::lsp::Config) -> Self {
        self.lsp = Some(config);
        self
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn browser(mut self, config: crate::browser::Config) -> Self {
        self.browser = Some(config);
        self
    }

    pub fn build(mut self, policy: ToolPolicy) -> Result<ToolBundle, BundleError> {
        let selected = select(&self.config)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let needs = |name: &str| selected.iter().any(|c| c.name == name);
        let shell = if selected.iter().any(|c| {
            matches!(
                c.family.as_str(),
                "bash" | "async-shell" | "interactive-terminal"
            )
        }) {
            self.shell
                .take()
                .map(|sandbox| {
                    shell::ShellBundle::new(shell::Config {
                        sandbox,
                        access: self.config.access,
                        git_remote_writes: self.config.git_remote_writes,
                        environment: self.config.shell_environment.clone(),
                    })
                })
                .transpose()?
        } else {
            None
        };
        if let Some(shell) = &shell {
            self.implementations.extend(shell.tools());
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let lsp = if needs("LSP") {
            self.lsp.take().map(crate::lsp::tool)
        } else {
            None
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(lsp) = &lsp {
            self.implementations.push(lsp.clone());
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let browser_owner = if needs("Browser")
            && let Some(mut browser) = self.browser.take()
        {
            if browser
                .executable
                .as_ref()
                .is_none_or(|path| !path.is_absolute())
            {
                return Err(BuildError::Unavailable(vec!["Browser".into()]).into());
            }
            browser.access = self.config.access;
            browser.allow_private_network_urls = self.config.allow_private_network_urls;
            let owner = Arc::new(crate::browser::ManagedRunner::new(browser.runner.clone()));
            browser.runner = owner.clone();
            self.implementations.push(crate::browser::tool(browser));
            Some(owner)
        } else {
            None
        };
        let mut registry =
            Registry::build_with_extra_tools(&self.config, self.implementations, self.extra_tools)?;
        registry.add_composed_tools(&self.config, self.composed_tools)?;
        // Optional host injections remain optional, matching the SDK bundle.
        // Registry construction already rejects absent runtime implementations.
        let mut prepared = registry.prepare(policy);
        prepared.policy.allowed_tools = Some(
            prepared
                .tools
                .iter()
                .map(|tool| tool.definition().name.clone())
                .collect(),
        );
        let (closed, receiver) = watch::channel(false);
        let handles = prepared
            .tools
            .iter()
            .map(|tool| {
                Arc::new(OwnedTool {
                    tool: Arc::downgrade(tool),
                    definition: tool.definition().clone(),
                    control_flow: tool.is_control_flow(),
                    timeout: tool.timeout(),
                    ceiling: None,
                    closed: receiver.clone(),
                }) as Arc<dyn Tool>
            })
            .collect();
        Ok(ToolBundle {
            config: self.config,
            builtin_names: selected.iter().map(|c| c.name.clone()).collect(),
            tools: prepared.tools,
            view_tools: Vec::new(),
            prepared: PreparedTools {
                tools: handles,
                policy: prepared.policy,
            },
            closed,
            shell,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            lsp,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            browser: browser_owner,
        })
    }
}

/// Non-cloneable lifecycle owner. Prepared handles do not extend its lifetime.
pub struct ToolBundle {
    config: Config,
    builtin_names: BTreeSet<String>,
    tools: Vec<Arc<dyn Tool>>,
    view_tools: Vec<Arc<dyn Tool>>,
    prepared: PreparedTools,
    closed: watch::Sender<bool>,
    shell: Option<shell::ShellBundle>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    lsp: Option<Arc<crate::lsp::LspTool>>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    browser: Option<Arc<crate::browser::ManagedRunner>>,
}

impl ToolBundle {
    pub fn prepared(&self) -> PreparedTools {
        PreparedTools {
            tools: self.prepared.tools.clone(),
            policy: self.prepared.policy.clone(),
        }
    }

    /// Create an owner-bound, non-escalating view for a specialist agent.
    ///
    /// Adapters share this bundle's resources. Mutation exceptions and control-flow
    /// grants are not inherited; caller exclusions apply before adaptation. Saved
    /// handles stop working when the owner closes or drops, just like parent handles.
    pub fn role_view(
        &mut self,
        access: AccessMode,
        excluded: &BTreeSet<String>,
    ) -> Result<PreparedTools, BundleError> {
        if *self.closed.borrow() {
            return Err(BundleError::Closed);
        }
        let mut policy = self.prepared.policy.clone();
        policy.access = narrow_access(narrow_access(policy.access, self.config.access), access);
        policy.allowed_mutating_tools.clear();
        let mut config = self.config.clone();
        config.access = policy.access;
        config.allowed_mutating_tools.clear();
        let selected: BTreeSet<_> = select(&config)?
            .into_iter()
            .map(|c| c.name.clone())
            .collect();
        let mut adapted = Vec::new();
        let mut names = BTreeSet::new();
        for tool in &self.tools {
            let name = &tool.definition().name;
            if excluded.contains(name)
                || (self.builtin_names.contains(name) && !selected.contains(name))
            {
                continue;
            }
            let tool = tool
                .for_access(policy.access)
                .unwrap_or_else(|| tool.clone());
            if &tool.definition().name != name {
                return Err(BuildError::Contract(format!("access adapter renamed {name}")).into());
            }
            if policy.decision(tool.definition()) == ToolDecision::Deny {
                continue;
            }
            if !names.insert(name.clone()) {
                return Err(BuildError::Duplicate(name.clone()).into());
            }
            adapted.push(tool);
        }
        policy.allowed_tools = Some(names);
        let handles = adapted
            .iter()
            .map(|tool| {
                Arc::new(OwnedTool {
                    tool: Arc::downgrade(tool),
                    definition: tool.definition().clone(),
                    control_flow: tool.is_control_flow(),
                    timeout: tool.timeout(),
                    ceiling: Some(policy.clone()),
                    closed: self.closed.subscribe(),
                }) as Arc<dyn Tool>
            })
            .collect();
        self.view_tools.extend(adapted);
        Ok(PreparedTools {
            tools: handles,
            policy,
        })
    }

    /// Join caller cancellation with this owner's lifetime, without cancelling the caller.
    pub fn context(&self, context: &Context) -> Context {
        scoped_context(context, self.closed.subscribe())
    }

    pub async fn close(&mut self) -> Result<(), BundleError> {
        self.closed.send_replace(true);
        if let Some(shell) = &self.shell {
            shell.close().await;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let result = match &self.lsp {
            Some(lsp) => lsp.close().await.map_err(BundleError::Teardown),
            None => Ok(()),
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let result = {
            let browser = match &self.browser {
                Some(browser) => browser.close().await.map_err(BundleError::Teardown),
                None => Ok(()),
            };
            result.and(browser)
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let result = Ok(());
        self.tools.clear();
        self.view_tools.clear();
        result
    }
}

impl Drop for ToolBundle {
    fn drop(&mut self) {
        self.closed.send_replace(true);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(browser) = &self.browser {
            browser.cancel();
        }
    }
}

struct ScopeCancellation {
    parent: Arc<dyn Cancellation>,
    closed: watch::Receiver<bool>,
}
impl Cancellation for ScopeCancellation {
    fn is_cancelled(&self) -> bool {
        *self.closed.borrow() || self.parent.is_cancelled()
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut closed = self.closed.clone();
            tokio::select! {
                _ = self.parent.cancelled() => {},
                _ = closed.wait_for(|closed| *closed) => {},
            }
        })
    }
}
fn scoped_context(context: &Context, closed: watch::Receiver<bool>) -> Context {
    Context {
        cancellation: Arc::new(ScopeCancellation {
            parent: context.cancellation.clone(),
            closed,
        }),
        ..context.clone()
    }
}

fn narrow_access(a: AccessMode, b: AccessMode) -> AccessMode {
    match (a, b) {
        (AccessMode::ReadOnly, _) | (_, AccessMode::ReadOnly) => AccessMode::ReadOnly,
        (AccessMode::WorkspaceWrite, _) | (_, AccessMode::WorkspaceWrite) => {
            AccessMode::WorkspaceWrite
        }
        _ => AccessMode::FullAccess,
    }
}

fn intersect_policy(caller: &ToolPolicy, ceiling: &ToolPolicy) -> ToolPolicy {
    let mut policy = caller.clone();
    policy.access = narrow_access(caller.access, ceiling.access);
    policy.allowed_tools = match (&caller.allowed_tools, &ceiling.allowed_tools) {
        (Some(a), Some(b)) => Some(a.intersection(b).cloned().collect()),
        (Some(a), None) | (None, Some(a)) => Some(a.clone()),
        (None, None) => None,
    };
    policy
        .denied_tools
        .extend(ceiling.denied_tools.iter().cloned());
    policy.allowed_mutating_tools = caller
        .allowed_mutating_tools
        .intersection(&ceiling.allowed_mutating_tools)
        .cloned()
        .collect();
    if ceiling.approval == ApprovalPolicy::All {
        policy.approval = ApprovalPolicy::All;
    }
    policy.timeout = match (caller.timeout, ceiling.timeout) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    policy.max_child_turns = match (caller.max_child_turns, ceiling.max_child_turns) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    policy
}

struct OwnedTool {
    tool: Weak<dyn Tool>,
    definition: ToolDefinition,
    control_flow: bool,
    timeout: Option<std::time::Duration>,
    ceiling: Option<ToolPolicy>,
    closed: watch::Receiver<bool>,
}
impl Tool for OwnedTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn is_control_flow(&self) -> bool {
        self.control_flow
    }
    fn timeout(&self) -> Option<std::time::Duration> {
        self.timeout
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            let policy = self.ceiling.as_ref().map_or_else(
                || context.policy.clone(),
                |ceiling| intersect_policy(&context.policy, ceiling),
            );
            let context = ToolContext {
                operation: scoped_context(&context.operation, self.closed.clone()),
                work_dir: context.work_dir.clone(),
                policy,
                idempotency_key: context.idempotency_key.clone(),
            };
            context.operation.check_active()?;
            if self.ceiling.is_some()
                && context.policy.decision(&self.definition) == ToolDecision::Deny
            {
                return Err(Error::new(
                    ErrorCategory::PermissionDenied,
                    "specialist tool access denied",
                ));
            }
            let tool = self
                .tool
                .upgrade()
                .ok_or_else(|| Error::new(ErrorCategory::Cancelled, "tool bundle is closed"))?;
            tokio::select! {
                biased;
                _ = context.operation.cancellation.cancelled() => Err(Error::new(ErrorCategory::Cancelled, "tool execution cancelled")),
                result = tool.execute(&context, call) => result,
            }
        })
    }
}
