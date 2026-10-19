//! Opt-in autonomous-loop tracking; the host decides when to update, nudge or stop.
//!
//! Feed only the latest result items once per turn. Tool calls count even when
//! their execution pauses; successful outputs do not clear the error history.
//! Native error projection joins text parts with newlines and ignores media and
//! reasoning. A mapped SDK output containing one text part is preserved exactly.
//! Unlike the SDK's machine-sized counters, native counters saturate at `u64::MAX`.

use adk_core::{Content, RunItem};

pub const DEFAULT_MAX_AUTO_LOOPS: u64 = 200;

const FINISH_GUIDANCE: &str = " Finish is a hard stop, not a progress report: if you named any backlog item, TODO, follow-up, remaining risk, or next step, call a tool for the top item now. Call finish only when no actionable work remains or a concrete external blocker prevents useful tool progress.";

#[derive(Debug, Clone, Default)]
pub struct AutoTracker {
    consecutive_no_tool_turns: u64,
    recent_tools: Vec<(String, String)>,
    recent_errors: Vec<String>,
    tool_call_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CircuitBreakerResult {
    pub tripped: bool,
    pub reason: String,
}

impl AutoTracker {
    pub fn update(&mut self, new_items: &[RunItem]) {
        let mut has_tool_calls = false;
        for item in new_items {
            match item {
                RunItem::ToolCall { call } => {
                    has_tool_calls = true;
                    self.tool_call_count = self.tool_call_count.saturating_add(1);
                    self.recent_tools.push((
                        call.name.clone(),
                        tool_call_signature(&call.name, &call.argument_text()),
                    ));
                    if self.recent_tools.len() > 10 {
                        self.recent_tools.remove(0);
                    }
                }
                RunItem::ToolResult { output, .. } if output.is_error => {
                    let mut message = output
                        .content
                        .iter()
                        .filter_map(|part| match part {
                            Content::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if message.len() > 200 {
                        message = truncate_title(&message, 200);
                    }
                    self.recent_errors.push(message);
                    if self.recent_errors.len() > 5 {
                        self.recent_errors.remove(0);
                    }
                }
                _ => {}
            }
        }
        self.consecutive_no_tool_turns = if has_tool_calls {
            0
        } else {
            self.consecutive_no_tool_turns.saturating_add(1)
        };
    }

    pub fn consecutive_no_tool_turns(&self) -> u64 {
        self.consecutive_no_tool_turns
    }

    pub fn tool_call_count(&self) -> u64 {
        self.tool_call_count
    }

    fn consecutive_same_error(&self) -> (usize, &str) {
        let Some(last) = self.recent_errors.last() else {
            return (0, "");
        };
        (
            self.recent_errors
                .iter()
                .rev()
                .take_while(|s| *s == last)
                .count(),
            last,
        )
    }

    fn detect_repetitive_cycle(&self) -> Option<(&[(String, String)], usize)> {
        for cycle_len in 2..=3 {
            if self.recent_tools.len() < cycle_len * 2 {
                continue;
            }
            let cycle = &self.recent_tools[self.recent_tools.len() - cycle_len..];
            let matched = self
                .recent_tools
                .iter()
                .rev()
                .enumerate()
                .take_while(|(offset, call)| call.1 == cycle[cycle_len - 1 - offset % cycle_len].1)
                .count();
            let repetitions = matched / cycle_len;
            if repetitions >= 2 {
                return Some((cycle, repetitions));
            }
        }
        None
    }

    pub fn check_circuit_breakers(&self) -> CircuitBreakerResult {
        let (error_count, error_message) = self.consecutive_same_error();
        let reason = if self.consecutive_no_tool_turns >= 5 {
            format!(
                "Agent stalled - no tool calls in {} consecutive turns",
                self.consecutive_no_tool_turns
            )
        } else if error_count >= 5 {
            format!(
                "Agent stuck - same error repeated {error_count} times: {}",
                truncate_title(error_message, 100)
            )
        } else if let Some((cycle, repetitions)) = self.detect_repetitive_cycle()
            && repetitions >= 6 / cycle.len()
        {
            let names = cycle
                .iter()
                .map(|call| call.0.as_str())
                .collect::<Vec<_>>()
                .join("->");
            format!("Agent stuck in tool loop - pattern [{names}] repeated {repetitions} times")
        } else {
            return CircuitBreakerResult::default();
        };
        CircuitBreakerResult {
            tripped: true,
            reason,
        }
    }
}

pub fn build_smart_nudge(tracker: Option<&AutoTracker>, phase: &str) -> String {
    let empty = AutoTracker::default();
    let tracker = tracker.unwrap_or(&empty);
    if let Some((cycle, _)) = tracker.detect_repetitive_cycle() {
        let names = cycle
            .iter()
            .map(|call| call.0.as_str())
            .collect::<Vec<_>>()
            .join(" -> ");
        return format!(
            "[SYSTEM] Detected repetitive tool pattern: [{names}]. You appear stuck in a loop. Step back, reassess your approach, and try something fundamentally different. If the current subtask is blocked, skip it and move on to other work.{FINISH_GUIDANCE}"
        );
    }
    let (error_count, error_message) = tracker.consecutive_same_error();
    if error_count >= 3 {
        let message = truncate_title(error_message, 120);
        return format!(
            "[SYSTEM] You have hit the same error {error_count} times: \"{message}\". Do NOT retry the same approach. Try a fundamentally different strategy, or skip this subtask and move on.{FINISH_GUIDANCE}"
        );
    }
    if tracker.consecutive_no_tool_turns >= 3 {
        return format!(
            "[SYSTEM] WARNING: You have produced text without tool calls for {} consecutive turns. In autonomous mode, every turn MUST include at least one tool call. Do NOT output only text.{FINISH_GUIDANCE}",
            tracker.consecutive_no_tool_turns
        );
    }
    if tracker.consecutive_no_tool_turns >= 1 {
        return format!(
            "[SYSTEM] You produced text without tool calls. In autonomous mode, every turn must include at least one tool call.{FINISH_GUIDANCE}"
        );
    }
    let phase_hint = if phase.is_empty() {
        String::new()
    } else {
        format!(" You are in the {phase} phase.")
    };
    format!(
        "[SYSTEM] Continue.{phase_hint} {} tool calls so far. Use tools to make progress.{FINISH_GUIDANCE}",
        tracker.tool_call_count
    )
}

fn tool_call_signature(name: &str, input: &str) -> String {
    let compact = compact_json(input);
    let input = compact.as_deref().unwrap_or(input.as_bytes());
    let mut hash = 0xcbf29ce484222325u64;
    for byte in name.as_bytes().iter().chain([0].iter()).chain(input) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{name}:{hash:x}")
}

fn compact_json(input: &str) -> Option<Vec<u8>> {
    // IgnoredAny validates syntax without decoding numbers, keys or surrogate
    // escapes, and without serde's recursive Value depth limit.
    serde_json::from_str::<serde::de::IgnoredAny>(input).ok()?;
    let mut compact = Vec::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;
    for byte in input.bytes() {
        if in_string {
            compact.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b' ' | b'\n' | b'\r' | b'\t' => continue,
                b'"' => in_string = true,
                b'[' | b'{' => {
                    depth += 1;
                    // encoding/json rejects nesting deeper than 10,000.
                    if depth > 10_000 {
                        return None;
                    }
                }
                b']' | b'}' => depth -= 1,
                _ => {}
            }
            compact.push(byte);
        }
    }
    Some(compact)
}

fn truncate_title(text: &str, max_len: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_len {
        return text.to_owned();
    }
    if max_len <= 3 {
        return text.chars().take(max_len).collect();
    }
    let mut title: String = text.chars().take(max_len - 3).collect();
    title.push_str("...");
    title
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{ToolCall, ToolOutput};

    fn call(name: &str, raw: &str) -> RunItem {
        RunItem::ToolCall {
            call: ToolCall {
                id: "id".into(),
                name: name.into(),
                arguments: serde_json::Value::Null,
                raw_arguments: Some(raw.into()),
            },
        }
    }

    fn error(text: &str) -> RunItem {
        RunItem::ToolResult {
            call_id: "id".into(),
            output: ToolOutput {
                content: vec![Content::Text { text: text.into() }],
                is_error: true,
                should_pause: false,
            },
        }
    }

    #[test]
    fn compact_preserves_spelling_and_invalid_input() {
        let raw = " { \"z\": 1e+9999, \"a\": -0, \"z\": \"\\ud800 \\u0000\" } \n";
        let compact = "{\"z\":1e+9999,\"a\":-0,\"z\":\"\\ud800 \\u0000\"}";
        assert_eq!(compact_json(raw).unwrap(), compact.as_bytes());
        assert_eq!(
            tool_call_signature("read", raw),
            tool_call_signature("read", compact)
        );
        for raw in [
            "",
            " ",
            " {bad} ",
            "[1,]",
            "01",
            "true false",
            "\"\\uZZZZ\"",
        ] {
            assert!(compact_json(raw).is_none(), "{raw:?}");
        }
        assert_ne!(
            tool_call_signature("read", " {bad} "),
            tool_call_signature("read", "{bad}")
        );
        assert_ne!(
            tool_call_signature("read", ""),
            tool_call_signature("read", "null")
        );
        assert_ne!(
            tool_call_signature("read", "1"),
            tool_call_signature("read", "1.0")
        );
        assert_eq!(tool_call_signature("", ""), ":af63bd4c8601b7df");
        assert_eq!(
            compact_json("\"a \\\" b \\\\ c\"").unwrap(),
            b"\"a \\\" b \\\\ c\""
        );
    }

    #[test]
    fn compact_matches_go_depth_limit_without_recursive_parsing() {
        for depth in [129, 10_000, 10_001] {
            let input = format!(" {}0{} ", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(compact_json(&input).is_some(), depth <= 10_000);
        }
    }

    #[test]
    fn raw_arguments_and_cycle_priority_are_authoritative() {
        let mut tracker = AutoTracker::default();
        for i in 0..10 {
            tracker.update(&[call("read", &i.to_string())]);
        }
        assert!(tracker.detect_repetitive_cycle().is_none());
        for _ in 0..4 {
            tracker.update(&[call("read", "{}")]);
        }
        assert_eq!(tracker.detect_repetitive_cycle().unwrap().1, 2);
        assert!(!tracker.check_circuit_breakers().tripped);
        assert!(
            build_smart_nudge(Some(&tracker), "ignored")
                .starts_with("[SYSTEM] Detected repetitive tool pattern: [read -> read].")
        );
        tracker.update(&[call("read", "{}"), call("read", "{}")]);
        assert_eq!(
            tracker.check_circuit_breakers().reason,
            "Agent stuck in tool loop - pattern [read->read] repeated 3 times"
        );
        assert_eq!(tracker.tool_call_count(), 16);
        assert_eq!(tracker.recent_tools.len(), 10);
    }

    #[test]
    fn three_call_cycles_trip_after_two_repetitions() {
        let mut tracker = AutoTracker::default();
        for _ in 0..2 {
            tracker.update(&[call("a", "{}"), call("b", "{}"), call("c", "{}")]);
        }
        assert_eq!(
            tracker.check_circuit_breakers().reason,
            "Agent stuck in tool loop - pattern [a->b->c] repeated 2 times"
        );
        tracker.update(&[call("a", "1")]);
        assert!(!tracker.check_circuit_breakers().tripped);
    }

    #[test]
    fn errors_persist_and_breaker_priority_differs_from_nudge() {
        let mut tracker = AutoTracker::default();
        for _ in 0..6 {
            tracker.update(&[call("a", "{}"), error(" failure ")]);
        }
        assert_eq!(tracker.recent_errors.len(), 5);
        assert_eq!(
            tracker.check_circuit_breakers().reason,
            "Agent stuck - same error repeated 5 times: failure"
        );
        for _ in 0..5 {
            tracker.update(&[]);
        }
        assert_eq!(
            tracker.check_circuit_breakers().reason,
            "Agent stalled - no tool calls in 5 consecutive turns"
        );
        assert!(
            build_smart_nudge(Some(&tracker), "")
                .starts_with("[SYSTEM] Detected repetitive tool pattern:")
        );
        tracker.update(&[call("different", "{}")]);
        assert_eq!(tracker.consecutive_no_tool_turns(), 0);
        assert!(
            build_smart_nudge(Some(&tracker), "")
                .starts_with("[SYSTEM] You have hit the same error 5 times: \"failure\".")
        );
    }

    #[test]
    fn unicode_truncation_and_native_output_projection() {
        assert_eq!(truncate_title("\u{85}😀é水abc\u{a0}", 5), "😀é...");
        assert_eq!(truncate_title(" 😀é水abc ", 3), "😀é水");
        let mut tracker = AutoTracker::default();
        let mut item = error(" first ");
        if let RunItem::ToolResult { output, .. } = &mut item {
            output.should_pause = true;
            output.content.push(Content::Image {
                uri: "ignored".into(),
                media_type: "image/png".into(),
            });
            output.content.push(Content::Text {
                text: "second".into(),
            });
        }
        tracker.update(&[item]);
        assert_eq!(tracker.recent_errors[0], " first \nsecond");
        tracker.update(&[error(&format!(" {} ", "水".repeat(100)))]);
        assert_eq!(tracker.recent_errors[1], "水".repeat(100));
        tracker.update(&[error(&"水".repeat(201))]);
        assert_eq!(tracker.recent_errors[2], format!("{}...", "水".repeat(197)));
    }

    #[test]
    fn default_nudges_and_saturating_counters() {
        assert_eq!(DEFAULT_MAX_AUTO_LOOPS, 200);
        assert_eq!(
            build_smart_nudge(None, "build"),
            format!(
                "[SYSTEM] Continue. You are in the build phase. 0 tool calls so far. Use tools to make progress.{FINISH_GUIDANCE}"
            )
        );
        let mut tracker = AutoTracker::default();
        assert_eq!(
            tracker.check_circuit_breakers(),
            CircuitBreakerResult::default()
        );
        tracker.update(&[]);
        assert_eq!(
            build_smart_nudge(Some(&tracker), "ignored"),
            format!(
                "[SYSTEM] You produced text without tool calls. In autonomous mode, every turn must include at least one tool call.{FINISH_GUIDANCE}"
            )
        );
        tracker.update(&[]);
        tracker.update(&[]);
        assert_eq!(
            build_smart_nudge(Some(&tracker), ""),
            format!(
                "[SYSTEM] WARNING: You have produced text without tool calls for 3 consecutive turns. In autonomous mode, every turn MUST include at least one tool call. Do NOT output only text.{FINISH_GUIDANCE}"
            )
        );
        tracker.consecutive_no_tool_turns = u64::MAX;
        tracker.tool_call_count = u64::MAX;
        tracker.update(&[]);
        assert_eq!(tracker.consecutive_no_tool_turns(), u64::MAX);
        tracker.update(&[call("a", "")]);
        assert_eq!(tracker.tool_call_count(), u64::MAX);
        assert_eq!(tracker.consecutive_no_tool_turns(), 0);
    }
}
