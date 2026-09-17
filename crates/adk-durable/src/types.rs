use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

use crate::{Error, Result};

pub const SCHEMA_VERSION: i32 = 2;

pub fn zero_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0).unwrap()
}

macro_rules! ids {
    ($($name:ident => $prefix:literal),* $(,)?) => {$ (
        #[derive(Clone, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl $name {
            pub fn new() -> Self { Self(format!("{}_{}", $prefix, uuid::Uuid::new_v4())) }
            pub fn as_str(&self) -> &str { &self.0 }
            pub fn is_empty(&self) -> bool { self.0.is_empty() }
        }
        impl From<String> for $name { fn from(value: String) -> Self { Self(value) } }
        impl From<&str> for $name { fn from(value: &str) -> Self { Self(value.into()) } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
    )*};
}
ids! { TenantId => "ten", RunId => "run", AttemptId => "att", StepId => "step",
ToolCallId => "tool", ApprovalId => "approval", ChildRunId => "child",
EffectId => "effect", EventId => "event", LeaseToken => "lease" }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClassification {
    #[default]
    #[serde(rename = "")]
    Unspecified,
    Public,
    Internal,
    Sensitive,
    Secret,
}
impl DataClassification {
    pub fn is_unspecified(&self) -> bool {
        *self == Self::Unspecified
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    #[serde(rename = "")]
    Unspecified,
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Attempt {
    pub id: AttemptId,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub outcome: String,
}
impl Default for Attempt {
    fn default() -> Self {
        Self {
            id: Default::default(),
            started_at: zero_time(),
            ended_at: zero_time(),
            outcome: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub id: StepId,
    pub kind: String,
    pub status: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub data: Option<Value>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}
impl Default for Step {
    fn default() -> Self {
        Self {
            id: Default::default(),
            kind: String::new(),
            status: String::new(),
            data: None,
            started_at: zero_time(),
            ended_at: zero_time(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "DataClassification::is_unspecified")]
    pub classification: DataClassification,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub input: Option<Value>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub output: Option<Value>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}
impl Default for ToolCall {
    fn default() -> Self {
        Self {
            id: Default::default(),
            name: String::new(),
            status: String::new(),
            classification: Default::default(),
            input: None,
            output: None,
            started_at: zero_time(),
            ended_at: zero_time(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Approval {
    pub id: ApprovalId,
    pub status: String,
    pub requested_at: DateTime<Utc>,
    pub resolved_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub resolved_by: String,
}
impl Default for Approval {
    fn default() -> Self {
        Self {
            id: Default::default(),
            status: String::new(),
            requested_at: zero_time(),
            resolved_at: zero_time(),
            resolved_by: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChildRun {
    pub id: ChildRunId,
    pub run_id: RunId,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}
impl Default for ChildRun {
    fn default() -> Self {
        Self {
            id: Default::default(),
            run_id: Default::default(),
            status: Default::default(),
            started_at: zero_time(),
            ended_at: zero_time(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cancellation {
    pub requested_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub requested_by: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    pub acknowledged_at: DateTime<Utc>,
}
impl Default for Cancellation {
    fn default() -> Self {
        Self {
            requested_at: zero_time(),
            requested_by: String::new(),
            reason: String::new(),
            acknowledged_at: zero_time(),
        }
    }
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetCounters {
    #[serde(skip_serializing_if = "is_zero")]
    pub input_tokens: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub output_tokens: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub tool_calls: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub cost_micros: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub wall_time_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClassification {
    Idempotent,
    Deduplicated,
    NonReplayable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Prepared,
    Dispatched,
    Succeeded,
    Failed,
    OutcomeUnknown,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub id: EffectId,
    pub classification: EffectClassification,
    #[serde(default, skip_serializing_if = "DataClassification::is_unspecified")]
    pub data_classification: DataClassification,
    pub state: EffectState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub idempotency_key: String,
    #[serde(default = "zero_time")]
    pub prepared_at: DateTime<Utc>,
    #[serde(default = "zero_time")]
    pub updated_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub outcome: Option<Value>,
}
impl Effect {
    pub fn new(run: &RunId, classification: EffectClassification, now: DateTime<Utc>) -> Self {
        let id = EffectId::new();
        let idempotency_key = idempotency_key(run, &id);
        Self {
            id,
            classification,
            data_classification: Default::default(),
            state: EffectState::Prepared,
            idempotency_key,
            prepared_at: now,
            updated_at: now,
            outcome: None,
        }
    }
}
pub fn idempotency_key(run: &RunId, effect: &EffectId) -> String {
    format!(
        "ga_{:x}",
        Sha256::digest(format!("{run}:{effect}").as_bytes())
    )
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    None,
    Retry,
    Reconcile,
    OperatorResolution,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryDecision {
    pub action: RecoveryAction,
    pub automatic: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub idempotency_key: String,
}
pub fn recover_effect(effect: &Effect) -> RecoveryDecision {
    use EffectState::*;
    use RecoveryAction::*;
    let (action, automatic) = match (effect.state, effect.classification) {
        (Prepared, _) => (Retry, true),
        (Dispatched, EffectClassification::NonReplayable) => (Reconcile, false),
        (OutcomeUnknown, EffectClassification::NonReplayable) => (OperatorResolution, false),
        (Dispatched | OutcomeUnknown, _) => (Retry, true),
        _ => (None, false),
    };
    RecoveryDecision {
        action,
        automatic,
        idempotency_key: effect.idempotency_key.clone(),
    }
}
pub fn transition_effect(effect: &mut Effect, next: EffectState, now: DateTime<Utc>) -> Result<()> {
    use EffectState::*;
    if !matches!(
        (effect.state, next),
        (Prepared, Dispatched | Failed)
            | (Dispatched, Succeeded | Failed | OutcomeUnknown)
            | (OutcomeUnknown, Succeeded | Failed)
    ) {
        return Err(Error::Invalid(format!(
            "invalid effect transition {:?} -> {next:?}",
            effect.state
        )));
    }
    effect.state = next;
    effect.updated_at = now;
    Ok(())
}
pub fn mark_interrupted_effect(effect: &mut Effect, now: DateTime<Utc>) -> Result<()> {
    if effect.state == EffectState::Dispatched {
        transition_effect(effect, EffectState::OutcomeUnknown, now)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub run_id: RunId,
    pub sequence: u64,
    pub at: DateTime<Utc>,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "DataClassification::is_unspecified")]
    pub classification: DataClassification,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub payload: Option<Value>,
}
impl Default for Event {
    fn default() -> Self {
        Self {
            id: Default::default(),
            tenant_id: Default::default(),
            run_id: Default::default(),
            sequence: 0,
            at: zero_time(),
            event_type: String::new(),
            classification: Default::default(),
            payload: None,
        }
    }
}

pub(crate) fn null_vec<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}
pub(crate) fn present_value<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Ok(Some(Value::deserialize(d)?))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunSnapshot {
    pub schema_version: i32,
    pub tenant_id: TenantId,
    pub run_id: RunId,
    pub revision: u64,
    pub event_sequence: u64,
    pub status: RunStatus,
    #[serde(skip_serializing_if = "DataClassification::is_unspecified")]
    pub classification: DataClassification,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub state: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub attempts: Vec<Attempt>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub steps: Vec<Step>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub approvals: Vec<Approval>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub child_runs: Vec<ChildRun>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub effects: Vec<Effect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<Cancellation>,
    pub cumulative_budget: BudgetCounters,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retain_until: Option<DateTime<Utc>>,
}
impl Default for RunSnapshot {
    fn default() -> Self {
        Self {
            schema_version: 0,
            tenant_id: Default::default(),
            run_id: Default::default(),
            revision: 0,
            event_sequence: 0,
            status: Default::default(),
            classification: Default::default(),
            state: None,
            attempts: vec![],
            steps: vec![],
            tool_calls: vec![],
            approvals: vec![],
            child_runs: vec![],
            effects: vec![],
            cancellation: None,
            cumulative_budget: Default::default(),
            created_at: zero_time(),
            updated_at: zero_time(),
            retain_until: None,
        }
    }
}
impl RunSnapshot {
    pub fn new(tenant_id: TenantId, run_id: RunId, now: DateTime<Utc>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            tenant_id,
            run_id,
            status: RunStatus::Pending,
            created_at: now,
            updated_at: now,
            ..Self::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lease {
    pub tenant_id: TenantId,
    pub run_id: RunId,
    pub owner: String,
    pub token: LeaseToken,
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub schema_version: i32,
    pub snapshot: RunSnapshot,
    #[serde(default, deserialize_with = "null_vec")]
    pub events: Vec<Event>,
}
