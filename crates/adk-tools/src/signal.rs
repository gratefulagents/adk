//! Stateless SDK signals. Pausing and user interaction remain runner/host behavior.
use crate::{Capability, json_text};
use adk_core::{
    BoxFuture, Content, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

pub trait FinishSink: Send + Sync {
    fn finish<'a>(
        &'a self,
        context: &'a adk_core::Context,
        summary: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>>;
}

pub fn finish(sink: Arc<dyn FinishSink>) -> Arc<dyn Tool> {
    let capability = crate::capabilities()
        .iter()
        .find(|c| c.name == "finish")
        .expect("finish capability");
    Arc::new(Signal {
        definition: capability.definition.clone().expect("finish definition"),
        control_flow: true,
        sink: Some(sink),
    })
}

pub(crate) fn builtin(capability: &Capability) -> Option<Arc<dyn Tool>> {
    match capability.name.as_str() {
        "think" | "AskUserQuestion" | "present_plan" | "finish" => Some(Arc::new(Signal {
            definition: capability
                .definition
                .clone()
                .expect("static signal definition"),
            control_flow: capability.control_flow,
            sink: None,
        })),
        _ => None,
    }
}

struct Signal {
    definition: ToolDefinition,
    control_flow: bool,
    sink: Option<Arc<dyn FinishSink>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Thought {
    thought: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Question {
    question: String,
    choices: Vec<String>,
    allow_freeform: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Summary {
    summary: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Plan {
    summary: String,
    actions: Vec<Action>,
    recommended: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Action {
    id: String,
    label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    style: String,
}

fn normalize(value: &mut Value) {
    if value.is_null() {
        *value = json!({});
    }
    if let Value::Object(fields) = value {
        fields.retain(|_, value| !value.is_null());
        if let Some(Value::Array(choices)) = fields.get_mut("choices") {
            for choice in choices.iter_mut().filter(|v| v.is_null()) {
                *choice = json!("");
            }
        }
        if let Some(Value::Array(actions)) = fields.get_mut("actions") {
            for action in actions {
                normalize(action);
            }
        }
    }
}

fn output(text: String, is_error: bool, should_pause: bool) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text }],
        is_error,
        should_pause,
    }
}

impl Signal {
    fn invoke(&self, mut input: Value) -> Result<ToolOutput, serde_json::Error> {
        normalize(&mut input);
        let result = match self.definition.name.as_str() {
            "think" => {
                let input: Thought = serde_json::from_value(input)?;
                if input.thought.trim().is_empty() {
                    output("thought is required".into(), true, false)
                } else {
                    output("(thought recorded)".into(), false, false)
                }
            }
            "AskUserQuestion" => {
                let input: Question = serde_json::from_value(input)?;
                let text = if input.choices.is_empty() {
                    input.question
                } else {
                    // A struct retains the SDK's field order in deterministic text results.
                    #[derive(Serialize)]
                    struct Answer {
                        question: String,
                        choices: Vec<String>,
                        allow_freeform: bool,
                    }
                    json_text(&Answer {
                        question: input.question,
                        choices: input.choices,
                        allow_freeform: input.allow_freeform.unwrap_or(true),
                    })?
                };
                output(text, false, false)
            }
            "present_plan" => {
                let input: Plan = serde_json::from_value(input)?;
                if input.summary.is_empty() {
                    output("summary is required".into(), true, false)
                } else if input.actions.is_empty() {
                    output("at least one action is required".into(), true, false)
                } else {
                    output(
                        format!("Plan presented to user.\n{}", json_text(&input)?),
                        false,
                        false,
                    )
                }
            }
            "finish" => {
                let input: Summary = serde_json::from_value(input)?;
                output(
                    format!("Run completed.\n\nSummary: {}", input.summary),
                    false,
                    true,
                )
            }
            _ => unreachable!("only stateless signals are constructed"),
        };
        Ok(result)
    }
}

impl Tool for Signal {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn is_control_flow(&self) -> bool {
        self.control_flow
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let mut input = call.arguments;
            normalize(&mut input);
            if let Some(sink) = &self.sink {
                let summary: Summary = match serde_json::from_value(input.clone()) {
                    Ok(summary) => summary,
                    Err(error) => {
                        return Ok(output(format!("Invalid input: {error}"), true, false));
                    }
                };
                let finished = sink.finish(&context.operation, &summary.summary).await;
                context.operation.check_active()?;
                if let Err(error) = finished {
                    return Ok(output(
                        format!("Failed to mark run as completed: {error}"),
                        true,
                        false,
                    ));
                }
            }
            Ok(self
                .invoke(input)
                .unwrap_or_else(|error| output(format!("Invalid input: {error}"), true, false)))
        })
    }
}
