//! Host-injected namespace memory, distinct from project-state memory tools.
use adk_core::{
    BoxFuture, Content, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use adk_project_state::memory::Store;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub fn tool(
    store: Arc<dyn Store>,
    namespace: &str,
    source_run: &str,
    repo_url: &str,
) -> Arc<dyn Tool> {
    let definition = crate::capabilities()
        .iter()
        .find(|c| c.name == "Memory")
        .and_then(|c| c.definition.clone())
        .expect("Memory definition");
    Arc::new(MemoryTool {
        definition,
        store,
        namespace: namespace.into(),
        source_run: source_run.into(),
        repo_url: repo_url.into(),
    })
}

struct MemoryTool {
    definition: ToolDefinition,
    store: Arc<dyn Store>,
    namespace: String,
    source_run: String,
    repo_url: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    action: String,
    content: String,
    tags: Vec<String>,
    id: String,
    limit: i32,
}

fn result(text: String, is_error: bool) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text }],
        is_error,
        should_pause: false,
    }
}

impl MemoryTool {
    fn invoke(&self, mut input: Value) -> ToolOutput {
        if input.is_null() {
            input = json!({});
        }
        if let Value::Object(fields) = &mut input {
            fields.retain(|_, value| !value.is_null());
            if let Some(Value::Array(tags)) = fields.get_mut("tags") {
                for tag in tags.iter_mut().filter(|v| v.is_null()) {
                    *tag = json!("");
                }
            }
        }
        let input: Input = match serde_json::from_value(input) {
            Ok(input) => input,
            Err(error) => return result(format!("Invalid input: {error}"), true),
        };
        let outcome = match input.action.as_str() {
            "store" => {
                if input.content.is_empty() {
                    return result("content is required for store action".into(), true);
                }
                let metadata = if self.repo_url.is_empty() {
                    json!({})
                } else {
                    json!({"repo": self.repo_url})
                };
                self.store
                    .store(
                        &self.namespace,
                        &input.content,
                        &input.tags,
                        &self.source_run,
                        metadata,
                    )
                    .map(|memory| crate::json_text(&memory).expect("memory JSON"))
                    .map_err(|error| format!("Failed to store memory: {error}"))
            }
            "search" | "list" => {
                if input.action == "search" && input.content.is_empty() {
                    return result(
                        "content is required for search action (used as query)".into(),
                        true,
                    );
                }
                let search = input.action == "search";
                let memories = if search {
                    self.store
                        .search(&self.namespace, &input.content, &input.tags, input.limit)
                } else {
                    self.store.list(&self.namespace, &input.tags, input.limit)
                };
                memories
                    .map(|memories| {
                        if memories.is_empty() {
                            if search {
                                "No matching memories found.".into()
                            } else {
                                "No memories found.".into()
                            }
                        } else {
                            crate::json_text(&memories).expect("memory list JSON")
                        }
                    })
                    .map_err(|error| format!("Failed to {} memories: {error}", input.action))
            }
            "delete" => {
                if input.id.is_empty() {
                    return result("id is required for delete action".into(), true);
                }
                let id = match Uuid::parse_str(input.id.trim()) {
                    Ok(id) => id,
                    Err(error) => return result(format!("Invalid memory ID: {error}"), true),
                };
                self.store
                    .delete(&self.namespace, id)
                    .map(|()| format!("Memory {id} deleted."))
                    .map_err(|error| format!("Failed to delete memory: {error}"))
            }
            _ => {
                return result(
                    format!(
                        "Unknown action: {:?}. Use store, search, list, or delete.",
                        input.action
                    ),
                    true,
                );
            }
        };
        match outcome {
            Ok(text) => result(text, false),
            Err(error) => result(error, true),
        }
    }
}

impl Tool for MemoryTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            Ok(self.invoke(call.arguments))
        })
    }
}
