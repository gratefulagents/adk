//! Bridge from exact-command authorization to the trusted subprocess executor.
//!
//! Compose all host policies, call [`adk_security::SecurityPolicy::authorize`],
//! and consume the resulting capability here. This adapter fixes the shell,
//! workspace-relative working directory and environment; model input cannot
//! replace the approved command, request stronger access, or inject credentials.
//! Interactive PTY hosts can use the lower-level sandbox API, but must preserve
//! these authorization and output-guardrail obligations themselves.

use adk_core::{Context, Error, ErrorCategory};
use adk_sandbox::{Completion, Executor, Network, Request, RunResult};
use adk_security::{AuthorizedCommand, check_secrets};

/// Execute one immutable approved command, with no raw approval-bit shortcut.
/// The capability is consumed even when execution fails. A deferred approval is
/// not a capability and cannot be passed here. The executor must have been built
/// by the trusted host for the same workspace the user was shown at approval.
///
/// Output is checked before it is returned to the embedding tool. On a detected
/// secret, no subprocess output is included in the error. Callers must not log
/// raw output from the lower-level executor before applying equivalent checks.
pub async fn run_authorized(
    executor: &Executor,
    context: &Context,
    authorized: AuthorizedCommand,
) -> Result<RunResult, Error> {
    context.check_active()?;
    if authorized.run_id() != context.run_id {
        return Err(Error::new(
            ErrorCategory::PermissionDenied,
            "command authorization belongs to a different run",
        ));
    }
    let mut request = Request::new("/bin/sh");
    request.args = vec!["-c".into(), authorized.command().to_owned()];
    request.access = authorized.policy().tools.access;
    request.timeout = authorized.policy().tools.timeout;
    request.network = if authorized.policy().allow_network {
        Network::Allow
    } else {
        Network::Deny
    };
    let result = executor.run(context, request).await.map_err(|error| {
        let category = match &error {
            adk_sandbox::Error::Invalid(_) => ErrorCategory::InvalidInput,
            adk_sandbox::Error::Unavailable(_) => ErrorCategory::Unsupported,
            adk_sandbox::Error::Cancelled => ErrorCategory::Cancelled,
            adk_sandbox::Error::TimedOut => ErrorCategory::DeadlineExceeded,
            _ => ErrorCategory::Tool,
        };
        // Keep OS diagnostics outside serialized errors: paths can contain
        // sensitive host data even though command environments are filtered.
        Error::new(category, "subprocess execution failed").with_source(error)
    })?;
    match result.completion {
        Completion::Cancelled => {
            return Err(Error::new(ErrorCategory::Cancelled, "subprocess cancelled"));
        }
        Completion::TimedOut => {
            return Err(Error::new(
                ErrorCategory::DeadlineExceeded,
                "subprocess timed out",
            ));
        }
        Completion::Exited => {}
    }
    check_secrets(&String::from_utf8_lossy(&result.stdout))?;
    check_secrets(&String::from_utf8_lossy(&result.stderr))?;
    Ok(result)
}
