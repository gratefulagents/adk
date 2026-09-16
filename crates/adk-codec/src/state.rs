use crate::CodecError;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
struct Event {
    seq: u64,
    event_id: String,
    #[serde(rename = "type")]
    kind: String,
    payload: Value,
}

#[derive(Clone, Deserialize)]
struct Task {
    id: String,
    status: String,
    priority: i64,
    updated_at: crate::timestamp::GoTimestamp,
    #[serde(default)]
    assignee: String,
    #[serde(default)]
    depends_on: Vec<String>,
}

#[derive(Deserialize)]
struct Claim {
    id: String,
    actor: String,
}
#[derive(Deserialize)]
struct Close {
    id: String,
    task: Task,
}

pub(crate) fn ready(input: &Value) -> Result<Value, CodecError> {
    let events: Vec<Event> = serde_json::from_value(input.clone())?;
    let mut tasks: HashMap<String, Task> = HashMap::new();
    let mut ids = HashSet::new();
    let invalid = |message: &str| CodecError::State(message.into());
    for (index, event) in events.into_iter().enumerate() {
        if event.seq != index as u64 + 1 || !ids.insert(event.event_id) {
            return Err(invalid("out-of-order or duplicate state event"));
        }
        match event.kind.as_str() {
            "project.initialized" => {}
            "task.created" => {
                let task: Task = serde_json::from_value(event.payload)?;
                if tasks.contains_key(&task.id) {
                    return Err(invalid("duplicate task"));
                }
                if task.depends_on.iter().any(|id| !tasks.contains_key(id)) {
                    return Err(invalid("unknown dependency"));
                }
                tasks.insert(task.id.clone(), task);
            }
            "task.claimed" => {
                let claim: Claim = serde_json::from_value(event.payload)?;
                let task = tasks
                    .get_mut(&claim.id)
                    .ok_or_else(|| invalid("unknown claimed task"))?;
                task.status = "in_progress".into();
                task.assignee = claim.actor;
            }
            "task.closed" => {
                let close: Close = serde_json::from_value(event.payload)?;
                if !tasks.contains_key(&close.id) {
                    return Err(invalid("unknown closed task"));
                }
                if close.id != close.task.id {
                    return Err(invalid("closed task ID mismatch"));
                }
                tasks.insert(close.id, close.task);
            }
            _ => return Err(invalid("unsupported state event")),
        }
    }
    for task in tasks.values() {
        if task.depends_on.iter().any(|id| !tasks.contains_key(id)) {
            return Err(invalid("unknown dependency"));
        }
    }
    let mut ready: Vec<_> = tasks
        .values()
        .filter(|task| {
            task.status == "open"
                && task.assignee.is_empty()
                && task
                    .depends_on
                    .iter()
                    .all(|id| tasks[id].status == "closed")
        })
        .collect();
    // The fixture protocol normalizes state timestamps to a single UTC date.
    let timestamp_key = |task: &Task| -> Result<(String, u32), CodecError> {
        let stamp = task.updated_at.as_str();
        if !stamp.ends_with('Z') {
            return Err(invalid("state baseline requires UTC timestamps"));
        }
        let fraction = if stamp.len() > 20 {
            &stamp[20..stamp.len() - 1]
        } else {
            ""
        };
        let nanos = format!("{fraction:0<9}")
            .parse::<u32>()
            .map_err(|_| invalid("invalid timestamp"))?;
        Ok((stamp[..19].to_owned(), nanos))
    };
    let mut keys = HashMap::new();
    for task in &ready {
        keys.insert(&task.id, timestamp_key(task)?);
    }
    // Go projectstate.sortTasks: priority ascending, UpdatedAt descending, ID ascending.
    ready.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| keys[&b.id].cmp(&keys[&a.id]))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(serde_json::to_value(
        ready.iter().map(|t| &t.id).collect::<Vec<_>>(),
    )?)
}
