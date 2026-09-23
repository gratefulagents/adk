//! Explicit Go compatibility adapters. Native hooks remain awaited and fail-closed.
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};

use adk_codec::{
    approval::{self, ApprovalMarker, ApprovalMarkerBoundary, ApprovalPhase, BridgeError},
    config::{ConfigError, EffectiveRunConfig, RunConfigSentinels},
    dto,
};
use adk_core::*;
use serde_json::Value;

use crate::{Observation, RunHooks, RunOutcome, Runner, RunnerConfig};

/// Infallible, synchronous observers, not authorization or security hooks.
/// Panics in these callbacks are isolated by [`GoCallbackAdapter`].
pub trait GoLifecycleCallbacks: Send + Sync {
    fn on_agent_start(&self, _context: &Context, _agent: &str) {}
    fn on_model_start(&self, _context: &Context, _agent: &str, _model: &str, _attempt: u32) {}
    fn on_model_end(&self, _context: &Context, _agent: &str, _response: &ModelResponse) {}
    fn on_tool_start(&self, _context: &Context, _agent: &str, _call: &ToolCall) {}
    fn on_tool_end(&self, _context: &Context, _call: &ToolCall, _output: &ToolOutput) {}
    fn on_agent_end(&self, _context: &Context, _agent: &str, _output: &Value) {}
    fn on_handoff(&self, _context: &Context, _from: &str, _to: &str) {}
}

pub struct GoCallbackAdapter {
    callbacks: Arc<dyn GoLifecycleCallbacks>,
}

impl GoCallbackAdapter {
    pub fn new(callbacks: Arc<dyn GoLifecycleCallbacks>) -> Self {
        Self { callbacks }
    }
}

impl RunHooks for GoCallbackAdapter {
    fn durable_observer(&self) -> bool {
        true
    }
    fn observe<'a>(
        &'a self,
        context: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let _ = catch_unwind(AssertUnwindSafe(|| match &observation {
                Observation::AgentStarted { agent, .. } => {
                    self.callbacks.on_agent_start(context, agent)
                }
                Observation::ModelAttempt {
                    agent,
                    model,
                    attempt,
                } => self
                    .callbacks
                    .on_model_start(context, agent, model, *attempt),
                Observation::ModelAccepted { agent, response } => {
                    self.callbacks.on_model_end(context, agent, response)
                }
                Observation::ToolStarted { agent, call } => {
                    self.callbacks.on_tool_start(context, agent, call)
                }
                Observation::RawToolOutput { call, output } => {
                    self.callbacks.on_tool_end(context, call, output)
                }
                Observation::AgentEnded { agent, output } => {
                    self.callbacks.on_agent_end(context, agent, output)
                }
                Observation::Handoff { from, to } => self.callbacks.on_handoff(context, from, to),
                _ => {}
            }));
            Ok(())
        })
    }
}

/// Applies representable scalar settings atomically, retaining native authorization.
/// Subagent, consecutive-error and stop-gate limits in the returned value remain
/// the caller's responsibility. Mutation-only approval never relaxes authorization.
pub fn apply_go_config(
    sentinels: &RunConfigSentinels,
    config: &mut RunnerConfig,
    policy: &mut RunPolicy,
) -> Result<EffectiveRunConfig, ConfigError> {
    let effective = sentinels.resolve()?;
    config.approve_mutating_tools = effective
        .tool_policy
        .as_ref()
        .is_some_and(|p| p.approval_required);
    config.consecutive_tool_error_limit = effective.consecutive_tool_error_limit;
    config.stop_gate_max_blocks = effective.stop_gate_max_blocks;
    policy.max_turns = effective.max_turns;
    config.output.max_bytes = effective.max_tool_output_bytes;
    config.output.untrusted = effective.untrusted_tool_outputs;
    config.model_idle_timeout = effective.model_idle_timeout;
    if let Some(timeout) = effective
        .tool_policy
        .as_ref()
        .and_then(|p| p.default_timeout)
    {
        policy.tools.timeout = Some(timeout);
    }
    Ok(effective)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ApprovalJournalEntry {
    pub marker: ApprovalMarker,
    pub new_items_before: usize,
    /// None when compaction removed the corresponding call and output.
    pub history_before: Option<usize>,
    pub reason: Option<String>,
}

/// Wire items plus lossless sidecars: Go Approved=false alone cannot distinguish
/// pending from denied, and the Go marker does not contain a reason field.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedApprovalHistory {
    pub items: Vec<dto::RunItem>,
    pub phases: Vec<ApprovalPhase>,
    pub reasons: Vec<Option<String>>,
}

#[derive(Default)]
struct JournalState {
    entries: Vec<ApprovalJournalEntry>,
    history_invalidated: bool,
}

/// One journal per run (including its continuations). Explicit provenance is
/// required on encoding; an agent cannot be inferred from native items.
#[derive(Default)]
pub struct ApprovalJournal {
    state: Mutex<JournalState>,
}

impl ApprovalJournal {
    pub(crate) fn restore(
        entries: Vec<ApprovalJournalEntry>,
        history_len: usize,
        new_items_len: usize,
    ) -> Result<Self, BridgeError> {
        for entry in &entries {
            entry.marker.validate()?;
            if entry.history_before.is_some_and(|n| n > history_len)
                || entry.new_items_before > new_items_len
            {
                return Err(BridgeError("approval journal anchor out of range"));
            }
        }
        Ok(Self {
            state: Mutex::new(JournalState {
                entries,
                history_invalidated: false,
            }),
        })
    }

    pub fn history_markers(&self) -> Result<Vec<ApprovalMarkerBoundary>, BridgeError> {
        let state = self.state.lock().unwrap();
        if state.history_invalidated {
            return Err(BridgeError(
                "approval history anchors invalidated by history replacement",
            ));
        }
        let mut markers: Vec<_> = state
            .entries
            .iter()
            .filter_map(|entry| {
                entry
                    .history_before
                    .map(|before_item| ApprovalMarkerBoundary {
                        before_item,
                        marker: entry.marker.clone(),
                    })
            })
            .collect();
        markers.sort_by_key(|entry| entry.before_item);
        Ok(markers)
    }
    pub fn entries(&self) -> Vec<ApprovalJournalEntry> {
        self.state.lock().unwrap().entries.clone()
    }

    pub fn encode_new_items(
        &self,
        items: &[RunItem],
        agents: &[Option<dto::AgentRef>],
    ) -> Result<EncodedApprovalHistory, BridgeError> {
        self.encode(items, agents, false)
    }

    /// Compaction rebases surviving anchors and prunes markers for removed calls.
    /// Ambiguous replacements return an error rather than encoding stale positions.
    pub fn encode_history(
        &self,
        items: &[RunItem],
        agents: &[Option<dto::AgentRef>],
    ) -> Result<EncodedApprovalHistory, BridgeError> {
        self.encode(items, agents, true)
    }

    fn encode(
        &self,
        items: &[RunItem],
        agents: &[Option<dto::AgentRef>],
        history: bool,
    ) -> Result<EncodedApprovalHistory, BridgeError> {
        let state = self.state.lock().unwrap();
        if history && state.history_invalidated {
            return Err(BridgeError(
                "approval history anchors invalidated by history replacement",
            ));
        }
        let mut entries: Vec<_> = state
            .entries
            .iter()
            .filter(|entry| !history || entry.history_before.is_some())
            .collect();
        let position = |entry: &&ApprovalJournalEntry| {
            if history {
                entry.history_before.unwrap()
            } else {
                entry.new_items_before
            }
        };
        entries.sort_by_key(position);
        let markers: Vec<_> = entries
            .iter()
            .map(|entry| ApprovalMarkerBoundary {
                before_item: position(entry),
                marker: entry.marker.clone(),
            })
            .collect();
        Ok(EncodedApprovalHistory {
            items: approval::encode_history(items, agents, &markers)?,
            phases: entries.iter().map(|entry| entry.marker.phase).collect(),
            reasons: entries.iter().map(|entry| entry.reason.clone()).collect(),
        })
    }
}

impl RunHooks for ApprovalJournal {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            match observation {
                Observation::ApprovalHistoryReplaced { markers, .. } => {
                    let mut used = std::collections::HashSet::new();
                    for entry in &mut state.entries {
                        entry.history_before = None;
                    }
                    for marker in markers {
                        let entry = state.entries.iter_mut().enumerate().find(|(index, entry)| {
                            !used.contains(index) && entry.marker == marker.marker
                        });
                        if let Some((index, entry)) = entry {
                            used.insert(index);
                            entry.history_before = Some(marker.before_item);
                        } else {
                            state.history_invalidated = true;
                            return Err(Error::new(
                                ErrorCategory::Internal,
                                "compaction invented an approval marker",
                            ));
                        }
                    }
                }
                Observation::ApprovalMarker {
                    agent,
                    call,
                    decision,
                    new_items_before,
                    history_before,
                    reason,
                } => {
                    let phase = match decision {
                        ApprovalDecision::Defer => ApprovalPhase::Pending,
                        ApprovalDecision::Approve => ApprovalPhase::Approved,
                        ApprovalDecision::Deny => ApprovalPhase::Denied,
                    };
                    state.entries.push(ApprovalJournalEntry {
                        marker: ApprovalMarker::from_call(
                            &call,
                            phase,
                            agent.map(|name| dto::AgentRef { name }),
                        ),
                        new_items_before,
                        history_before: Some(history_before),
                        reason,
                    });
                }
                Observation::HistoryReplaced { before, after } if before != after => {
                    for entry in &mut state.entries {
                        if rebase_entry(entry, &before, &after).is_err() {
                            state.history_invalidated = true;
                            break;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })
    }
}

fn rebase_entry(
    entry: &mut ApprovalJournalEntry,
    before: &[RunItem],
    after: &[RunItem],
) -> Result<(), BridgeError> {
    let Some(boundary) = entry.history_before else {
        return Ok(());
    };
    if boundary > before.len() {
        return Err(BridgeError("cannot rebase a future approval boundary"));
    }
    let call = approval::approval_call(&entry.marker.data)?;
    if !after.iter().any(|item| match item {
        RunItem::ToolCall { call: surviving } => surviving == &call,
        RunItem::ToolResult { call_id, .. } => call_id == &call.id,
        _ => false,
    }) {
        entry.history_before = None;
        return Ok(());
    }
    let mut correspondence = Vec::new();
    for (old_index, item) in before.iter().enumerate() {
        // Approval-local, call-ID-correlated anchors; repeated/reordered prose
        // is normal in a conversation and is not evidence of ambiguity.
        if matches!(item, RunItem::Message { .. }) {
            continue;
        }
        let mut matches = after
            .iter()
            .enumerate()
            .filter(|(_, candidate)| *candidate == item);
        if let Some((new_index, _)) = matches.next() {
            if matches.next().is_some()
                || before.iter().filter(|candidate| *candidate == item).count() > 1
                || correspondence
                    .last()
                    .is_some_and(|&(_, previous)| previous >= new_index)
            {
                return Err(BridgeError("ambiguous approval history correspondence"));
            }
            correspondence.push((old_index, new_index));
        }
    }
    entry.history_before = Some(
        correspondence
            .iter()
            .find(|(old, _)| *old >= boundary)
            .map(|(_, new)| *new)
            .or_else(|| correspondence.last().map(|(_, new)| new + 1))
            .ok_or(BridgeError("no surviving approval boundary anchor"))?,
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoApprovalDecision {
    pub approved: bool,
    pub reason: String,
}

pub trait GoApprovalGate: Send + Sync {
    fn approve<'a>(
        &'a self,
        context: &'a Context,
        request: &'a ApprovalRequest,
    ) -> BoxFuture<'a, Result<GoApprovalDecision, Error>>;
}

struct DeferredHost(Arc<dyn Host>);

impl Host for DeferredHost {
    fn emit<'a>(
        &'a self,
        context: &'a Context,
        event: RunEvent,
    ) -> BoxFuture<'a, Result<(), Error>> {
        self.0.emit(context, event)
    }

    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Defer) })
    }
}

pub const DEFAULT_GO_MAX_RESUMES: usize = 12;

#[derive(Debug)]
struct RetainedSpills(Vec<Arc<crate::output::SpillFile>>);

impl std::fmt::Display for RetainedSpills {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "retaining {} tool-output spill files",
            self.0.len()
        )
    }
}

impl std::error::Error for RetainedSpills {}

/// Execute eligible effects before asking the gate, then resolve each gate/tool
/// pair sequentially. None and Some(0) select the Go default of twelve resumes.
/// Native tool pauses without approvals are returned to the caller unchanged.
pub async fn run_go_chat(
    runner: &Runner,
    context: Context,
    request: RunRequest,
    host: Arc<dyn Host>,
    gate: &dyn GoApprovalGate,
    max_resumes: Option<usize>,
) -> Result<RunOutcome, RunError> {
    let mut outcome = runner
        .run(context, request, Arc::new(DeferredHost(host)))
        .await?;
    let mut resumes = 0;
    while !outcome.result.pending_approvals.is_empty() {
        if resumes
            == max_resumes
                .filter(|limit| *limit > 0)
                .unwrap_or(DEFAULT_GO_MAX_RESUMES)
        {
            outcome.result.status = RunStatus::Incomplete;
            outcome.result.final_output = None;
            return Err(RunError::with_partial(
                Error::new(
                    ErrorCategory::MaxTurns,
                    "Go chat approval resume limit exceeded",
                )
                .with_source(RetainedSpills(outcome.spills)),
                outcome.result,
            ));
        }
        let Some(continuation) = outcome.continuation.take() else {
            return Err(RunError::with_partial(
                Error::new(
                    ErrorCategory::Internal,
                    "pending approvals have no continuation",
                )
                .with_source(RetainedSpills(outcome.spills)),
                outcome.result,
            ));
        };
        outcome = continuation.resume_go_gate(gate).await?;
        resumes += 1;
    }
    Ok(outcome)
}

/// Go stream payload bridge. Tool items are published at the settled batch boundary,
/// so pending markers remain interleaved in call order rather than scheduling order.
#[derive(Debug, Clone, PartialEq)]
pub enum GoStreamEvent {
    TextDelta(String),
    Item(Box<dto::RunItemSnapshot>),
}
pub trait GoEventSink: Send + Sync {
    fn emit<'a>(
        &'a self,
        context: &'a Context,
        event: GoStreamEvent,
    ) -> BoxFuture<'a, Result<(), Error>>;
}
pub struct GoEventAdapter(pub Arc<dyn GoEventSink>);
impl RunHooks for GoEventAdapter {
    fn observe<'a>(
        &'a self,
        context: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            match observation {
                Observation::TextDelta { delta } => {
                    self.0
                        .emit(context, GoStreamEvent::TextDelta(delta))
                        .await?
                }
                Observation::CommittedItems {
                    items,
                    agents,
                    markers,
                } => {
                    // Stream projection is deliberately not a checkpoint codec:
                    // pause state stays on the owned native outcome, while Go's
                    // event carries its ordinary output payload. A native handoff
                    // projects to the paired tool output emitted by the Go runner.
                    let items: Vec<_> = items
                        .into_iter()
                        .map(|item| match item {
                            RunItem::ToolResult {
                                call_id,
                                mut output,
                            } => {
                                output.should_pause = false;
                                RunItem::ToolResult { call_id, output }
                            }
                            RunItem::Handoff { call_id, agent } => RunItem::ToolResult {
                                call_id,
                                output: ToolOutput {
                                    content: vec![Content::Text {
                                        text: format!("Handing off to {agent}"),
                                    }],
                                    is_error: false,
                                    should_pause: false,
                                },
                            },
                            other => other,
                        })
                        .collect();
                    let wire =
                        approval::encode_history(&items, &agents, &markers).map_err(|error| {
                            Error::new(ErrorCategory::Unsupported, error.to_string())
                        })?;
                    for item in adk_codec::snapshot_items(&wire) {
                        self.0
                            .emit(context, GoStreamEvent::Item(Box::new(item)))
                            .await?;
                    }
                }
                _ => {}
            }
            Ok(())
        })
    }
}
