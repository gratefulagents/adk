//! Host-owned policy and guardrails. Authorization is not OS confinement.
//! See the crate README for the accepted shell grammar and executor contract.

mod policy;
mod secrets;
mod shell;

pub use policy::{SecurityPolicy, clamp_access, normalize_access};
pub use secrets::{SecretKind, check_secrets, redact_secrets};
pub use shell::{CommandClass, classify_command, inspect_literal_commands, validate_git_push};

use adk_core::{
    ApprovalDecision, ApprovalRequest, Context, Error, ErrorCategory, Host, ToolCall, ToolDecision,
    ToolDefinition,
};

/// Exact model call with a validated command-only schema; no permission fields.
/// This type intentionally has no deserializer or mutable accessors.
pub struct CommandRequest {
    call: ToolCall,
    command: String,
}

impl CommandRequest {
    pub fn from_call(call: ToolCall) -> Result<Self, Error> {
        let object = call
            .arguments
            .as_object()
            .ok_or_else(|| invalid("expected command object"))?;
        if object.len() != 1 {
            return Err(invalid("only the command argument is supported"));
        }
        let command = object
            .get("command")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| invalid("expected nonempty command string"))?
            .to_owned();
        Ok(Self { call, command })
    }

    pub fn call(&self) -> &ToolCall {
        &self.call
    }
    pub fn command(&self) -> &str {
        &self.command
    }
}

/// Single-use by convention: consume this value at the sandbox execution boundary.
/// Execute only `command()`, using `policy()` and the approving run's context.
/// No Clone, Deserialize, public constructor or permission-bearing model input.
pub struct AuthorizedCommand {
    request: CommandRequest,
    policy: SecurityPolicy,
    run_id: String,
}

impl AuthorizedCommand {
    pub fn call(&self) -> &ToolCall {
        self.request.call()
    }
    pub fn command(&self) -> &str {
        self.request.command()
    }
    pub fn policy(&self) -> &SecurityPolicy {
        &self.policy
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

pub enum Authorization {
    Approved(AuthorizedCommand),
    Deferred(ApprovalRequest),
}

fn invalid(message: &'static str) -> Error {
    Error::new(ErrorCategory::InvalidInput, message)
}

impl SecurityPolicy {
    /// Definition and policy must come from the trusted host registry. Compose all
    /// platform/run/tool policies before calling. A deferred call must be checked
    /// again on resume; there is intentionally no API to inject an approval bit.
    pub async fn authorize(
        &self,
        context: &Context,
        host: &dyn Host,
        definition: &ToolDefinition,
        request: CommandRequest,
    ) -> Result<Authorization, Error> {
        context.check_active()?;
        if request.call.name != definition.name {
            return Err(invalid("tool call does not match registered definition"));
        }
        let decision = self.tools.decision(definition);
        if decision == ToolDecision::Deny {
            return Err(Error::new(
                ErrorCategory::PermissionDenied,
                "tool policy denied call",
            ));
        }
        check_secrets(request.command())?;
        let class = classify_command(request.command(), self.tools.access, self.git_remote_writes)?;
        if decision == ToolDecision::RequireApproval
            || (self.approve_mutations && class == CommandClass::Mutating)
        {
            let approval = ApprovalRequest {
                call: request.call.clone(),
                reason: "host approval required for this exact command".to_owned(),
            };
            let deadline = async {
                match context.deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let decision = tokio::select! {
                biased;
                _ = context.cancellation.cancelled() => return Err(Error::new(ErrorCategory::Cancelled, "approval cancelled")),
                _ = deadline => return Err(Error::new(ErrorCategory::DeadlineExceeded, "approval deadline exceeded")),
                decision = host.approve(context, approval.clone()) => decision?,
            };
            match decision {
                ApprovalDecision::Approve => {}
                ApprovalDecision::Deny => {
                    return Err(Error::new(
                        ErrorCategory::ApprovalDenied,
                        "host denied command",
                    ));
                }
                ApprovalDecision::Defer => {
                    context.check_active()?;
                    return Ok(Authorization::Deferred(approval));
                }
            }
        }
        context.check_active()?;
        Ok(Authorization::Approved(AuthorizedCommand {
            request,
            policy: self.clone(),
            run_id: context.run_id.clone(),
        }))
    }
}
