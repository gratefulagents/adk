use adk_core::*;
use adk_runtime::{compaction::*, tracing::*, *};
use serde_json::json;
use std::{
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn message(role: Role, text: impl Into<String>) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn text(item: &RunItem) -> &str {
    match item {
        RunItem::Message { message } => match &message.content[0] {
            Content::Text { text } => text,
            _ => panic!(),
        },
        _ => panic!(),
    }
}
fn response(items: Vec<RunItem>, summary: bool) -> ModelResponse {
    ModelResponse {
        items,
        usage: Usage {
            requests: 1,
            input_tokens: if summary { 7 } else { 11 },
            output_tokens: if summary { 3 } else { 2 },
            cache_read_tokens: 2,
            cache_creation_tokens: 1,
            context_tokens: Some(100),
        },
        end_turn: Some(true),
        response_id: None,
        metadata: Default::default(),
        snapshot_raw: None,
        snapshot_projection: None,
        raw: None,
    }
}
#[derive(Clone, Copy)]
enum Summary {
    Good,
    Error,
    Empty,
    Large,
    Wait,
    Cancel,
}
struct ModelImpl {
    summary: Summary,
    forced: bool,
    cancel: Arc<CancellationToken>,
    normals: AtomicUsize,
    requests: Mutex<Vec<ModelRequest>>,
}
impl ModelImpl {
    fn new(summary: Summary, forced: bool) -> Arc<Self> {
        Arc::new(Self {
            summary,
            forced,
            cancel: Arc::new(CancellationToken::new()),
            normals: AtomicUsize::new(0),
            requests: Mutex::new(vec![]),
        })
    }
}
impl Model for ModelImpl {
    fn provider(&self) -> &str {
        "test"
    }
    fn complete<'a>(
        &'a self,
        _context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let summary = request.instructions.starts_with("You are compacting");
            self.requests.lock().unwrap().push(request);
            if summary {
                let items = match self.summary {
                    Summary::Good => vec![
                        message(Role::Assistant, "  [COMPACTED HISTORY SUMMARY]\n sentinel "),
                        RunItem::Reasoning {
                            reasoning: Reasoning {
                                text: "ignore reasoning".into(),
                                ..Default::default()
                            },
                        },
                        message(Role::Assistant, " next step  "),
                    ],
                    Summary::Error => {
                        return Err(Error::new(ErrorCategory::Provider, "summary failed"));
                    }
                    Summary::Empty => {
                        vec![message(Role::Assistant, " [COMPACTED HISTORY SUMMARY]  ")]
                    }
                    Summary::Large => vec![message(Role::Assistant, "oversized ".repeat(80_000))],
                    Summary::Wait => return std::future::pending().await,
                    Summary::Cancel => {
                        self.cancel.cancel();
                        return std::future::pending().await;
                    }
                };
                Ok(response(items, true))
            } else if self.forced && self.normals.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(Error::new(
                    ErrorCategory::Provider,
                    "context_length_exceeded",
                ))
            } else {
                Ok(response(vec![message(Role::Assistant, "done")], false))
            }
        })
    }
}
struct HostImpl;
impl Host for HostImpl {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
#[derive(Default)]
struct Records(Mutex<Vec<GenerationRecord>>);
impl GenerationObserver for Records {
    fn start(&self, _: &Context, _: &GenerationRecord) {}
    fn end(&self, _: &Context, record: &GenerationRecord) {
        self.0.lock().unwrap().push(record.clone());
    }
}
#[derive(Default)]
struct Hooks(Mutex<Vec<Observation>>);
impl RunHooks for Hooks {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        event: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(event);
            Ok(())
        })
    }
}
struct Cost;
impl CostEstimator for Cost {
    fn cost(&self, model: &str, _: &Usage) -> f64 {
        assert_eq!(model, "active");
        0.25
    }
}
fn context() -> Context {
    Context {
        run_id: "llm-compaction".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn input() -> Vec<RunItem> {
    vec![
        message(Role::User, "protected task"),
        message(Role::Assistant, "removed evidence ".repeat(4_000)),
        message(Role::User, "protected latest"),
    ]
}
fn request() -> RunRequest {
    RunRequest {
        input: input(),
        input_provenance: vec![
            ItemProvenance::Unattributed,
            ItemProvenance::Agent {
                name: "old-agent".into(),
            },
            ItemProvenance::Unattributed,
        ],
        policy: RunPolicy {
            max_turns: NonZeroU32::new(2).unwrap(),
            ..Default::default()
        },
    }
}
fn config(forced: bool) -> RunnerConfig {
    RunnerConfig {
        local_compaction: LocalCompactionPolicy {
            trigger_tokens: if forced { 1_000_000 } else { 30_000 },
            target_tokens: 20_000,
            preserve_initial_user_messages: 1,
            preserve_recent_items: 1,
            ..Default::default()
        },
        transient_context: vec![message(Role::User, "private transient sentinel")],
        prompt_cache_key: Some("cache".into()),
        ..Default::default()
    }
}
fn runner(model: Arc<ModelImpl>, config: RunnerConfig) -> Runner {
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("active", model));
    agent.instructions = "normal instructions".into();
    agent.settings.insert("temperature".into(), json!(0.7));
    Runner::new(agent, config).unwrap()
}

#[tokio::test]
async fn llm_summary_normal_and_forced_request_usage_provenance_and_trace_contract() {
    for forced in [false, true] {
        let model = ModelImpl::new(Summary::Good, forced);
        let records = Arc::new(Records::default());
        let hooks = Arc::new(Hooks::default());
        let mut config = config(forced);
        config.generation_observer = Some(records.clone());
        config.hooks = Some(hooks.clone());
        config.cost_estimator = Some(Arc::new(Cost));
        let result = runner(model.clone(), config)
            .run(context(), request(), Arc::new(HostImpl))
            .await
            .unwrap()
            .result;
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), if forced { 3 } else { 2 });
        let summary = &requests[usize::from(forced)];
        assert_eq!(summary.model, "active");
        assert_eq!(
            summary.settings,
            serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
                json!({"max_tokens":2048,"reasoning_effort":"low"})
            )
            .unwrap()
        );
        assert!(summary.tools.is_empty());
        assert!(summary.output_schema.is_none());
        assert!(summary.output_schema_name.is_empty());
        assert!(!summary.output_schema_strict);
        assert_eq!(summary.input_provenance, vec![ItemProvenance::Unattributed]);
        assert_eq!(summary.input.len(), 1);
        let transcript = text(&summary.input[0]);
        assert!(
            transcript
                .starts_with("Transcript segment to summarize:\n\n[assistant] removed evidence")
        );
        assert!(!transcript.contains("protected"));
        assert!(!transcript.contains("private transient"));
        let last = requests.last().unwrap();
        assert!(extract_summary(&last.input).ends_with("sentinel\nnext step"));
        assert!(last.settings.contains_key("prompt_cache_key"));
        assert_eq!(
            text(last.input.last().unwrap()),
            "private transient sentinel"
        );
        assert_eq!(last.input_provenance[0], ItemProvenance::Unattributed);
        assert_eq!(result.new_items, vec![message(Role::Assistant, "done")]);
        assert_eq!(result.responses.len(), 1);
        assert_eq!(result.usage.requests, 2);
        assert_eq!(result.usage.input_tokens, 18);
        assert_eq!(result.usage.output_tokens, 5);
        assert_eq!(result.usage.cache_read_tokens, 4);
        assert_eq!(result.usage.cache_creation_tokens, 2);
        assert!(
            hooks
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, Observation::Usage { cost, .. } if *cost == 0.5))
        );
        let records = records.0.lock().unwrap();
        assert_eq!(records.len(), if forced { 2 } else { 1 });
        assert_eq!(&records.last().unwrap().request, last);
        assert!(records.last().unwrap().request_snapshot.is_ok());
    }
}

#[tokio::test]
async fn rejected_summaries_keep_deterministic_history_and_charge_returned_usage() {
    for forced in [false, true] {
        for behavior in [Summary::Error, Summary::Empty, Summary::Large] {
            let model = ModelImpl::new(behavior, forced);
            let mut deterministic = config(forced);
            deterministic.local_compaction.use_llm_summary = false;
            let expected = runner(ModelImpl::new(Summary::Good, forced), deterministic)
                .run(context(), request(), Arc::new(HostImpl))
                .await
                .unwrap()
                .result;
            let hooks = Arc::new(Hooks::default());
            let mut config = config(forced);
            config.hooks = Some(hooks.clone());
            let result = runner(model.clone(), config)
                .run(context(), request(), Arc::new(HostImpl))
                .await
                .unwrap()
                .result;
            assert_eq!(
                extract_summary(&result.history),
                extract_summary(&expected.history)
            );
            assert_eq!(
                model.requests.lock().unwrap().len(),
                if forced { 3 } else { 2 }
            );
            let returned = !matches!(behavior, Summary::Error);
            assert_eq!(result.usage.requests, if returned { 2 } else { 1 });
            assert_eq!(result.usage.input_tokens, if returned { 18 } else { 11 });
            assert_eq!(result.responses.len(), 1);
            let events = hooks.0.lock().unwrap();
            let warnings = events
                .iter()
                .filter_map(|event| match event {
                    Observation::CompactionFailed { error } => Some(error),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(warnings.len(), 1);
            assert_eq!(
                warnings[0].message,
                "LLM compaction summary unavailable or ineffective; keeping deterministic summary"
            );
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, Observation::Compacted { .. }))
            );
        }
    }
}

#[tokio::test]
async fn explicit_false_uses_no_summary_calls_normal_or_forced() {
    for forced in [false, true] {
        let model = ModelImpl::new(Summary::Good, forced);
        let mut config = config(forced);
        config.local_compaction.use_llm_summary = false;
        let result = runner(model.clone(), config)
            .run(context(), request(), Arc::new(HostImpl))
            .await
            .unwrap()
            .result;
        assert!(!extract_summary(&result.history).contains("sentinel"));
        assert_eq!(
            model.requests.lock().unwrap().len(),
            if forced { 2 } else { 1 }
        );
        assert_eq!(result.usage.requests, 1);
    }
}

#[tokio::test(start_paused = true)]
async fn summary_timeout_defaults_to_120_seconds_and_honors_run_override() {
    for forced in [false, true] {
        for timeout in [None, Some(Duration::from_secs(3))] {
            let model = ModelImpl::new(Summary::Wait, forced);
            let mut config = config(forced);
            config.model_idle_timeout = timeout;
            let start = tokio::time::Instant::now();
            let result = runner(model.clone(), config)
                .run(context(), request(), Arc::new(HostImpl))
                .await
                .unwrap()
                .result;
            assert_eq!(
                start.elapsed(),
                timeout
                    .filter(|t| !t.is_zero())
                    .unwrap_or(Duration::from_secs(120))
            );
            assert!(!extract_summary(&result.history).is_empty());
            assert_eq!(result.usage.requests, 1);
            assert_eq!(
                model.requests.lock().unwrap().len(),
                if forced { 3 } else { 2 }
            );
        }
    }
}

#[tokio::test]
async fn cancellation_and_deadline_during_summary_do_not_dispatch_next_generation() {
    for forced in [false, true] {
        for cancel in [false, true] {
            let model = ModelImpl::new(
                if cancel {
                    Summary::Cancel
                } else {
                    Summary::Wait
                },
                forced,
            );
            let mut context = context();
            context.cancellation = model.cancel.clone();
            if !cancel {
                context.deadline = Some(std::time::Instant::now() + Duration::from_millis(30));
            }
            let error = runner(model.clone(), config(forced))
                .run(context, request(), Arc::new(HostImpl))
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.error.info.category,
                if cancel {
                    ErrorCategory::Cancelled
                } else {
                    ErrorCategory::DeadlineExceeded
                }
            );
            assert_eq!(
                model.requests.lock().unwrap().len(),
                if forced { 2 } else { 1 }
            );
            assert_eq!(error.partial.unwrap().usage.requests, 0);
        }
    }
}

struct Events(Option<ModelResponse>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async {
            Ok(self
                .0
                .take()
                .map(|response| ModelEvent::Complete { response }))
        })
    }
}
impl StreamingModel for ModelImpl {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        assert!(!request.instructions.starts_with("You are compacting"));
        Box::pin(async move {
            Ok(
                Box::new(Events(Some(self.complete(context, request).await?)))
                    as Box<dyn ModelStream>,
            )
        })
    }
}
#[tokio::test]
async fn streaming_generation_still_summarizes_via_complete_and_carries_live_state() {
    for forced in [false, true] {
        let model = ModelImpl::new(Summary::Good, forced);
        let mut config = config(forced);
        config.working_state_context = "live-state sentinel".into();
        let runner = Runner::new(
            AgentConfig::new("agent", ModelBinding::streaming("active", model.clone())),
            config,
        )
        .unwrap();
        let result = runner
            .stream(context(), request(), Arc::new(HostImpl))
            .finish()
            .await
            .unwrap()
            .result;
        assert!(extract_summary(&result.history).ends_with("sentinel\nnext step"));
        assert!(
            result
                .history
                .iter()
                .any(|i| text(i).contains("live-state sentinel"))
        );
        assert_eq!(
            model.requests.lock().unwrap().len(),
            if forced { 3 } else { 2 }
        );
        assert_eq!(result.usage.requests, 2);
        assert_eq!(
            result.history_provenance[1],
            ItemProvenance::Agent {
                name: "context-summary".into()
            }
        );
    }
}
struct PrimaryFailure;
impl Model for PrimaryFailure {
    fn retry_advice(&self, _: &Error) -> Option<ModelRetryAdvice> {
        Some(ModelRetryAdvice {
            should_retry: true,
            retry_after: Duration::ZERO,
            reason: "overloaded".into(),
        })
    }
    fn provider(&self) -> &str {
        "primary"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        assert!(
            !request.instructions.starts_with("You are compacting"),
            "summary must use the actual fallback binding even when model names match"
        );
        Box::pin(async { Err(Error::new(ErrorCategory::Provider, "primary unavailable")) })
    }
}
#[tokio::test]
async fn forced_summary_uses_active_fallback_binding_not_a_model_name_lookup() {
    let fallback = ModelImpl::new(Summary::Good, true);
    let mut agent = AgentConfig::new(
        "agent",
        ModelBinding::complete("active", Arc::new(PrimaryFailure)),
    );
    agent
        .fallbacks
        .push(ModelBinding::complete("active", fallback.clone()));
    let runner = Runner::new(agent, config(true)).unwrap();
    let mut request = request();
    request.policy.max_turns = NonZeroU32::new(3).unwrap();
    let result = runner
        .run(context(), request, Arc::new(HostImpl))
        .await
        .unwrap()
        .result;
    assert_eq!(fallback.requests.lock().unwrap().len(), 3);
    assert!(extract_summary(&result.history).ends_with("sentinel\nnext step"));
    assert_eq!(result.usage.requests, 2);
}
#[tokio::test]
async fn disabled_or_unchanged_plan_does_not_call_summary_model() {
    for disabled in [false, true] {
        let model = ModelImpl::new(Summary::Good, false);
        let mut config = config(!disabled);
        config.local_compaction.enabled = !disabled;
        let result = runner(model.clone(), config)
            .run(context(), request(), Arc::new(HostImpl))
            .await
            .unwrap()
            .result;
        assert!(extract_summary(&result.history).is_empty());
        assert_eq!(model.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn normal_summary_uses_new_fallback_model_and_default_thresholds() {
    let fallback = ModelImpl::new(Summary::Good, false);
    let mut agent = AgentConfig::new(
        "agent",
        ModelBinding::complete("gpt-6", Arc::new(PrimaryFailure)),
    );
    agent
        .fallbacks
        .push(ModelBinding::complete("active", fallback.clone()));
    let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
    let mut request = request();
    request.input[1] = message(Role::Assistant, "removed evidence ".repeat(40_000));
    let result = runner
        .run(context(), request, Arc::new(HostImpl))
        .await
        .unwrap()
        .result;
    assert_eq!(fallback.requests.lock().unwrap().len(), 2);
    assert!(extract_summary(&result.history).ends_with("sentinel\nnext step"));
    assert_eq!(result.usage.requests, 2);
}

#[tokio::test]
async fn rejected_summary_usage_can_exhaust_run_budget_before_normal_generation() {
    let model = ModelImpl::new(Summary::Empty, false);
    let mut config = config(false);
    config.limits.max_tokens = Some(10);
    let error = runner(model.clone(), config)
        .run(context(), request(), Arc::new(HostImpl))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Guardrail);
    assert_eq!(error.partial.unwrap().usage.requests, 1);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
}
