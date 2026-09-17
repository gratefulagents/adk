//! Plan artifacts are host-owned; tools never invent a durable store or session.
use adk_core::{
    BoxFuture, Content, Context, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub trait ArtifactStore: Send + Sync {
    /// Persist the plan under the trusted session identity. Implementations must
    /// atomically replace the plan and record the summary and update timestamp.
    fn save<'a>(
        &'a self,
        context: &'a Context,
        session_id: &'a str,
        plan: &'a str,
        summary: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>>;
    /// A missing artifact is `Ok(None)`, not a store failure.
    fn get<'a>(
        &'a self,
        context: &'a Context,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, Error>>;
}

pub fn tools(store: Arc<dyn ArtifactStore>, session_id: &str) -> Vec<Arc<dyn Tool>> {
    crate::capabilities()
        .iter()
        .filter(|c| matches!(c.name.as_str(), "save_plan" | "get_plan"))
        .map(|c| {
            Arc::new(PlanTool {
                definition: c.definition.clone().expect("plan definition"),
                store: store.clone(),
                session_id: session_id.to_owned(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

struct PlanTool {
    definition: ToolDefinition,
    store: Arc<dyn ArtifactStore>,
    session_id: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    plan: String,
    summary: String,
}

fn result(text: String, is_error: bool) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text }],
        is_error,
        should_pause: false,
    }
}

impl Tool for PlanTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn is_control_flow(&self) -> bool {
        true
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            if self.definition.name == "get_plan" {
                let stored = self.store.get(&context.operation, &self.session_id).await;
                context.operation.check_active()?;
                return Ok(match stored {
                    Ok(Some(plan)) if !plan.is_empty() => result(plan, false),
                    Ok(_) => result(
                        "No plan found. Use save_plan to create one first.".into(),
                        false,
                    ),
                    Err(error) => result(format!("Failed to read plan: {error}"), true),
                });
            }
            let mut input = call.arguments;
            if input.is_null() {
                input = json!({});
            }
            if let Value::Object(fields) = &mut input {
                fields.retain(|_, value| !value.is_null());
            }
            let input: Input = match serde_json::from_value(input) {
                Ok(input) => input,
                Err(error) => return Ok(result(format!("Invalid input: {error}"), true)),
            };
            if input.plan.is_empty() {
                return Ok(result("plan content is required".into(), true));
            }
            let summary = if !input.summary.is_empty() {
                input.summary
            } else if input.plan.len() > 200 {
                // encoding/json replaces each invalid byte in Go's byte-sliced summary.
                let mut end = 200;
                while !input.plan.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}{}...", &input.plan[..end], "�".repeat(200 - end))
            } else {
                input.plan.clone()
            };
            let saved = self
                .store
                .save(&context.operation, &self.session_id, &input.plan, &summary)
                .await;
            context.operation.check_active()?;
            Ok(match saved {
                Ok(()) => result(
                    format!("Plan saved ({} bytes) to artifact store", input.plan.len()),
                    false,
                ),
                Err(error) => result(format!("Failed to persist plan: {error}"), true),
            })
        })
    }
}
