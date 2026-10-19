use crate::{BoxFuture, Context, Error};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const ACTION_DECISION_ALLOW: &str = "allow";
pub const ACTION_DECISION_DENY: &str = "deny";
pub const ACTION_DECISION_ASK: &str = "ask";
pub const ACTION_DECISION_CLASSIFY: &str = "classify";
pub const ACTION_RISK_LOW: &str = "low";
pub const ACTION_RISK_MEDIUM: &str = "medium";
pub const ACTION_RISK_HIGH: &str = "high";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionRequest {
    pub tool_name: String,
    /// Original UTF-8 argument text, including absent or malformed input.
    pub input: String,
    pub work_dir: PathBuf,
    pub read_only: bool,
    pub provenance: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionAuthorization {
    pub decision: String,
    pub reason: String,
    pub rule: String,
    pub risk: String,
}

pub fn normalize_action_authorization(
    authorization: ActionAuthorization,
    error: Option<&Error>,
) -> ActionAuthorization {
    let (reason, rule) = if error.is_some() {
        ("action authorizer failed", "authorizer-error")
    } else if matches!(
        authorization.decision.as_str(),
        ACTION_DECISION_ALLOW | ACTION_DECISION_DENY | ACTION_DECISION_ASK
    ) {
        return authorization;
    } else {
        (
            "action authorizer returned an invalid decision",
            "invalid-decision",
        )
    };
    ActionAuthorization {
        decision: ACTION_DECISION_DENY.into(),
        reason: reason.into(),
        rule: rule.into(),
        risk: authorization.risk,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct ActionAuthorizationFailure {
    pub risk: String,
    #[source]
    pub error: Error,
}

impl From<Error> for ActionAuthorizationFailure {
    fn from(error: Error) -> Self {
        Self {
            risk: String::new(),
            error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionAuditRecord {
    pub request: ActionRequest,
    pub authorization: ActionAuthorization,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// Host-owned authorization can restrict an action, never relax the native policy ceiling.
/// Requests and audit records may contain private arguments; do not log them unredacted.
pub trait ActionAuthorizer: Send + Sync {
    fn authorize<'a>(
        &'a self,
        context: &'a Context,
        request: ActionRequest,
    ) -> BoxFuture<'a, Result<ActionAuthorization, ActionAuthorizationFailure>>;

    /// Opt in only when repeated authorization is safe and this key identifies every
    /// decision-affecting policy input. Changed bindings cannot resume a checkpoint.
    fn durable_key(&self) -> Option<&str> {
        None
    }
}
