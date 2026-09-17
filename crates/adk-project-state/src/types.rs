use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    pub seq: i64,
    pub event_id: String,
    pub project_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub run_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub actor: String,
    #[serde(serialize_with = "go_time::serialize")]
    pub time: DateTime<Utc>,
    #[serde(rename = "type")]
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
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(rename = "type")]
    pub task_type: String,
    pub status: String,
    pub priority: i32,
    #[serde(skip_serializing_if = "String::is_empty")]
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
    #[serde(serialize_with = "go_time::serialize")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "go_time::serialize")]
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "go_time::serialize_optional")]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_run: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskComment {
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub actor: String,
    pub body: String,
    #[serde(serialize_with = "go_time::serialize")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub id: String,
    pub kind: String,
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
    #[serde(serialize_with = "go_time::serialize")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "go_time::serialize")]
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "go_time::serialize_optional")]
    pub last_read_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSummary {
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub run_id: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_vec")]
    pub task_ids: Vec<String>,
    #[serde(serialize_with = "go_time::serialize")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "go_time::serialize")]
    pub updated_at: DateTime<Utc>,
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

pub(crate) fn null_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

pub(crate) mod go_time {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::Serializer;
    pub fn format(time: &DateTime<Utc>) -> String {
        let full = time.to_rfc3339_opts(SecondsFormat::Nanos, true);
        let trimmed = full
            .trim_end_matches('Z')
            .trim_end_matches('0')
            .trim_end_matches('.');
        format!("{trimmed}Z")
    }
    pub fn serialize<S: Serializer>(
        time: &DateTime<Utc>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format(time))
    }
    pub fn serialize_optional<S: Serializer>(
        time: &Option<DateTime<Utc>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match time {
            Some(t) => serializer.serialize_some(&format(t)),
            None => serializer.serialize_none(),
        }
    }
}
