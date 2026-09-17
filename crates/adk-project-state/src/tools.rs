//! The baseline SDK's 15 project-state tools, independent of the full tool registry.
use crate::*;
use adk_core::{BoxFuture, Content, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

pub fn tools(store: Arc<dyn Store>, actor: &str) -> Vec<Arc<dyn Tool>> {
    let definitions: Vec<ToolDefinition> =
        serde_json::from_str(include_str!("tool-definitions.json"))
            .expect("static tool definitions");
    definitions
        .into_iter()
        .map(|definition| {
            Arc::new(StateTool {
                definition,
                store: store.clone(),
                actor: actor.trim().into(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

struct StateTool {
    definition: ToolDefinition,
    store: Arc<dyn Store>,
    actor: String,
}

impl StateTool {
    fn invoke(&self, input: Value) -> Result<String> {
        let text = |key: &str| -> Result<String> {
            match input.get(key) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(s)) => Ok(s.clone()),
                _ => Err(Error::Invalid(format!(
                    "Invalid input: {key} must be a string"
                ))),
            }
        };
        let actor = || -> Result<String> {
            let explicit = text("actor")?;
            Ok(if explicit.trim().is_empty() {
                self.actor.clone()
            } else {
                explicit.trim().into()
            })
        };
        let value = match self.definition.name.as_str() {
            "task_create" => {
                let mut value = input.clone();
                if input.get("priority").is_none_or(Value::is_null) {
                    value["priority"] = json!(2);
                }
                let mut data: CreateTaskInput = serde_json::from_value(value)?;
                data.source_run.clear();
                data.metadata = Value::Null;
                serde_json::to_value(self.store.create_task(data)?)?
            }
            "task_ready" => {
                let mut filter: TaskFilter = serde_json::from_value(input.clone())?;
                filter.actor = self.actor.clone();
                serde_json::to_value(self.store.ready_tasks(filter)?)?
            }
            "task_show" => serde_json::to_value(self.store.get_task(&text("id")?)?)?,
            "task_update" => {
                let mut patch: TaskPatch = serde_json::from_value(input.clone())?;
                patch.metadata = None;
                patch.replace_labels = input.get("labels").is_some_and(|v| !v.is_null());
                serde_json::to_value(self.store.update_task(&text("id")?, patch)?)?
            }
            "task_claim" => serde_json::to_value(self.store.claim_task(&text("id")?, &actor()?)?)?,
            "task_close" => {
                serde_json::to_value(self.store.close_task(&text("id")?, &text("reason")?)?)?
            }
            "task_comment" => serde_json::to_value(self.store.add_comment(
                &text("id")?,
                &actor()?,
                &text("body")?,
            )?)?,
            "task_link" => {
                let id = text("id")?;
                let dependency = text("depends_on")?;
                let action = text("action")?;
                if action.eq_ignore_ascii_case("remove") {
                    self.store.remove_dependency(&id, &dependency)?;
                } else {
                    self.store.add_dependency(&id, &dependency)?;
                }
                json!({"id":id,"depends_on":dependency,"action":if action.trim().is_empty(){"add"}else{action.trim()}})
            }
            "memory_remember" => serde_json::to_value(
                self.store
                    .upsert_memory(serde_json::from_value(input.clone())?)?,
            )?,
            "memory_list" | "memory_recall" | "memory_stats" => {
                let filter = serde_json::from_value(input.clone())?;
                let memories = if self.definition.name == "memory_recall" {
                    self.store.search_memories(filter)?
                } else {
                    self.store.list_memories(filter)?
                };
                if self.definition.name == "memory_stats" {
                    let mut kinds = BTreeMap::<String, usize>::new();
                    let mut scopes = BTreeMap::<String, usize>::new();
                    let mut tags = BTreeMap::<String, usize>::new();
                    for memory in &memories {
                        *kinds.entry(memory.kind.clone()).or_default() += 1;
                        *scopes.entry(memory.scope.clone()).or_default() += 1;
                        for tag in &memory.tags {
                            if !tag.trim().is_empty() {
                                *tags.entry(tag.trim().into()).or_default() += 1;
                            }
                        }
                    }
                    json!({"total":memories.len(),"by_kind":kinds,"by_scope":scopes,"by_tag":tags})
                } else {
                    serde_json::to_value(memories)?
                }
            }
            "memory_update" => {
                let id = text("id")?;
                let memory = self
                    .store
                    .list_memories(MemoryFilter::default())?
                    .into_iter()
                    .find(|m| m.id == id.trim())
                    .ok_or_else(|| Error::NotFound(format!("memory {id:?}")))?;
                let mut value = serde_json::to_value(memory)?;
                for key in [
                    "content",
                    "kind",
                    "scope",
                    "tags",
                    "task_ids",
                    "file_paths",
                    "source_run",
                ] {
                    if let Some(field) = input.get(key).filter(|v| !v.is_null()) {
                        value[key] = field.clone();
                    }
                }
                serde_json::to_value(self.store.upsert_memory(serde_json::from_value(value)?)?)?
            }
            "memory_delete" => {
                let id = text("id")?;
                self.store.delete_memory(&id)?;
                json!({"id":id.trim(),"status":"deleted"})
            }
            "prime_context" => {
                let mut opts: PrimeOptions = serde_json::from_value(input.clone())?;
                opts.actor = actor()?;
                return self.store.prime_context(opts);
            }
            _ => unreachable!("static tool definitions"),
        };
        Ok(serde_json::to_string_pretty(&value)?)
    }
}

impl Tool for StateTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, std::result::Result<ToolOutput, adk_core::Error>> {
        Box::pin(async move {
            let (text, is_error) = match self.invoke(call.arguments) {
                Ok(text) => (text, false),
                Err(error) => (error.to_string(), true),
            };
            Ok(ToolOutput {
                content: vec![Content::Text { text }],
                is_error,
                should_pause: false,
            })
        })
    }
}
