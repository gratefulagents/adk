#![cfg(feature = "observability")]
//! Replay stimuli against Rust and compare with independently executed, pinned Go.
//! Snapshots use their actual wire serializer; span data is explicitly projected
//! because tracewriter types are not SDK Go wire DTOs. Nil/empty Go span file
//! slices both project to lists. Only clock readings and generated UUIDs normalize.
use adk::{
    codec::dto::Usage,
    progress::{HookEvent, HookResult, ProgressTracker},
    tracewriter::{Span, SpanData, Trace},
    tracing::TraceProcessor,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(serde::Serialize)]
struct GoFloat(#[serde(serialize_with = "adk::codec::dto::serialize_go_float")] f64);
fn wire_float(value: f64) -> Value {
    serde_json::to_value(GoFloat(value)).unwrap()
}

#[derive(Default)]
struct Captured {
    ids: BTreeMap<String, String>,
    calls: Vec<Value>,
}
#[derive(Default)]
struct Probe(Mutex<Captured>);
impl Probe {
    fn capture(&self, phase: &str, span: &Span) {
        let mut state = self.0.lock().unwrap();
        let next = format!("span-{}", state.ids.len() + 1);
        let id = state.ids.entry(span.id.clone()).or_insert(next).clone();
        let data = match span.data.as_ref().unwrap() {
            SpanData::Session(d) => {
                json!({"kind":"session", "model":d.model,"cost":wire_float(d.cost_usd),"turns":d.num_turns,"duration_ms":d.duration_ms,"input":d.input_tokens,"output":d.output_tokens,"cache_read":d.cache_read_input_tokens,"cache_create":d.cache_creation_input_tokens,"stop":d.stop_reason})
            }
            SpanData::Subagent(d) => {
                json!({"kind":"subagent","task":d.task_id,"type":d.subagent_type,"description":d.description,"model":d.model,"status":d.status,"cost":wire_float(d.cost_usd),"turns":d.num_turns,"total":d.total_tokens,"input":d.input_tokens,"output":d.output_tokens,"cache_read":d.cache_read_tokens,"cache_create":d.cache_creation_tokens,"tools":d.tool_count,"duration_ms":"<elapsed>","stop":d.stop_reason,"isolation":d.isolation,"prompt":d.prompt,"result":d.result_text,"files_read":d.files_read,"files_written":d.files_written})
            }
            SpanData::Retry {
                error_code,
                retry_after_ms,
                attempt,
                max_retries,
            } => {
                json!({"kind":"retry","error":error_code,"after_ms":retry_after_ms,"attempt":attempt,"max":max_retries})
            }
            SpanData::Compaction {
                tokens_before,
                tokens_after,
            } => json!({"kind":"compaction","before":tokens_before,"after":tokens_after}),
            other => panic!("unexpected span {other:?}"),
        };
        let finished = span.end_time
            != adk::tracestore::ZERO_TIME
                .parse::<chrono::DateTime<chrono::FixedOffset>>()
                .unwrap();
        state.calls.push(json!({"phase":phase,"id":id,"parent":span.parent_id,"name":span.name,"finished":finished,"data":data}));
    }
}
impl TraceProcessor for Probe {
    fn span_start(&self, span: &Span) {
        self.capture("start", span);
    }
    fn span_end(&self, span: &Span) {
        self.capture("end", span);
    }
    fn trace_start(&self, _: &Trace) {
        self.0.lock().unwrap().calls.push(json!("trace_start"));
    }
    fn trace_end(&self, _: &Trace) {
        self.0.lock().unwrap().calls.push(json!("trace_end"));
    }
    fn flush(&self) -> Result<(), adk::core::Error> {
        self.0.lock().unwrap().calls.push(json!("flush"));
        Ok(())
    }
}
fn normalize_time(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if key == "timestamp" {
                    // Do not let normalization hide a broken timestamp wire type.
                    chrono::DateTime::parse_from_rfc3339(value.as_str().unwrap()).unwrap();
                    *value = json!("<time>");
                } else {
                    normalize_time(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_time(value);
            }
        }
        _ => (),
    }
}
fn compare(name: &str) {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/progress/observations.json")).unwrap();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap();
    let probe = Arc::new(Probe::default());
    let root = ProgressTracker::new()
        .with_max_tool_result_bytes(7)
        .with_tracing_processor(probe.clone());
    root.set_root_span_id("root-A");
    let mut trackers = BTreeMap::from([("root".to_owned(), root)]);
    let mut tasks: Vec<String> = Vec::new();
    for (index, op) in case["ops"].as_array().unwrap().iter().enumerate() {
        let target = op["target"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("root");
        let tracker = trackers[target].clone();
        let s = |i: usize| op["args"].get(i).and_then(Value::as_str).unwrap_or("");
        let n = |i: usize| op["n"].get(i).and_then(Value::as_i64).unwrap_or(0);
        let cost = op["cost"].as_f64().unwrap_or(0.0);
        let usage = Usage {
            input_tokens: n(0),
            output_tokens: n(1),
            cache_read_tokens: n(2),
            cache_create_tokens: n(3),
            ..Usage::default()
        };
        let mut extra = Value::Null;
        match op["op"].as_str().unwrap() {
            "snapshot" => (),
            "session" => tracker.set_session(n(0) as i32, s(0)),
            "step" => tracker.set_step(s(0)),
            "init" => tracker.record_system_init("model", "cwd", &[], 4, &[], ""),
            "text" => tracker.record_assistant_text(s(0)),
            "thinking" => tracker.record_assistant_thinking(s(0)),
            "tool" => tracker.record_tool_use(s(0), s(1), "call-1", 1, s(2), "agent", ""),
            "tool_result" => {
                tracker.record_tool_result("call-1", s(0), s(1), n(0) != 0, 42, "agent", "")
            }
            "lifecycle" => tracker.record_lifecycle_event(s(0), s(1)),
            "result" => tracker.write_result(s(0), s(1), s(2), s(3)),
            "hook" => tracker.record_hook_decision(
                &HookEvent {
                    tool_name: "Write".into(),
                    event_type: "pre".into(),
                },
                &HookResult {
                    decision: "deny".into(),
                    reason: "policy".into(),
                },
            ),
            "usage" => tracker.record_llm_usage(s(0), cost, &usage),
            "complete" => tracker.record_session_complete(s(0), cost, n(4), &usage, n(5), s(1)),
            "child" => {
                trackers.insert(s(0).into(), tracker.child(s(0)));
            }
            "root_id" => tracker.set_root_span_id(s(0)),
            "start" => {
                if !tasks.iter().any(|t| t == s(0)) {
                    tasks.push(s(0).into());
                }
                tracker.record_subagent_started(s(0), "call-1", s(1), s(2), s(3), s(4), s(5));
            }
            "progress" => tracker.record_subagent_progress(s(0), n(0) as i32, n(1), n(2), s(1)),
            "finish" => tracker.record_subagent_completed(
                s(0),
                s(1),
                s(2),
                cost,
                n(4),
                &usage,
                s(3),
                &["src/read.rs".into()],
                &["src/write.rs".into()],
            ),
            "retry" => tracker.record_api_retry(s(0), n(0), n(1) as i32, n(2) as i32),
            "compact" => tracker.record_compact_boundary(n(0), n(1), s(0)),
            "drain" => extra = serde_json::to_value(tracker.drain_pending_events()).unwrap(),
            other => panic!("unknown operation {other}"),
        }
        let states: BTreeMap<_, _> = trackers.iter().map(|(name, tr)| {
            let agents: BTreeMap<_, _> = tasks.iter().filter_map(|task| tr.agent_meta(task).map(|meta| (task,meta))).collect();
            // Compare actual serialization, not a hand-maintained snapshot field projection.
            let snapshot = serde_json::from_str::<Value>(&serde_json::to_string(&tr.snapshot()).unwrap()).unwrap();
            (name, json!({"snapshot":snapshot, "model_usage_wire":serde_json::to_string(&tr.snapshot().model_usage).unwrap(),"text":tr.last_assistant_text(),"thinking":tr.last_assistant_thinking(),"session_tools":tr.session_tool_call_count(),"session_cost":wire_float(tr.session_cost_usd()),"task":tr.task_id(),"root":tr.root_span_id(),"max_bytes":tr.max_tool_result_bytes(),"agents":agents}))
        }).collect();
        let mut actual =
            json!({"states":states,"callbacks":probe.0.lock().unwrap().calls,"extra":extra});
        normalize_time(&mut actual);
        assert_eq!(
            actual, fixture["observations"][name][index],
            "{name}, operation {index}: {op}"
        );
    }
}
macro_rules! oracle_test {
    ($($name:ident),+ $(,)?) => { $(#[test] fn $name() { compare(stringify!($name)); })+ };
}
oracle_test!(
    empty_snapshot,
    session_and_step_deduplication,
    assistant_thinking_does_not_replace_activity,
    tool_step_inference,
    tool_result_success_and_error,
    lifecycle_preserves_full_summary,
    result_summary_field_order_and_fallback,
    hook_is_noop,
    usage_accumulates_and_sorts_models,
    nested_usage_forwards_without_parent_activity,
    session_completion_does_not_double_count,
    ring_eviction_does_not_drain_pending,
    retry_and_compaction_callbacks,
    subagent_completion_reuses_start_span,
    orphan_completion_emits_start_and_end,
    nonterminal_completion_retains_metadata,
);
