//! Public SDK user-input helpers. These inspect records; they do not execute tools.
use crate::dto::{RunItem, RunItemType};
pub(crate) use crate::go_decode::{ElementSeed, GoSlice};
use crate::go_decode::{fold_key, go_object, read_field};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) use crate::go_decode::go_utf8;

/// Decode SDK user-input JSON with Go-compatible string replacement.
#[doc(hidden)]
pub fn decode<T: serde::de::DeserializeOwned>(input: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(&crate::go_decode::normalize_strings(input))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct QuickAction {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub style: String,
}

go_object!(QuickAction, "id" => id: scalar, "label" => label: scalar, "style" => style: scalar);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserInputPause {
    pub requested: bool,
    pub plan_review: bool,
    pub question: String,
    pub actions: Option<Vec<u8>>,
}

/// `None` preserves a nil Go slice (`null`); an empty slice encodes as `[]`.
pub fn marshal_quick_actions(actions: Option<&[QuickAction]>) -> Vec<u8> {
    crate::snapshots::to_go_json(&actions).expect("quick actions contain only strings")
}

pub fn extract_ask_user_choices(input: &[u8]) -> Option<Vec<u8>> {
    #[derive(Default)]
    struct Input {
        choices: GoSlice<String>,
    }
    go_object!(Input, "choices" => choices: slice);
    let input = decode::<Option<Input>>(input).ok()??;
    let choices = input.choices.into_values().ok()?.unwrap_or_default();
    if choices.is_empty() {
        return None;
    }
    let actions: Vec<_> = choices
        .into_iter()
        .enumerate()
        .map(|(i, label)| QuickAction {
            id: format!("choice_{i}"),
            label,
            style: if i == 0 { "primary" } else { "secondary" }.into(),
        })
        .collect();
    Some(marshal_quick_actions(Some(&actions)))
}

pub fn extract_present_plan_data(input: &[u8]) -> (String, Option<Vec<u8>>) {
    #[derive(Default)]
    struct Input {
        summary: String,
        actions: GoSlice<QuickAction>,
        recommended: String,
    }
    go_object!(Input, "summary" => summary: scalar, "actions" => actions: slice, "recommended" => recommended: scalar);
    let Ok(input) = decode::<Option<Input>>(input) else {
        return (String::new(), None);
    };
    let input = input.unwrap_or_default();
    let Ok(actions) = input.actions.into_values() else {
        return (String::new(), None);
    };
    (
        input.summary,
        Some(marshal_quick_actions(actions.as_deref())),
    )
}

pub fn extract_ask_user_question(input: &[u8]) -> String {
    #[derive(Default)]
    struct Choice {
        label: String,
        description: String,
    }
    #[derive(Default)]
    struct Question {
        question: String,
        header: String,
        options: GoSlice<Choice>,
    }
    #[derive(Default)]
    struct Structured {
        questions: GoSlice<Question>,
    }
    #[derive(Default)]
    struct Simple {
        question: String,
    }
    go_object!(Choice, "label" => label: scalar, "description" => description: scalar);
    go_object!(Question, "question" => question: scalar, "header" => header: scalar, "options" => options: slice);
    go_object!(Structured, "questions" => questions: slice);
    go_object!(Simple, "question" => question: scalar);
    if let Ok(Some(value)) = decode::<Option<Structured>>(input) {
        let questions = value
            .questions
            .into_values()
            .ok()
            .flatten()
            .unwrap_or_default();
        if !questions.is_empty() {
            let formatted = questions
                .into_iter()
                .map(|q| -> Result<String, serde_json::Error> {
                    let mut text = q.question;
                    let options = q.options.into_values()?.unwrap_or_default();
                    if !options.is_empty() {
                        text.push_str(" Options: ");
                        text.push_str(
                            &options
                                .into_iter()
                                .map(|o| {
                                    if o.description.is_empty() {
                                        o.label
                                    } else {
                                        format!("{}: {}", o.label, o.description)
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(" | "),
                        );
                    }
                    Ok(text)
                })
                .collect::<Result<Vec<_>, _>>();
            if let Ok(formatted) = formatted {
                return formatted.join("\n");
            }
        }
    }
    if let Ok(Some(value)) = decode::<Option<Simple>>(input) {
        if !value.question.is_empty() {
            return value.question;
        }
    }
    go_utf8(input)
}

pub fn detect_user_input_pause(items: &[RunItem], final_text: &str) -> UserInputPause {
    for item in items {
        if item.kind != RunItemType(1) {
            continue;
        }
        let Some(call) = &item.tool_call else {
            continue;
        };
        let input = match &call.input {
            crate::dto::RawJson::Missing => Vec::new(),
            crate::dto::RawJson::Encoded(raw) => raw.get().as_bytes().to_vec(),
            crate::dto::RawJson::EncodedBytes(bytes) => bytes.clone(),
            crate::dto::RawJson::Present(value) => {
                crate::snapshots::to_go_json(value).expect("JSON value")
            }
        };
        let (mut question, actions, plan_review) = match call.name.as_str() {
            "AskUserQuestion" => (
                extract_ask_user_question(&input),
                extract_ask_user_choices(&input),
                false,
            ),
            "present_plan" => {
                let (question, actions) = extract_present_plan_data(&input);
                (question, actions, true)
            }
            _ => continue,
        };
        let trimmed = question.trim();
        if question.is_empty()
            || (trimmed.starts_with('{') && trimmed.ends_with('}') && !final_text.trim().is_empty())
        {
            question = final_text.trim().into();
        }
        if question.is_empty() {
            question = "The agent needs your input to continue.".into();
        }
        return UserInputPause {
            requested: true,
            plan_review,
            question,
            actions,
        };
    }
    UserInputPause::default()
}

pub fn build_auto_turn_cap_prompt(max_turns: i64) -> String {
    format!("Auto mode global turn cap ({max_turns}) reached.")
}

/// Explicit SDK child-budget guidance; this does not configure or enforce a runner limit.
pub fn build_subagent_budget_context(max_turns: i64) -> String {
    let max_turns = if max_turns > 0 { max_turns } else { 50 };
    format!(
        r#"<sub_agent_budget>
Turn budget: {max_turns} LLM turns for this sub-agent.
A turn is one model response, not one tool call. Tool calls happen inside a turn.
Act, don't announce: when you intend to use tools, include those tool calls in the SAME turn. Never end a turn with only a statement of intent ("I'll start by…", "Let me…"); a turn that contains no tool calls is treated as your final answer and ends the task.
This is a hard ceiling, not a target. Do not try to use the full budget.
Finish as soon as the requested output is evidence-backed enough to be useful.
You will receive a [SYSTEM] turn-budget warning shortly before the cap: treat it as the signal to stop exploring and return your findings immediately.
If the task is broader than the remaining budget, stop exploring and return a concise partial summary with: files checked, concrete findings, gaps/unknowns, and recommended next steps.
</sub_agent_budget>

<result_contract>
Your final message goes back to a coordinating agent, so make it self-contained and concise.
If your deliverable is large (long reports, generated code listings, full logs), write the complete artifact to a file under the working directory and return a short summary plus the file path instead of pasting everything inline.
Always state in your final message: what you did, the key findings/decisions, file paths you created or modified, and anything still unresolved.
</result_contract>"#
    )
}

/// Legacy SDK top-level guidance, retained for explicit callers, not automatic injection.
pub fn build_run_budget_context(max_turns: i64) -> String {
    let max_turns = if max_turns > 0 { max_turns } else { 100 };
    format!(
        r#"<run_budget>
Turn budget: {max_turns} LLM turns for this top-level run.
A turn is one model response, not one tool call. Tool calls happen inside a turn.
This is a hard ceiling, not a target. Do not try to use the full budget.
Finish as soon as the requested outcome is complete and verified.
You will receive a [SYSTEM] turn-budget warning shortly before the budget runs out: when it appears, persist anything that must survive (commit and push work in progress, record durable notes/tasks), then deliver your best final answer from what you already have.
</run_budget>"#
    )
}
