//! An explicitly owned, read-only critic run used as an advisory final-answer verifier.
use std::{num::NonZeroU32, sync::Arc};

use adk_core::*;
use serde_json::Value;

use crate::{AgentConfig, FinalAnswerVerifier, Runner, RunnerConfig};

pub const DEFAULT_CRITIC_INSTRUCTIONS: &str = r#"You are an independent reviewer. Another agent claims to have completed a task; your job is to try to REFUTE that claim using read-only inspection.

Actively look for: requirements that were not satisfied, files or artifacts that should exist but do not (or have wrong content), claims in the answer contradicted by the actual state of the workspace, tests/checks that were never run, and unhandled edge cases the task implies.

Verify evidence with your tools instead of trusting the answer's claims. Be precise and cite file paths or command output for every problem you report.

End your reply with exactly one verdict line:
VERDICT: APPROVED — only if you could not find any substantive problem.
VERDICT: REJECTED — followed by a numbered list of concrete, actionable problems.
Do not reject for style, phrasing, or hypothetical concerns you did not verify."#;

const INCONCLUSIVE: &str = "The independent verifier did not return a usable verdict. Re-check your answer against the original task requirements, verify your claims, and provide your final answer again.";

const CRITIC_TURNS: NonZeroU32 = NonZeroU32::new(12).unwrap();

pub struct CriticVerifier {
    runner: Runner,
    host: Arc<dyn Host>,
    original_task: String,
}

impl CriticVerifier {
    pub fn new(
        mut critic: AgentConfig,
        original_task: impl Into<String>,
        host: Arc<dyn Host>,
    ) -> Result<Self, Error> {
        if critic.instructions.trim().is_empty() {
            critic.instructions = DEFAULT_CRITIC_INSTRUCTIONS.into();
        }
        Ok(Self {
            runner: Runner::new(
                critic,
                RunnerConfig {
                    force_final_summary_turn: true,
                    subagent_max_turns: Some(CRITIC_TURNS),
                    ..Default::default()
                },
            )?,
            host,
            original_task: original_task.into(),
        })
    }
}

impl FinalAnswerVerifier for CriticVerifier {
    fn verify<'a>(
        &'a self,
        context: &'a Context,
        output: &'a Value,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            let candidate = match output {
                Value::Null => String::new(),
                Value::String(text) => text.clone(),
                _ => String::from_utf8(
                    adk_codec::snapshots::to_go_json(output).expect("JSON output"),
                )
                .expect("JSON is UTF-8"),
            };
            let prompt = format!(
                "<original_task>\n{}\n</original_task>\n\n<candidate_final_answer>\n{}\n</candidate_final_answer>\n\nReview the candidate final answer against the original task. Verify its claims with your tools, then give your verdict.",
                self.original_task, candidate
            );
            let outcome = self
                .runner
                .run(
                    context.clone(),
                    RunRequest {
                        input: vec![RunItem::Message {
                            message: Message {
                                role: Role::User,
                                content: vec![Content::Text { text: prompt }],
                            },
                        }],
                        input_provenance: vec![ItemProvenance::Unattributed],
                        policy: RunPolicy {
                            max_turns: CRITIC_TURNS,
                            tools: ToolPolicy {
                                access: AccessMode::ReadOnly,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    },
                    self.host.clone(),
                )
                .await
                .map_err(|failure| failure.error)?;
            let verdict = outcome.result.final_text();
            let mut found = false;
            let mut approved = false;
            let mut rejected = false;
            for line in verdict.split('\n') {
                let upper = line.trim().to_uppercase();
                let Some(rest) = upper.strip_prefix("VERDICT:") else {
                    continue;
                };
                match rest.trim() {
                    "APPROVED" | "APPROVED." => {
                        found = true;
                        approved = true;
                    }
                    text if text.starts_with("REJECTED") => {
                        found = true;
                        approved = false;
                        rejected = true;
                    }
                    _ => {}
                }
            }
            if approved && !rejected {
                Ok(String::new())
            } else if !found {
                Ok(INCONCLUSIVE.into())
            } else {
                Ok(verdict.into())
            }
        })
    }
}
