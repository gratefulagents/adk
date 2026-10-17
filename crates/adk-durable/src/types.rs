use chrono::{DateTime, FixedOffset, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

use crate::{Error, Result};

pub const SCHEMA_VERSION: i32 = 2;

pub fn zero_time() -> DateTime<FixedOffset> {
    Utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0)
        .unwrap()
        .fixed_offset()
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

macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $wire:literal),* $(,)? }) => {
        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name { $($variant,)* Unknown(String) }
        impl From<String> for $name {
            fn from(value: String) -> Self {
                match value.as_str() { $($wire => Self::$variant,)* _ => Self::Unknown(value) }
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                match value { $($name::$variant => $wire.into(),)* $name::Unknown(value) => value }
            }
        }
        impl Default for $name {
            fn default() -> Self { String::new().into() }
        }
    };
}
wire_enum!(DataClassification {
    Unspecified => "", Public => "public", Internal => "internal",
    Sensitive => "sensitive", Secret => "secret",
});
impl DataClassification {
    pub fn is_unspecified(&self) -> bool {
        *self == Self::Unspecified
    }
}
wire_enum!(RunStatus {
    Unspecified => "", Pending => "pending", Running => "running",
    Succeeded => "succeeded", Failed => "failed", Cancelled => "cancelled",
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Attempt {
    #[serde(deserialize_with = "null_default")]
    pub id: AttemptId,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub started_at: DateTime<FixedOffset>,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub ended_at: DateTime<FixedOffset>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
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
    #[serde(deserialize_with = "null_default")]
    pub id: StepId,
    #[serde(deserialize_with = "null_default")]
    pub kind: String,
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub data: Option<Value>,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub started_at: DateTime<FixedOffset>,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub ended_at: DateTime<FixedOffset>,
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
    #[serde(deserialize_with = "null_default")]
    pub id: ToolCallId,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    #[serde(skip_serializing_if = "DataClassification::is_unspecified")]
    #[serde(deserialize_with = "null_default")]
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
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub started_at: DateTime<FixedOffset>,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub ended_at: DateTime<FixedOffset>,
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
    #[serde(deserialize_with = "null_default")]
    pub id: ApprovalId,
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub requested_at: DateTime<FixedOffset>,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub resolved_at: DateTime<FixedOffset>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
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
    #[serde(deserialize_with = "null_default")]
    pub id: ChildRunId,
    #[serde(deserialize_with = "null_default")]
    pub run_id: RunId,
    #[serde(deserialize_with = "null_default")]
    pub status: RunStatus,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub started_at: DateTime<FixedOffset>,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub ended_at: DateTime<FixedOffset>,
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
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub requested_at: DateTime<FixedOffset>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub requested_by: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub reason: String,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub acknowledged_at: DateTime<FixedOffset>,
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
    #[serde(deserialize_with = "null_default")]
    pub input_tokens: i64,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub output_tokens: i64,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub tool_calls: i64,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub cost_micros: i64,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub wall_time_ms: i64,
}

wire_enum!(EffectClassification {
    Idempotent => "idempotent", Deduplicated => "deduplicated", NonReplayable => "non_replayable",
});
wire_enum!(EffectState {
    Prepared => "prepared", Dispatched => "dispatched", Succeeded => "succeeded",
    Failed => "failed", OutcomeUnknown => "outcome_unknown",
});
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effect {
    #[serde(deserialize_with = "null_default")]
    pub id: EffectId,
    #[serde(deserialize_with = "null_default")]
    pub classification: EffectClassification,
    #[serde(default, skip_serializing_if = "DataClassification::is_unspecified")]
    #[serde(deserialize_with = "null_default")]
    pub data_classification: DataClassification,
    #[serde(deserialize_with = "null_default")]
    pub state: EffectState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub idempotency_key: String,
    #[serde(default = "zero_time")]
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub prepared_at: DateTime<FixedOffset>,
    #[serde(default = "zero_time")]
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub updated_at: DateTime<FixedOffset>,
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
            prepared_at: now.fixed_offset(),
            updated_at: now.fixed_offset(),
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
wire_enum!(RecoveryAction {
    None => "none", Retry => "retry", Reconcile => "reconcile", OperatorResolution => "operator_resolution",
});
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecoveryDecision {
    #[serde(deserialize_with = "null_default")]
    pub action: RecoveryAction,
    #[serde(deserialize_with = "null_default")]
    pub automatic: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub idempotency_key: String,
}
pub fn recover_effect(effect: &Effect) -> RecoveryDecision {
    use EffectState::*;
    use RecoveryAction::*;
    let (action, automatic) = match (&effect.state, &effect.classification) {
        (EffectState::Unknown(_), _) | (_, EffectClassification::Unknown(_)) => (None, false),
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
        (&effect.state, &next),
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
    effect.updated_at = now.fixed_offset();
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
    #[serde(deserialize_with = "null_default")]
    pub id: EventId,
    #[serde(deserialize_with = "null_default")]
    pub tenant_id: TenantId,
    #[serde(deserialize_with = "null_default")]
    pub run_id: RunId,
    #[serde(deserialize_with = "null_default")]
    pub sequence: u64,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub at: DateTime<FixedOffset>,
    #[serde(rename = "type")]
    #[serde(deserialize_with = "null_default")]
    pub event_type: String,
    #[serde(skip_serializing_if = "DataClassification::is_unspecified")]
    #[serde(deserialize_with = "null_default")]
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
    #[serde(deserialize_with = "null_default")]
    pub schema_version: i32,
    #[serde(deserialize_with = "null_default")]
    pub tenant_id: TenantId,
    #[serde(deserialize_with = "null_default")]
    pub run_id: RunId,
    #[serde(deserialize_with = "null_default")]
    pub revision: u64,
    #[serde(deserialize_with = "null_default")]
    pub event_sequence: u64,
    #[serde(deserialize_with = "null_default")]
    pub status: RunStatus,
    #[serde(skip_serializing_if = "DataClassification::is_unspecified")]
    #[serde(deserialize_with = "null_default")]
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
    #[serde(deserialize_with = "null_default")]
    pub cumulative_budget: BudgetCounters,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub created_at: DateTime<FixedOffset>,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub updated_at: DateTime<FixedOffset>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_time"
    )]
    pub retain_until: Option<DateTime<FixedOffset>>,
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
            created_at: now.fixed_offset(),
            updated_at: now.fixed_offset(),
            ..Self::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lease {
    #[serde(deserialize_with = "null_default")]
    pub tenant_id: TenantId,
    #[serde(deserialize_with = "null_default")]
    pub run_id: RunId,
    #[serde(deserialize_with = "null_default")]
    pub owner: String,
    #[serde(deserialize_with = "null_default")]
    pub token: LeaseToken,
    #[serde(deserialize_with = "null_time", serialize_with = "serialize_time")]
    pub expires_at: DateTime<FixedOffset>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub schema_version: i32,
    pub snapshot: RunSnapshot,
    #[serde(default, deserialize_with = "null_vec")]
    pub events: Vec<Event>,
}

fn null_default<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> std::result::Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
fn time_string(time: &DateTime<FixedOffset>) -> String {
    let mut wire = time.to_rfc3339_opts(SecondsFormat::Nanos, true);
    let dot = wire.find('.').unwrap();
    let end = dot + 10;
    let trimmed = wire[dot..end]
        .trim_end_matches('0')
        .trim_end_matches('.')
        .len();
    wire.replace_range(dot + trimmed..end, "");
    wire
}
fn serialize_time<S: serde::Serializer>(
    time: &DateTime<FixedOffset>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&time_string(time))
}
fn serialize_optional_time<S: serde::Serializer>(
    time: &Option<DateTime<FixedOffset>>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    time.as_ref().map(time_string).serialize(serializer)
}
fn null_time<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<DateTime<FixedOffset>, D::Error> {
    Ok(Option::<DateTime<FixedOffset>>::deserialize(d)?.unwrap_or_else(zero_time))
}
impl Default for Effect {
    fn default() -> Self {
        Self {
            id: Default::default(),
            classification: Default::default(),
            data_classification: Default::default(),
            state: Default::default(),
            idempotency_key: String::new(),
            prepared_at: zero_time(),
            updated_at: zero_time(),
            outcome: None,
        }
    }
}
impl Default for Lease {
    fn default() -> Self {
        Self {
            tenant_id: Default::default(),
            run_id: Default::default(),
            owner: String::new(),
            token: Default::default(),
            expires_at: zero_time(),
        }
    }
}
