use std::{panic::AssertUnwindSafe, sync::Arc};

use adk_core::{
    BoxFuture, Content, Context, Error, ErrorCategory, GuardrailPhase, GuardrailReport, RunItem,
    ToolCall, ToolOutput,
};
use futures_util::FutureExt;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct GuardrailResult {
    pub output: Value,
    pub tripwire_triggered: bool,
    pub replacement_content: Option<String>,
}

#[derive(Clone, Copy)]
pub enum GuardrailInput<'a> {
    Input(&'a [RunItem]),
    Output(&'a Value),
    ToolInput(&'a ToolCall),
    ToolOutput {
        call: &'a ToolCall,
        output: &'a ToolOutput,
    },
}

impl GuardrailInput<'_> {
    fn phase(self) -> GuardrailPhase {
        match self {
            Self::Input(_) => GuardrailPhase::Input,
            Self::Output(_) => GuardrailPhase::Output,
            Self::ToolInput(_) => GuardrailPhase::ToolInput,
            Self::ToolOutput { .. } => GuardrailPhase::ToolOutput,
        }
    }
    fn tool_name(self) -> Option<String> {
        match self {
            Self::ToolInput(call) | Self::ToolOutput { call, .. } => Some(call.name.clone()),
            _ => None,
        }
    }
}

pub trait Guardrail: Send + Sync {
    fn name(&self) -> &str;
    fn check<'a>(
        &'a self,
        context: &'a Context,
        agent: &'a str,
        input: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>>;
    /// A stable policy identity promises deterministic checks across crash recovery.
    fn durable_key(&self) -> Option<&str> {
        None
    }
}

#[derive(Debug)]
pub struct GuardrailTripwire {
    pub phase: GuardrailPhase,
    pub name: String,
    pub tool_name: Option<String>,
    pub output: Value,
}

impl std::fmt::Display for GuardrailTripwire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} guardrail {:?} triggered", self.phase, self.name)
    }
}
impl std::error::Error for GuardrailTripwire {}

pub struct GuardrailOutcome {
    pub reports: Vec<GuardrailReport>,
    pub error: Option<Error>,
}

impl GuardrailOutcome {
    pub fn tripped(&self) -> bool {
        self.error
            .as_ref()
            .and_then(|e| e.source.as_ref())
            .is_some_and(|e| e.is::<GuardrailTripwire>())
    }
}

async fn check_one(
    guard: &dyn Guardrail,
    context: &Context,
    agent: &str,
    input: GuardrailInput<'_>,
) -> Result<(GuardrailReport, Option<String>), Error> {
    context.check_active()?;
    let phase = input.phase();
    let tool_name = input.tool_name();
    // Include callback construction and polling in the unwind boundary.
    let checked =
        AssertUnwindSafe(async { guard.check(context, agent, input).await }).catch_unwind();
    let result = tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => return Err(Error::new(ErrorCategory::Cancelled, "guardrail cancelled")),
        _ = async { match context.deadline { Some(d) => tokio::time::sleep_until(d.into()).await, None => std::future::pending().await } } => return Err(Error::new(ErrorCategory::DeadlineExceeded, "guardrail deadline exceeded")),
        result = checked => result.map_err(|_| Error::new(ErrorCategory::Guardrail, format!("{phase:?} guardrail {:?} panicked", guard.name())))?,
    };
    let result = result
        .map_err(|error| {
            Error::new(
                ErrorCategory::Guardrail,
                format!("{phase:?} guardrail {:?} failed", guard.name()),
            )
            .with_source(error)
        })?
        .unwrap_or_default();
    Ok((
        GuardrailReport {
            phase,
            guardrail_name: guard.name().into(),
            tool_name,
            output: result.output,
            tripwire_triggered: result.tripwire_triggered,
        },
        result.replacement_content,
    ))
}

fn tripwire(report: &GuardrailReport) -> Option<Error> {
    report.tripwire_triggered.then(|| {
        let cause = GuardrailTripwire {
            phase: report.phase,
            name: report.guardrail_name.clone(),
            tool_name: report.tool_name.clone(),
            output: report.output.clone(),
        };
        Error::new(ErrorCategory::Guardrail, cause.to_string()).with_source(cause)
    })
}

pub async fn run_guardrails(
    guards: &[Arc<dyn Guardrail>],
    context: &Context,
    agent: &str,
    input: GuardrailInput<'_>,
) -> GuardrailOutcome {
    let mut reports = Vec::new();
    for guard in guards {
        let report = match check_one(guard.as_ref(), context, agent, input).await {
            Ok((report, _)) => report,
            Err(error) => {
                return GuardrailOutcome {
                    reports: vec![],
                    error: Some(error),
                };
            }
        };
        let error = tripwire(&report);
        reports.push(report);
        if error.is_some() {
            return GuardrailOutcome { reports, error };
        }
    }
    GuardrailOutcome {
        reports,
        error: None,
    }
}

pub async fn run_tool_output_guardrails(
    guards: &[Arc<dyn Guardrail>],
    context: &Context,
    agent: &str,
    call: &ToolCall,
    output: &mut ToolOutput,
) -> GuardrailOutcome {
    let mut reports = Vec::new();
    for guard in guards {
        let (report, replacement) = match check_one(
            guard.as_ref(),
            context,
            agent,
            GuardrailInput::ToolOutput { call, output },
        )
        .await
        {
            Ok(report) => report,
            Err(error) => {
                return GuardrailOutcome {
                    reports: vec![],
                    error: Some(error),
                };
            }
        };
        let error = tripwire(&report);
        reports.push(report);
        if error.is_some() {
            return GuardrailOutcome { reports, error };
        }
        if let Some(text) = replacement {
            output.content = vec![Content::Text { text }];
        }
    }
    GuardrailOutcome {
        reports,
        error: None,
    }
}
