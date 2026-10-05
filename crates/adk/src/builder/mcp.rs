use super::{McpFeatures, McpInput, invalid};
use adk_core::{Context, Error, ErrorCategory};
use adk_mcp::session::{OwnedMcpSession, ToolSelection};
use std::{collections::BTreeSet, path::Path};

pub(super) async fn assemble(
    input: McpInput,
    features: &McpFeatures,
    work_dir: &Path,
    context: &Context,
) -> Result<OwnedMcpSession, Error> {
    let allowed: BTreeSet<_> = features
        .allowed_servers
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .collect();
    let servers = input
        .config
        .config()
        .servers()
        .iter()
        .filter(|(name, config)| {
            config.enabled() && (features.allow_all_servers || allowed.contains(name.as_str()))
        })
        .map(|(name, _)| name.clone())
        .collect();
    let tools = ToolSelection {
        allow_all: features.allow_all_tools,
        allowed: features
            .allowed_tools
            .iter()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect(),
        resources: features.resource_tools,
    };
    let deadline = async {
        match context.deadline {
            Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => Err(Error::new(ErrorCategory::Cancelled, "MCP assembly cancelled")),
        _ = deadline => Err(Error::new(ErrorCategory::DeadlineExceeded, "MCP assembly deadline exceeded")),
        result = OwnedMcpSession::connect(input, servers, tools, work_dir) => result.map_err(|error| invalid("MCP assembly failed").with_source(error)),
    }
}
