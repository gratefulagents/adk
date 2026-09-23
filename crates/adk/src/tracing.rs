//! Owned, shareable trace scopes. Clones share one root; the last owner ends it.
//!
//! Processors run synchronously in registration order and must not panic. Scope
//! completion does not flush or shut down host-owned stores/exporters. Span guards
//! and generation observers retain the root, including during cancellation cleanup.

use crate::tracewriter::{Span, SpanData, Trace, TraceWriter, generation_span};
use adk_core::Context;
use adk_runtime::tracing::{GenerationObserver, GenerationRecord};
use std::sync::{Arc, Mutex};

/// Synchronous lifecycle sink. Empty composition is a no-op processor.
pub trait TraceProcessor: Send + Sync {
    fn trace_start(&self, _trace: &Trace) {}
    fn trace_end(&self, _trace: &Trace) {}
    fn span_start(&self, _span: &Span) {}
    fn span_end(&self, _span: &Span) {}
    /// Observational conversion failures do not change the run outcome.
    fn error(&self, _message: &str) {}
}

impl TraceProcessor for TraceWriter {
    fn trace_start(&self, trace: &Trace) {
        self.trace_start(trace);
    }
    fn trace_end(&self, trace: &Trace) {
        self.trace_end(trace);
    }
    fn span_start(&self, span: &Span) {
        self.span_start(span);
    }
    fn span_end(&self, span: &Span) {
        self.span_end(span);
    }
    fn error(&self, message: &str) {
        self.record_error(message);
    }
}

#[cfg(feature = "otel")]
impl TraceProcessor for crate::telemetry::SpanProcessor {
    fn trace_start(&self, trace: &Trace) {
        self.on_trace_start(trace);
    }
    fn trace_end(&self, trace: &Trace) {
        self.on_trace_end(trace);
    }
    fn span_start(&self, span: &Span) {
        self.on_span_start(span);
    }
    fn span_end(&self, span: &Span) {
        self.on_span_end(span);
    }
}

/// Ordered fanout; processors and their storage/exporter lifetimes remain host-owned.
#[derive(Default)]
pub struct CompositeTraceProcessor(pub Vec<Arc<dyn TraceProcessor>>);
impl TraceProcessor for CompositeTraceProcessor {
    fn trace_start(&self, trace: &Trace) {
        for p in &self.0 {
            p.trace_start(trace);
        }
    }
    fn trace_end(&self, trace: &Trace) {
        for p in &self.0 {
            p.trace_end(trace);
        }
    }
    fn span_start(&self, span: &Span) {
        for p in &self.0 {
            p.span_start(span);
        }
    }
    fn span_end(&self, span: &Span) {
        for p in &self.0 {
            p.span_end(span);
        }
    }
    fn error(&self, message: &str) {
        for p in &self.0 {
            p.error(message);
        }
    }
}

struct Shared {
    trace: Mutex<Trace>,
    processor: Arc<dyn TraceProcessor>,
}
impl Drop for Shared {
    fn drop(&mut self) {
        let trace = self.trace.get_mut().expect("trace scope poisoned");
        trace.finish();
        self.processor.trace_end(trace);
    }
}

/// Shared ownership, not a new trace per clone. The final owner closes the root.
#[derive(Clone)]
pub struct TraceSession(Arc<Shared>);
impl TraceSession {
    pub fn new(name: impl Into<String>, processor: Arc<dyn TraceProcessor>) -> Self {
        let trace = Trace::new(name);
        processor.trace_start(&trace);
        Self(Arc::new(Shared {
            trace: Mutex::new(trace),
            processor,
        }))
    }
    pub fn snapshot(&self) -> Trace {
        self.0.trace.lock().expect("trace scope poisoned").clone()
    }
    pub fn id(&self) -> String {
        self.0
            .trace
            .lock()
            .expect("trace scope poisoned")
            .id
            .clone()
    }
    /// Release this owner. Existing clones, spans and observers keep the root open.
    pub fn finish(self) {
        drop(self);
    }
    pub fn span(&self, name: impl Into<String>, data: Option<SpanData>) -> SpanGuard {
        self.start(Span::new(name, self.id(), data))
    }
    fn start(&self, span: Span) -> SpanGuard {
        self.0
            .trace
            .lock()
            .expect("trace scope poisoned")
            .add_span(span.clone());
        self.0.processor.span_start(&span);
        SpanGuard {
            session: self.clone(),
            span: Some(span),
        }
    }
    /// Attach a runtime without transferring store/exporter shutdown ownership.
    /// Release the observer (including its Runner configuration) to close the root.
    pub fn generation_observer(&self) -> Arc<dyn GenerationObserver> {
        self.observer(self.id())
    }
    fn observer(&self, parent: String) -> Arc<dyn GenerationObserver> {
        Arc::new(ScopedGenerations {
            session: self.clone(),
            parent,
            active: Mutex::new(std::collections::HashMap::new()),
        })
    }
}

/// A span ends exactly once, on explicit finish or Drop. Children retain the root,
/// but can outlive their parent; processors retain ended-parent contexts as needed.
pub struct SpanGuard {
    session: TraceSession,
    span: Option<Span>,
}
impl SpanGuard {
    pub fn id(&self) -> &str {
        &self.span.as_ref().expect("active span").id
    }
    pub fn data_mut(&mut self) -> &mut Option<SpanData> {
        &mut self.span.as_mut().expect("active span").data
    }
    pub fn child(&self, name: impl Into<String>, data: Option<SpanData>) -> SpanGuard {
        self.session.start(Span::new(name, self.id(), data))
    }
    pub fn generation_observer(&self) -> Arc<dyn GenerationObserver> {
        self.session.observer(self.id().into())
    }
    pub fn finish(mut self) -> Span {
        self.end(true).expect("active span")
    }
    fn end(&mut self, stamp: bool) -> Option<Span> {
        let mut span = self.span.take()?;
        if stamp {
            span.finish();
        }
        {
            let mut trace = self.session.0.trace.lock().expect("trace scope poisoned");
            let saved = trace
                .spans
                .iter_mut()
                .find(|s| s.id == span.id)
                .expect("registered span");
            *saved = span.clone();
        }
        self.session.0.processor.span_end(&span);
        Some(span)
    }
}
impl Drop for SpanGuard {
    fn drop(&mut self) {
        self.end(true);
    }
}

struct ScopedGenerations {
    session: TraceSession,
    parent: String,
    active: Mutex<std::collections::HashMap<String, SpanGuard>>,
}
impl GenerationObserver for ScopedGenerations {
    fn start(&self, context: &Context, record: &GenerationRecord) {
        let mut span = generation_span(context, record);
        span.parent_id.clone_from(&self.parent);
        let guard = self.session.start(span);
        self.active
            .lock()
            .expect("generation scope poisoned")
            .insert(record.id.clone(), guard);
    }
    fn end(&self, context: &Context, record: &GenerationRecord) {
        let guard = self
            .active
            .lock()
            .expect("generation scope poisoned")
            .remove(&record.id);
        if let Some(mut guard) = guard {
            let (mut span, error) = crate::tracewriter::generation_end_span(context, record);
            span.parent_id.clone_from(&self.parent);
            if let Some(error) = error {
                self.session.0.processor.error(&error);
            }
            guard.span = Some(span);
            guard.end(false);
        }
    }
}
