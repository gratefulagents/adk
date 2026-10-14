//! Synchronous, observational generation spans with owned cancellation cleanup.

use adk_core::{Context, ErrorInfo, ModelRequest, ModelResponse};
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

#[derive(Debug, Clone)]
pub struct GenerationRecord {
    pub id: String,
    pub agent: String,
    pub provider: String,
    pub resolved_model: String,
    pub input_tokens_include_cache: Option<bool>,
    pub task_id: Option<String>,
    pub cost_usd: Option<f64>,
    pub turn: u32,
    pub request: ModelRequest,
    pub declared_tool_timeouts: Vec<Option<Duration>>,
    pub request_snapshot:
        Result<adk_codec::snapshots::RequestSnapshot, adk_codec::approval::BridgeError>,
    pub response: Option<ModelResponse>,
    pub error: Option<ErrorInfo>,
    pub retry_reason: Option<String>,
    pub status: GenerationStatus,
    pub retry_after: Option<Duration>,
    pub fallback_model: Option<String>,
    pub started_at: SystemTime,
    pub ended_at: Option<SystemTime>,
    pub latency: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationStatus {
    Started,
    Completed,
    Failed,
    Retrying,
    Fallback,
    Interrupted,
}

/// These observers must not panic, detach work, or alter execution policy.
/// The runtime closes every started record, including when its future is dropped.
pub trait GenerationObserver: Send + Sync {
    fn start(&self, context: &Context, record: &GenerationRecord);
    fn end(&self, context: &Context, record: &GenerationRecord);
    /// Nonempty reasoning surfaced by opted-in blocking runs only, never by streamed runs.
    /// Delivery commits the attempt against retry and immediate-input replacement.
    fn reasoning_delta(&self, _context: &Context, _record: &GenerationRecord, _delta: &str) {}
    /// Flush pending content once before `end`, including cancellation and future drop.
    /// The record need not contain the final response, error, or retry decision yet.
    fn returned(&self, _context: &Context, _record: &GenerationRecord) {}
    /// Successful record after `end`, immediately before the `ModelAccepted` hooks.
    /// Retained responses from failed attempts never reach this callback.
    fn accepted(&self, _context: &Context, _record: &GenerationRecord) {}
}

pub struct CompositeGenerationObserver {
    observers: Vec<Arc<dyn GenerationObserver>>,
}
impl CompositeGenerationObserver {
    pub fn new(observers: Vec<Arc<dyn GenerationObserver>>) -> Self {
        Self { observers }
    }
}
impl GenerationObserver for CompositeGenerationObserver {
    fn start(&self, context: &Context, record: &GenerationRecord) {
        for observer in &self.observers {
            observer.start(context, record);
        }
    }
    fn end(&self, context: &Context, record: &GenerationRecord) {
        for observer in &self.observers {
            observer.end(context, record);
        }
    }
    fn reasoning_delta(&self, context: &Context, record: &GenerationRecord, delta: &str) {
        for observer in &self.observers {
            observer.reasoning_delta(context, record, delta);
        }
    }
    fn returned(&self, context: &Context, record: &GenerationRecord) {
        for observer in &self.observers {
            observer.returned(context, record);
        }
    }
    fn accepted(&self, context: &Context, record: &GenerationRecord) {
        for observer in &self.observers {
            observer.accepted(context, record);
        }
    }
}

pub(crate) struct GenerationGuard {
    observer: Arc<dyn GenerationObserver>,
    context: Context,
    pub record: GenerationRecord,
    started: Instant,
    finished: bool,
}
impl GenerationGuard {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        observer: Arc<dyn GenerationObserver>,
        context: &Context,
        agent: &str,
        info: adk_core::ModelInfo,
        task_id: Option<&str>,
        turn: u32,
        request: &ModelRequest,
        declared_tool_timeouts: Vec<Option<Duration>>,
        request_snapshot: Result<
            adk_codec::snapshots::RequestSnapshot,
            adk_codec::approval::BridgeError,
        >,
    ) -> Self {
        let guard = Self {
            observer,
            context: context.clone(),
            record: GenerationRecord {
                id: uuid::Uuid::new_v4().to_string(),
                agent: agent.into(),
                provider: info.provider,
                resolved_model: info.model,
                input_tokens_include_cache: info.input_tokens_include_cache,
                task_id: task_id.map(str::to_owned),
                cost_usd: None,
                turn,
                request: request.clone(),
                declared_tool_timeouts,
                request_snapshot,
                response: None,
                error: None,
                retry_reason: None,
                status: GenerationStatus::Started,
                retry_after: None,
                fallback_model: None,
                started_at: SystemTime::now(),
                ended_at: None,
                latency: Duration::ZERO,
            },
            started: Instant::now(),
            finished: false,
        };
        guard.observer.start(&guard.context, &guard.record);
        guard
    }
    pub fn reasoning_delta(&self, delta: &str) {
        self.observer
            .reasoning_delta(&self.context, &self.record, delta);
    }
    pub fn returned(&mut self) {
        if self.record.ended_at.is_some() {
            return;
        }
        self.record.ended_at = Some(SystemTime::now());
        self.record.latency = self.started.elapsed();
        self.observer.returned(&self.context, &self.record);
    }
    pub fn finish(&mut self, status: GenerationStatus) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.record.status = status;
        if self.record.ended_at.is_none() {
            self.returned();
        }
        self.observer.end(&self.context, &self.record);
    }
}
impl Drop for GenerationGuard {
    fn drop(&mut self) {
        self.finish(if self.record.error.is_some() {
            GenerationStatus::Failed
        } else {
            GenerationStatus::Interrupted
        });
    }
}
