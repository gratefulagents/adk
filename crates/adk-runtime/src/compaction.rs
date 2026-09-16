//! Deterministic local history compaction, derived from SDK 1dc92b73900fac74dc357a938e4b5eee6392b418.
//! See docs/local-compaction.md for the GPL-3.0-only source, parity boundary and runner integration.
use crate::runner::{CompactedHistory, CompactionConfig, CompactionRequest, Compactor};
use adk_codec::approval::{ApprovalMarker, ApprovalMarkerBoundary};
use adk_codec::dto::RawJson;
use adk_core::{BoxFuture, Content, Context, Error, Message, ModelRequest, Role, RunItem, Usage};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

pub const SUMMARY_MARKER: &str = "[COMPACTED HISTORY SUMMARY]";
pub const CARRY_FORWARD_MARKER: &str = "[COMPACTION CARRY-FORWARD]";
pub const DEFAULT_OUTPUT_RESERVE: u64 = 16_384;
pub const REQUEST_SAFETY_BUFFER: u64 = 8_192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalCompactionPolicy {
    pub enabled: bool,
    pub trigger_tokens: u64,
    pub target_tokens: u64,
    pub preserve_recent_items: usize,
    pub preserve_initial_user_messages: usize,
    pub summary_bullet_limit: usize,
}
impl Default for LocalCompactionPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_tokens: 180_000,
            target_tokens: 100_000,
            preserve_recent_items: 12,
            preserve_initial_user_messages: 2,
            summary_bullet_limit: 4,
        }
    }
}
impl LocalCompactionPolicy {
    pub fn for_model(model: &str) -> Self {
        let model = model
            .trim()
            .split_once('/')
            .map_or(model.trim(), |(_, name)| name)
            .trim()
            .to_lowercase();
        let (trigger_tokens, target_tokens) = if ["spark", "nano", "mini", "lite", "flash"]
            .iter()
            .any(|part| model.contains(part))
        {
            (110_000, 60_000)
        } else if model.starts_with("gpt-6") {
            (244_800, 136_000)
        } else if model.starts_with("gpt-5.6") {
            (334_800, 186_000)
        } else if ["gpt-5.5", "gpt-5.4", "gpt-5.3-codex", "gpt-5.2", "gpt-5.1"]
            .iter()
            .any(|prefix| model.starts_with(prefix))
        {
            (360_000, 200_000)
        } else if model.contains("fable") {
            (900_000, 500_000)
        } else {
            (180_000, 100_000)
        };
        Self {
            trigger_tokens,
            target_tokens,
            ..Self::default()
        }
    }
    pub fn normalized(mut self) -> Self {
        if self.trigger_tokens == 0 {
            self.trigger_tokens = 180_000;
        }
        if self.target_tokens == 0 || self.target_tokens >= self.trigger_tokens {
            self.target_tokens = (self.trigger_tokens / 2).max(1);
        }
        if self.preserve_recent_items == 0 {
            self.preserve_recent_items = 12;
        }
        if self.preserve_initial_user_messages == 0 {
            self.preserve_initial_user_messages = 2;
        }
        if self.summary_bullet_limit == 0 {
            self.summary_bullet_limit = 4;
        }
        self
    }
    pub fn config(self) -> CompactionConfig {
        let policy = self.normalized();
        CompactionConfig {
            trigger_tokens: policy.trigger_tokens,
            target_tokens: policy.target_tokens,
            compactor: Arc::new(LocalCompactor { policy }),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LocalCompactor {
    pub policy: LocalCompactionPolicy,
}
impl Compactor for LocalCompactor {
    fn compact<'a>(
        &'a self,
        _context: &'a Context,
        request: CompactionRequest,
    ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
        Box::pin(async move {
            // The adapter contract receives the *estimated full request*, not billed usage.
            let overhead = request
                .context_tokens
                .saturating_sub(estimate_history_tokens(&request.history));
            let policy = LocalCompactionPolicy {
                target_tokens: request.target_tokens,
                ..self.policy
            };
            let outcome = compact_for_request(
                &request.history,
                policy,
                overhead.min(i64::MAX as u64) as i64,
            );
            let history = if outcome.changed {
                finalize_local_history(&outcome.history, &request.history)
            } else {
                outcome.history
            };
            let context_tokens = if outcome.changed {
                estimate_history_tokens(&history) + overhead
            } else {
                outcome.after_tokens
            };
            Ok(CompactedHistory {
                history,
                context_tokens,
                usage: Usage::default(),
                cost: 0.0,
            })
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalCompactionOutcome {
    pub history: Vec<RunItem>,
    pub markers: Vec<ApprovalMarkerBoundary>,
    pub before_tokens: u64,
    pub after_tokens: u64,
    pub changed: bool,
    pub reason: &'static str,
}

#[derive(Debug, Clone)]
enum HistoryItem {
    Native(RunItem),
    Approval(ApprovalMarker),
}

fn mixed_history(items: &[RunItem], markers: &[ApprovalMarkerBoundary]) -> Vec<HistoryItem> {
    let mut markers: Vec<_> = markers.iter().collect();
    markers.sort_by_key(|boundary| boundary.before_item);
    assert!(
        markers
            .last()
            .is_none_or(|boundary| boundary.before_item <= items.len()),
        "approval boundary exceeds history length"
    );
    let mut markers = markers.into_iter().peekable();
    let mut history = Vec::with_capacity(items.len() + markers.len());
    for index in 0..=items.len() {
        while markers
            .peek()
            .is_some_and(|boundary| boundary.before_item == index)
        {
            history.push(HistoryItem::Approval(
                markers.next().unwrap().marker.clone(),
            ));
        }
        if let Some(item) = items.get(index) {
            history.push(HistoryItem::Native(item.clone()));
        }
    }
    history
}

fn split_history(items: Vec<HistoryItem>) -> (Vec<RunItem>, Vec<ApprovalMarkerBoundary>) {
    let mut history = Vec::new();
    let mut markers = Vec::new();
    for item in items {
        match item {
            HistoryItem::Native(item) => history.push(item),
            HistoryItem::Approval(marker) => markers.push(ApprovalMarkerBoundary {
                before_item: history.len(),
                marker,
            }),
        }
    }
    (history, markers)
}

/// Default Go finalization without approval markers or dynamic carry-forward hooks.
pub fn finalize_local_history(compacted: &[RunItem], previous: &[RunItem]) -> Vec<RunItem> {
    finalize_local_history_with_approvals(compacted, &[], previous, &[]).0
}

/// Finalize ordered mixed history, preserving approval provenance and pending call references.
pub fn finalize_local_history_with_approvals(
    compacted: &[RunItem],
    markers: &[ApprovalMarkerBoundary],
    previous: &[RunItem],
    previous_markers: &[ApprovalMarkerBoundary],
) -> (Vec<RunItem>, Vec<ApprovalMarkerBoundary>) {
    split_history(finalize_mixed_history(
        &mixed_history(compacted, markers),
        &mixed_history(previous, previous_markers),
    ))
}

/// Default Go post-compaction finalization, with no dynamic carry-forward hook.
/// Call only after a successful plan. `previous` is the uncompacted history.
fn finalize_mixed_history(compacted: &[HistoryItem], previous: &[HistoryItem]) -> Vec<HistoryItem> {
    let items: Vec<_> = compacted
        .iter()
        .filter(|item| {
            !matches!(
                item,
                HistoryItem::Native(RunItem::Message { .. } | RunItem::PhasedMessage { .. })
            ) || !item_text(item).starts_with(CARRY_FORWARD_MARKER)
        })
        .cloned()
        .collect();
    let mut ref_calls = HashMap::new();
    let mut ref_outputs = HashMap::new();
    for item in previous.iter().chain(items.iter()) {
        match item {
            HistoryItem::Native(RunItem::ToolCall { call }) if !call.id.is_empty() => {
                ref_calls.insert(call.id.as_str(), item);
            }
            HistoryItem::Native(RunItem::ToolResult { call_id, .. }) if !call_id.is_empty() => {
                ref_outputs.insert(call_id.as_str(), item);
            }
            _ => {}
        }
    }
    let current_outputs: HashSet<_> = items
        .iter()
        .filter_map(|item| match item {
            HistoryItem::Native(RunItem::ToolResult { call_id, .. }) if !call_id.is_empty() => {
                Some(call_id.as_str())
            }
            _ => None,
        })
        .collect();
    let pending_approvals: HashSet<_> = previous
        .iter()
        .chain(items.iter())
        .filter_map(|item| {
            if let HistoryItem::Approval(marker) = item {
                Some(marker.data.call_id.as_str())
            } else {
                None
            }
        })
        .collect();
    let last_output = items
        .iter()
        .rposition(|item| matches!(item, HistoryItem::Native(RunItem::ToolResult { .. })));
    let mut emitted_calls = HashSet::new();
    let mut emitted_outputs = HashSet::new();
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            HistoryItem::Native(RunItem::ToolCall { call }) => {
                let id = call.id.as_str();
                if id.is_empty() || emitted_calls.contains(id) {
                    continue;
                }
                if !current_outputs.contains(id)
                    && !ref_outputs.contains_key(id)
                    && !pending_approvals.contains(id)
                    && last_output.is_some_and(|last| index <= last)
                {
                    continue;
                }
                out.push(item.clone());
                emitted_calls.insert(id);
                if !current_outputs.contains(id)
                    && let Some(output) = ref_outputs.get(id)
                {
                    out.push((*output).clone());
                    emitted_outputs.insert(id);
                }
            }
            HistoryItem::Native(RunItem::ToolResult { call_id, .. }) => {
                let id = call_id.as_str();
                if id.is_empty() || emitted_outputs.contains(id) {
                    continue;
                }
                if !emitted_calls.contains(id) {
                    if let Some(call) = ref_calls.get(id) {
                        out.push((*call).clone());
                        emitted_calls.insert(id);
                    } else {
                        continue;
                    }
                }
                out.push(item.clone());
                emitted_outputs.insert(id);
            }
            _ => out.push(item.clone()),
        }
    }
    out
}

pub fn estimate_string_tokens(text: &str) -> u64 {
    let text = text.trim();
    if text.is_empty() {
        0
    } else {
        text.chars().count() as u64 / 4 + 1
    }
}
fn content_text(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            Content::Text { text } | Content::Reasoning { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn item_text(item: &HistoryItem) -> String {
    match item {
        HistoryItem::Native(
            RunItem::Message { message } | RunItem::PhasedMessage { message, .. },
        ) => content_text(&message.content).trim().into(),
        HistoryItem::Native(RunItem::Reasoning { reasoning }) => reasoning.text.trim().into(),
        HistoryItem::Native(RunItem::Compaction { .. }) => "OpenAI compaction item".into(),
        _ => String::new(),
    }
}
fn arguments(call: &adk_core::ToolCall) -> String {
    call.arguments.to_string()
}
fn estimate_mixed_tokens(items: &[HistoryItem]) -> u64 {
    items
        .iter()
        .map(|item| match item {
            HistoryItem::Native(
                RunItem::Message { message } | RunItem::PhasedMessage { message, .. },
            ) => estimate_string_tokens(&content_text(&message.content)) + 8,
            HistoryItem::Native(RunItem::ToolCall { call }) => {
                estimate_string_tokens(&call.name) + estimate_string_tokens(&arguments(call)) + 16
            }
            HistoryItem::Native(RunItem::ToolResult { output, .. }) => {
                estimate_string_tokens(&content_text(&output.content)) + 12
            }
            HistoryItem::Native(RunItem::Handoff { agent, .. }) => {
                estimate_string_tokens(agent) + 8
            }
            HistoryItem::Native(RunItem::Reasoning { reasoning }) => {
                estimate_string_tokens(&reasoning.text) + 8
            }
            HistoryItem::Native(RunItem::Compaction { compaction }) => {
                estimate_string_tokens(&compaction.encrypted_content).min(20_000) + 8
            }
            HistoryItem::Approval(marker) => {
                let input = match &marker.data.input {
                    RawJson::Missing => String::new(),
                    RawJson::Present(value) => value.to_string(),
                };
                estimate_string_tokens(&marker.data.tool_name) + estimate_string_tokens(&input) + 8
            }
        })
        .sum()
}
pub fn estimate_history_tokens(items: &[RunItem]) -> u64 {
    estimate_history_tokens_with_approvals(items, &[])
}

pub fn estimate_history_tokens_with_approvals(
    items: &[RunItem],
    markers: &[ApprovalMarkerBoundary],
) -> u64 {
    estimate_mixed_tokens(&mixed_history(items, markers))
}
pub fn output_reserve_tokens(request: &ModelRequest) -> u64 {
    let setting = |name| {
        request
            .settings
            .get(name)
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    };
    let max = setting("max_tokens");
    (if max == 0 {
        DEFAULT_OUTPUT_RESERVE
    } else {
        max
    })
    .max(setting("thinking_budget"))
}
pub fn estimate_request_overhead_tokens(request: &ModelRequest) -> u64 {
    estimate_string_tokens(&request.instructions)
        + 8
        + request
            .tools
            .iter()
            .map(|t| {
                estimate_string_tokens(&t.name)
                    + estimate_string_tokens(&t.description)
                    + estimate_string_tokens(
                        &serde_json::to_string(&t.input_schema).expect("schema serializes"),
                    )
                    + 32
            })
            .sum::<u64>()
        + output_reserve_tokens(request)
        + REQUEST_SAFETY_BUFFER
}

#[derive(Debug, Clone, Copy)]
pub struct EstimateCalibration(pub f64);
impl Default for EstimateCalibration {
    fn default() -> Self {
        Self(1.0)
    }
}
impl EstimateCalibration {
    pub fn observe(
        &mut self,
        actual_prompt_tokens: u64,
        sent_history: &[RunItem],
        request: &ModelRequest,
    ) {
        let prompt_overhead = estimate_request_overhead_tokens(request)
            - output_reserve_tokens(request)
            - REQUEST_SAFETY_BUFFER;
        self.observe_estimate(
            actual_prompt_tokens,
            estimate_history_tokens(sent_history) + prompt_overhead,
        );
    }
    pub fn observe_estimate(&mut self, actual_prompt_tokens: u64, estimated_prompt_tokens: u64) {
        if actual_prompt_tokens > 0 && estimated_prompt_tokens > 0 {
            self.0 = (0.5 * self.0
                + 0.5 * actual_prompt_tokens as f64 / estimated_prompt_tokens as f64)
                .clamp(0.5, 2.5);
        }
    }
    pub fn apply(self, policy: LocalCompactionPolicy) -> LocalCompactionPolicy {
        if !policy.enabled || self.0 == 1.0 {
            return policy;
        }
        let mut policy = policy.normalized();
        policy.trigger_tokens = ((policy.trigger_tokens as f64 / self.0) as u64).max(1);
        policy.target_tokens = ((policy.target_tokens as f64 / self.0) as u64).max(1);
        policy
    }
}

pub fn compact_for_request(
    items: &[RunItem],
    policy: LocalCompactionPolicy,
    overhead: i64,
) -> LocalCompactionOutcome {
    compact_with_approvals(items, &[], policy, overhead)
}

/// Plan compaction over native items and ordered approval markers without mutating either input.
/// Boundaries must not exceed `items.len()`; equal boundaries retain marker slice order.
pub fn compact_with_approvals(
    items: &[RunItem],
    markers: &[ApprovalMarkerBoundary],
    policy: LocalCompactionPolicy,
    overhead: i64,
) -> LocalCompactionOutcome {
    compact_mixed(&mixed_history(items, markers), policy, overhead)
}

pub fn extract_summary(items: &[RunItem]) -> String {
    extract_mixed_summary(&mixed_history(items, &[]))
}

fn compact_mixed(
    items: &[HistoryItem],
    policy: LocalCompactionPolicy,
    overhead: i64,
) -> LocalCompactionOutcome {
    let policy = policy.normalized();
    let unchanged = |tokens, reason| {
        let (history, markers) = split_history(items.to_vec());
        LocalCompactionOutcome {
            history,
            markers,
            before_tokens: tokens,
            after_tokens: tokens,
            changed: false,
            reason,
        }
    };
    if !policy.enabled || items.is_empty() {
        return unchanged(0, "disabled");
    }
    let overhead = overhead.max(0) as u64;
    let before = estimate_mixed_tokens(items) + overhead;
    if before <= policy.trigger_tokens {
        return unchanged(before, "below-threshold");
    }
    // Go normalizes again after subtracting overhead (including target >= trigger).
    let adjusted = LocalCompactionPolicy {
        trigger_tokens: policy.trigger_tokens.saturating_sub(overhead).max(1),
        target_tokens: policy.target_tokens.saturating_sub(overhead).max(1),
        ..policy
    }
    .normalized();
    if estimate_mixed_tokens(items) <= adjusted.trigger_tokens {
        return unchanged(before, "below-threshold");
    }
    let mut prefix = HashSet::new();
    let mut initial = 0;
    for (i, item) in items.iter().enumerate() {
        if must_preserve(item) {
            prefix.insert(i);
        }
        if initial < adjusted.preserve_initial_user_messages && is_initial_user(item) {
            prefix.insert(i);
            initial += 1;
        }
    }
    let mut best = None;
    let mut reason = "no-removable-history";
    for recent in (1..=adjusted.preserve_recent_items.min(items.len()).max(1)).rev() {
        let mut protected = prefix.clone();
        protected.extend(items.len().saturating_sub(recent)..items.len());
        protect_pairs(items, &mut protected);
        let removed: Vec<_> = items
            .iter()
            .enumerate()
            .filter(|(i, _)| !protected.contains(i))
            .map(|(_, item)| item.clone())
            .collect();
        if removed.is_empty() {
            continue;
        }
        let mut summary = summarize(&removed, adjusted.summary_bullet_limit);
        if estimate_string_tokens(&summary) + 8 >= estimate_mixed_tokens(&removed) {
            summary = summarize_terse(&removed, adjusted.summary_bullet_limit);
        }
        let mut history = rebuild(items, &protected, &summary);
        let mut after = estimate_mixed_tokens(&history);
        if after > adjusted.target_tokens {
            summary = summarize_terse(&removed, adjusted.summary_bullet_limit);
            history = rebuild(items, &protected, &summary);
            after = estimate_mixed_tokens(&history);
        }
        if after > adjusted.target_tokens {
            history = rebuild(
                items,
                &protected,
                "[COMPACTED HISTORY SUMMARY]\nEarlier context compacted.",
            );
            after = estimate_mixed_tokens(&history);
        }
        if after >= before - overhead {
            if reason == "no-removable-history" {
                reason = "ineffective-summary";
            }
            continue;
        }
        let (history, markers) = split_history(history);
        let outcome = LocalCompactionOutcome {
            history,
            markers,
            before_tokens: before,
            after_tokens: after + overhead,
            changed: true,
            reason: "",
        };
        if best
            .as_ref()
            .is_none_or(|b: &LocalCompactionOutcome| outcome.after_tokens < b.after_tokens)
        {
            best = Some(outcome);
        }
        if after <= adjusted.target_tokens {
            return best.expect("candidate selected");
        }
    }
    best.unwrap_or_else(|| unchanged(before, reason))
}
fn must_preserve(item: &HistoryItem) -> bool {
    // Preserve non-text context losslessly; local summaries cannot reconstruct it.
    match item {
        HistoryItem::Native(
            RunItem::Message { message } | RunItem::PhasedMessage { message, .. },
        ) => {
            matches!(message.role, Role::System | Role::Developer)
                || message
                    .content
                    .iter()
                    .any(|c| !matches!(c, Content::Text { .. }))
        }
        HistoryItem::Native(RunItem::ToolResult { output, .. }) => output
            .content
            .iter()
            .any(|c| !matches!(c, Content::Text { .. })),
        HistoryItem::Native(RunItem::Handoff { .. }) => true,
        HistoryItem::Native(RunItem::Compaction { compaction }) => {
            !compaction.encrypted_content.trim().is_empty()
        }
        _ => false,
    }
}
fn is_initial_user(item: &HistoryItem) -> bool {
    matches!(item, HistoryItem::Native(RunItem::Message { message } | RunItem::PhasedMessage { message, .. }) if message.role == Role::User)
        && {
            let text = item_text(item);
            !text.is_empty()
                && !["[SYSTEM]", "[PHASE TRANSITION", CARRY_FORWARD_MARKER]
                    .iter()
                    .any(|p| text.starts_with(p))
        }
}
fn protect_pairs(items: &[HistoryItem], protected: &mut HashSet<usize>) {
    let mut calls = HashMap::new();
    let mut outputs = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        match item {
            HistoryItem::Native(RunItem::ToolCall { call }) if !call.id.is_empty() => {
                calls.insert(&call.id, i);
            }
            HistoryItem::Native(RunItem::ToolResult { call_id, .. }) if !call_id.is_empty() => {
                outputs.insert(call_id, i);
            }
            _ => {}
        }
    }
    let extras: Vec<_> = protected
        .iter()
        .filter_map(|i| match &items[*i] {
            HistoryItem::Native(RunItem::ToolCall { call }) => outputs.get(&call.id),
            HistoryItem::Native(RunItem::ToolResult { call_id, .. }) => calls.get(call_id),
            _ => None,
        })
        .copied()
        .collect();
    protected.extend(extras);
}
fn summary_item(text: &str) -> HistoryItem {
    HistoryItem::Native(RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![Content::Text { text: text.into() }],
        },
    })
}
fn rebuild(items: &[HistoryItem], protected: &HashSet<usize>, summary: &str) -> Vec<HistoryItem> {
    let first_removed = (0..items.len()).find(|i| !protected.contains(i));
    let first_protected = (0..items.len()).find(|i| protected.contains(i));
    let defer = first_protected.filter(|p| {
        first_removed.is_some_and(|r| r < *p)
            && matches!(&items[*p], HistoryItem::Native(RunItem::Message { message } | RunItem::PhasedMessage { message, .. }) if message.role == Role::User)
    });
    let mut out = vec![];
    let mut inserted = false;
    for (i, item) in items.iter().enumerate() {
        if protected.contains(&i) {
            out.push(item.clone());
            if defer == Some(i) && !inserted {
                out.push(summary_item(summary));
                inserted = true;
            }
        } else if !inserted && defer.is_none_or(|d| i > d) {
            out.push(summary_item(summary));
            inserted = true;
        }
    }
    out
}
fn extract_mixed_summary(items: &[HistoryItem]) -> String {
    items
        .iter()
        .rev()
        .find_map(|item| match item {
            HistoryItem::Native(
                RunItem::Message { message } | RunItem::PhasedMessage { message, .. },
            ) if message.role == Role::Assistant => {
                let text = item_text(item);
                text.starts_with(SUMMARY_MARKER).then_some(text)
            }
            _ => None,
        })
        .unwrap_or_default()
}
fn is_summary(item: &HistoryItem) -> bool {
    matches!(item, HistoryItem::Native(RunItem::Message { message } | RunItem::PhasedMessage { message, .. }) if message.role == Role::Assistant)
        && item_text(item).starts_with(SUMMARY_MARKER)
}
fn normalize_summary(text: &str) -> String {
    let mut text = text
        .trim()
        .strip_prefix(SUMMARY_MARKER)
        .unwrap_or(text.trim())
        .trim()
        .to_string();
    if let Some((_, rest)) = text.split_once("<summary>")
        && let Some((body, _)) = rest.split_once("</summary>")
    {
        text = format!("Summary:\n{}", body.trim());
    }
    let mut blank = false;
    text.split('\n')
        .filter(|line| {
            let now = line.trim().is_empty();
            let keep = !now || !blank;
            blank = now;
            keep
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .into()
}
fn truncate(text: &str, max: usize) -> String {
    let text = text.replace('\n', " ");
    let text = text.trim();
    if text.chars().count() <= max {
        text.into()
    } else {
        format!("{}...", text.chars().take(max).collect::<String>())
    }
}
fn scope(items: &[HistoryItem]) -> String {
    let users = items
        .iter()
        .filter(|i| matches!(i, HistoryItem::Native(RunItem::Message { message } | RunItem::PhasedMessage { message, .. }) if message.role == Role::User))
        .count();
    let tools = items
        .iter()
        .filter(|i| matches!(i, HistoryItem::Native(RunItem::ToolResult { .. })))
        .count();
    format!(
        "Scope: {} earlier messages compacted (user={}, assistant={}, tool={}).",
        items.len(),
        users,
        items.len() - users - tools,
        tools
    )
}
fn unique_bullets(
    items: &[HistoryItem],
    limit: usize,
    select: impl Fn(&HistoryItem) -> String,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut bullets = vec![];
    for item in items.iter().rev() {
        let bullet = select(item).trim().to_string();
        if !bullet.is_empty() && seen.insert(bullet.clone()) {
            bullets.push(bullet);
            if bullets.len() >= limit {
                break;
            }
        }
    }
    bullets.reverse();
    bullets
}
fn section(lines: &mut Vec<String>, title: &str, bullets: Vec<String>, indent: &str) {
    if !bullets.is_empty() {
        lines.push(format!("- {title}:"));
        lines.extend(bullets.into_iter().map(|b| format!("{indent}{b}")));
    }
}
fn summary_parts(summary: &str, timeline: bool) -> Vec<String> {
    let mut in_timeline = false;
    let mut out = vec![];
    for line in normalize_summary(summary).split('\n') {
        let line = line.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        if trimmed == "- Key timeline:" || trimmed == "Key timeline:" {
            in_timeline = true;
            continue;
        }
        if timeline && in_timeline && trimmed.is_empty() {
            break;
        }
        if in_timeline == timeline
            && !trimmed.is_empty()
            && (timeline || !["Summary:", "Conversation summary:"].contains(&trimmed))
        {
            out.push(line.into());
        }
    }
    out
}
fn summarize(items: &[HistoryItem], limit: usize) -> String {
    let existing = normalize_summary(&extract_mixed_summary(items));
    let filtered: Vec<_> = items.iter().filter(|i| !is_summary(i)).cloned().collect();
    let items = if filtered.is_empty() {
        items
    } else {
        &filtered
    };
    let mut lines = vec![
        "Conversation summary:".into(),
        format!("- {}", scope(items)),
    ];
    let mut seen = HashSet::new();
    let mut names = vec![];
    for item in items {
        if let HistoryItem::Native(RunItem::ToolCall { call }) = item {
            let name = call.name.trim();
            if !name.is_empty() && seen.insert(name.to_lowercase()) {
                names.push(name);
            }
        }
    }
    names.sort();
    if !names.is_empty() {
        lines.push(format!("- Tools mentioned: {}.", names.join(", ")));
    }
    section(
        &mut lines,
        "Recent user requests",
        unique_bullets(items, limit, |item| {
            let text = item_text(item);
            if matches!(item, HistoryItem::Native(RunItem::Message { message } | RunItem::PhasedMessage { message, .. }) if message.role == Role::User)
                && !text.starts_with("[SYSTEM]")
                && !text.starts_with("[PHASE TRANSITION")
            {
                truncate(&text, 160)
            } else {
                String::new()
            }
        }),
        "  - ",
    );
    section(
        &mut lines,
        "Pending work",
        unique_bullets(items, limit, |item| {
            let text = item_text(item);
            let lower = text.to_lowercase();
            if ["todo", "next", "pending", "follow up", "remaining"]
                .iter()
                .any(|p| lower.contains(p))
            {
                truncate(&text, 160)
            } else {
                String::new()
            }
        }),
        "  - ",
    );
    let files = referenced_paths(items, 8);
    if !files.is_empty() {
        lines.push(format!("- Key files referenced: {}.", files.join(", ")));
    }
    if let Some(work) = items.iter().rev().map(item_text).find(|s| !s.is_empty()) {
        lines.push(format!("- Current work: {}", truncate(&work, 180)));
    }
    lines.push("- Key timeline:".into());
    lines.extend(
        items
            .iter()
            .skip(items.len().saturating_sub((limit * 10).max(20)))
            .filter_map(timeline),
    );
    let new = lines.join("\n");
    if existing.is_empty() {
        return format!("{SUMMARY_MARKER}\n{new}");
    }
    let mut lines = vec!["Conversation summary:".into()];
    section(
        &mut lines,
        "Previously compacted context",
        summary_parts(&existing, false),
        "  ",
    );
    section(
        &mut lines,
        "Newly compacted context",
        summary_parts(&new, false),
        "  ",
    );
    section(&mut lines, "Key timeline", summary_parts(&new, true), "  ");
    format!(
        "{SUMMARY_MARKER}\n{}",
        if lines.len() == 1 {
            new
        } else {
            lines.join("\n")
        }
    )
}
fn summarize_terse(items: &[HistoryItem], limit: usize) -> String {
    let mut lines = vec![SUMMARY_MARKER.into(), scope(items)];
    let mut counts = BTreeMap::<&str, usize>::new();
    for item in items {
        if let HistoryItem::Native(RunItem::ToolCall { call }) = item {
            *counts.entry(&call.name).or_default() += 1;
        }
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let tools: Vec<_> = counts
        .iter()
        .take(limit.min(2))
        .map(|(name, n)| format!("{name} ran {n} time{}", if *n == 1 { "" } else { "s" }))
        .collect();
    if !tools.is_empty() {
        lines.push(format!("Tools: {}.", tools.join("; ")));
    }
    let files = referenced_paths(items, limit.min(2));
    if !files.is_empty() {
        lines.push(format!("Files: {}.", files.join(", ")));
    }
    let existing = normalize_summary(&extract_mixed_summary(items));
    if !existing.is_empty() {
        lines.push(format!(
            "Previously compacted context: {}",
            truncate(&existing, 600)
        ));
    }
    lines.join("\n")
}
fn timeline(item: &HistoryItem) -> Option<String> {
    match item {
        HistoryItem::Native(
            RunItem::Message { message } | RunItem::PhasedMessage { message, .. },
        ) => {
            let text = item_text(item);
            if text.is_empty() {
                None
            } else if text.starts_with(SUMMARY_MARKER) {
                Some("summary: previous compacted context".into())
            } else {
                Some(format!(
                    "  - {}: {}",
                    if message.role == Role::User {
                        "user"
                    } else {
                        "assistant"
                    },
                    truncate(&text, 160)
                ))
            }
        }
        HistoryItem::Native(RunItem::ToolCall { call }) => {
            if call.name.eq_ignore_ascii_case("bash") {
                let command = call
                    .arguments
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if !command.is_empty() {
                    return Some(format!(
                        "  - assistant: tool_use Bash({})",
                        truncate(command, 160)
                    ));
                }
            }
            let args = arguments(call);
            let args = args.trim();
            Some(if args.is_empty() || args == "{}" || args == "null" {
                format!("  - assistant: tool_use {}", call.name)
            } else {
                format!(
                    "  - assistant: tool_use {}({})",
                    call.name,
                    truncate(&truncate(args, 160), 160)
                )
            })
        }
        HistoryItem::Native(RunItem::ToolResult { output, .. }) => Some(format!(
            "  - tool: tool_result: {}{}",
            if output.is_error { "error " } else { "" },
            truncate(&content_text(&output.content), 160)
        )),
        HistoryItem::Approval(marker) => Some(format!(
            "  - assistant: tool_approval {}",
            marker.data.tool_name
        )),
        HistoryItem::Native(RunItem::Reasoning { reasoning }) => {
            (!reasoning.text.trim().is_empty())
                .then(|| format!("  - assistant: {}", truncate(&reasoning.text, 160)))
        }
        HistoryItem::Native(RunItem::Compaction { .. }) => {
            Some("  - assistant: OpenAI compaction item".into())
        }
        HistoryItem::Native(RunItem::Handoff { .. }) => None,
    }
}
fn referenced_paths(items: &[HistoryItem], limit: usize) -> Vec<String> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    for item in items {
        let text = match item {
            HistoryItem::Native(RunItem::Message { .. } | RunItem::PhasedMessage { .. }) => {
                item_text(item)
            }
            HistoryItem::Native(RunItem::ToolCall { call }) => arguments(call),
            HistoryItem::Native(RunItem::ToolResult { output, .. }) => {
                content_text(&output.content)
            }
            _ => String::new(),
        };
        for path in path_matches(&text) {
            let path = path.trim_matches([
                '`', '\'', '"', '.', ',', ';', ':', '(', ')', '[', ']', '{', '}', '<', '>',
            ]);
            let path = path.strip_prefix("./").unwrap_or(path);
            if path.is_empty()
                || (path.starts_with('.') && !path.starts_with(".github/"))
                || [
                    ".git/",
                    "node_modules/",
                    "internal/dashboard/web_dist/",
                    "web/dist/",
                    "dist/",
                    "build/",
                ]
                .iter()
                .any(|p| path.starts_with(p))
            {
                continue;
            }
            if seen.insert(path.to_string()) {
                out.push(path.to_string());
                if out.len() == limit {
                    return out;
                }
            }
        }
    }
    out
}
fn path_matches(text: &str) -> Vec<&str> {
    // Equivalent leftmost, greedy ASCII path matching without adding a regex dependency.
    const EXTENSIONS: &[&str] = &[
        "go", "ts", "tsx", "js", "jsx", "json", "yaml", "yml", "toml", "md", "rs", "swift",
        "proto", "sql", "css", "scss", "html", "sh", "py",
    ];
    let valid = |b: u8| b.is_ascii_alphanumeric() || b"_.@+-".contains(&b);
    let bytes = text.as_bytes();
    let mut pos = 0;
    let mut out = vec![];
    while pos < bytes.len() {
        if !valid(bytes[pos]) {
            pos += 1;
            continue;
        }
        let start = pos;
        let mut end = pos;
        while end < bytes.len() && (valid(bytes[end]) || bytes[end] == b'/') {
            end += 1;
        }
        let mut matched = None;
        for directories in [true, false] {
            for dot in (start + 1..end).rev() {
                if bytes[dot] != b'.' {
                    continue;
                }
                let prefix = &text[start..dot];
                let segments: Vec<_> = prefix.split('/').collect();
                if segments.iter().any(|s| s.is_empty())
                    || (directories && segments.len() < 2)
                    || (!directories && segments.len() != 1)
                {
                    continue;
                }
                for extension in EXTENSIONS {
                    if text[dot + 1..].starts_with(extension) {
                        matched = Some(dot + 1 + extension.len());
                        break;
                    }
                }
                if matched.is_some() {
                    break;
                }
            }
            if matched.is_some() {
                break;
            }
        }
        if let Some(end) = matched {
            out.push(&text[start..end]);
            pos = end;
        } else {
            pos = bytes[start..end]
                .iter()
                .position(|b| *b == b'/')
                .map_or(end, |slash| start + slash + 1);
        }
    }
    out
}
