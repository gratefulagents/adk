use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    #[serde(deserialize_with = "null_default")]
    pub seq: i64,
    #[serde(deserialize_with = "null_default")]
    pub event_id: String,
    #[serde(deserialize_with = "null_default")]
    pub project_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub run_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub actor: String,
    #[serde(
        default = "go_time::zero",
        deserialize_with = "go_time::deserialize",
        serialize_with = "go_time::serialize"
    )]
    pub time: DateTime<FixedOffset>,
    #[serde(rename = "type")]
    #[serde(deserialize_with = "null_default")]
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub schema_version: i32,
    pub project_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub workdir: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub state_dir: String,
    #[serde(serialize_with = "go_time::serialize")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "go_time::serialize")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub description: String,
    #[serde(rename = "type")]
    #[serde(deserialize_with = "null_default")]
    pub task_type: String,
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    #[serde(deserialize_with = "null_default")]
    pub priority: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub assignee: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub blocks: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub comments: Vec<TaskComment>,
    #[serde(
        default = "go_time::zero",
        deserialize_with = "go_time::deserialize",
        serialize_with = "go_time::serialize"
    )]
    pub created_at: DateTime<FixedOffset>,
    #[serde(
        default = "go_time::zero",
        deserialize_with = "go_time::deserialize",
        serialize_with = "go_time::serialize"
    )]
    pub updated_at: DateTime<FixedOffset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "go_time::serialize_optional")]
    pub closed_at: Option<DateTime<FixedOffset>>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub source_run: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskComment {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub actor: String,
    #[serde(deserialize_with = "null_default")]
    pub body: String,
    #[serde(
        default = "go_time::zero",
        deserialize_with = "go_time::deserialize",
        serialize_with = "go_time::serialize"
    )]
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub kind: String,
    #[serde(deserialize_with = "null_default")]
    pub scope: String,
    #[serde(deserialize_with = "null_default")]
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub task_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub file_paths: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub source_run: String,
    #[serde(
        default = "go_time::zero",
        deserialize_with = "go_time::deserialize",
        serialize_with = "go_time::serialize"
    )]
    pub created_at: DateTime<FixedOffset>,
    #[serde(
        default = "go_time::zero",
        deserialize_with = "go_time::deserialize",
        serialize_with = "go_time::serialize"
    )]
    pub updated_at: DateTime<FixedOffset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "go_time::serialize_optional")]
    pub last_read_at: Option<DateTime<FixedOffset>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSummary {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub run_id: String,
    #[serde(deserialize_with = "null_default")]
    pub summary: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub task_ids: Vec<String>,
    #[serde(
        default = "go_time::zero",
        deserialize_with = "go_time::deserialize",
        serialize_with = "go_time::serialize"
    )]
    pub created_at: DateTime<FixedOffset>,
    #[serde(
        default = "go_time::zero",
        deserialize_with = "go_time::deserialize",
        serialize_with = "go_time::serialize"
    )]
    pub updated_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateTaskInput {
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(rename = "type", skip_serializing_if = "String::is_empty")]
    pub task_type: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub priority: i32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub assignee: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_run: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub replace_labels: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskFilter {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub actor: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub assignee: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub limit: i32,
    #[serde(skip_serializing_if = "is_false")]
    pub include_assigned: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpsertMemoryInput {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scope: String,
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub task_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub file_paths: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_run: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryFilter {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub query: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub kinds: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub limit: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrimeOptions {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub actor: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub active_task_id: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub ready_limit: i32,
    #[serde(skip_serializing_if = "is_zero")]
    pub memory_limit: i32,
}

fn is_false(v: &bool) -> bool {
    !v
}
fn is_zero(v: &i32) -> bool {
    *v == 0
}

fn present_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

pub(crate) fn null_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

pub(crate) mod go_time {
    use chrono::{DateTime, FixedOffset, SecondsFormat, TimeZone};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn zero() -> DateTime<FixedOffset> {
        chrono::NaiveDate::from_ymd_opt(1, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .fixed_offset()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DateTime<FixedOffset>, D::Error> {
        Ok(Option::<DateTime<FixedOffset>>::deserialize(deserializer)?.unwrap_or_else(zero))
    }

    pub fn format<Tz: TimeZone>(time: &DateTime<Tz>) -> String {
        let full = time.to_rfc3339_opts(SecondsFormat::Nanos, true);
        let (local, offset) = full.split_at(full.rfind(['Z', '+', '-']).unwrap());
        let trimmed = local.trim_end_matches('0').trim_end_matches('.');
        format!("{trimmed}{offset}")
    }
    pub fn serialize<S: Serializer, Tz: TimeZone>(
        time: &DateTime<Tz>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format(time))
    }
    pub fn serialize_optional<S: Serializer>(
        time: &Option<DateTime<FixedOffset>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match time {
            Some(t) => serializer.serialize_some(&format(t)),
            None => serializer.serialize_none(),
        }
    }
}
