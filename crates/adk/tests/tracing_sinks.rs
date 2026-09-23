#![cfg(all(feature = "otel", target_os = "linux"))]
use adk::{
    telemetry::Telemetry,
    tracestore::{FilesystemTraceStore, RunMetadata},
    tracewriter::{Options, TraceWriter},
    tracing::{CompositeTraceProcessor, TraceSession},
};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use std::sync::Arc;

#[test]
fn scoped_writer_and_otel_share_parentage_and_leave_exporter_host_owned() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FilesystemTraceStore::new(directory.path()).unwrap());
    let writer = Arc::new(TraceWriter::new(store, "run", Options::default()));
    let path = writer
        .init_run(&RunMetadata {
            run_id: "run".into(),
            ..Default::default()
        })
        .unwrap();
    let exporter = InMemorySpanExporter::default();
    let telemetry = Telemetry::with_exporter("scope", exporter.clone());
    let processor = Arc::new(CompositeTraceProcessor(vec![
        writer.clone(),
        telemetry.span_processor(),
    ]));
    let trace = TraceSession::new("root", processor);
    let parent = trace.span("parent", None);
    let child = parent.child("child", None);
    trace.finish();
    parent.finish();
    child.finish();
    telemetry.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    assert_eq!(spans.len(), 3);
    let root = spans.iter().find(|s| s.name == "root").unwrap();
    let parent = spans.iter().find(|s| s.name == "parent").unwrap();
    let child = spans.iter().find(|s| s.name == "child").unwrap();
    assert_eq!(parent.parent_span_id, root.span_context.span_id());
    assert_eq!(child.parent_span_id, parent.span_context.span_id());
    assert_eq!(child.span_context.trace_id(), root.span_context.trace_id());
    let text = std::fs::read_to_string(path.join("spans.jsonl")).unwrap();
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.first().unwrap()["type"], "trace_start");
    assert_eq!(records.last().unwrap()["type"], "trace_end");
    assert_eq!(writer.health().write_errors, 0);
    telemetry.shutdown().unwrap();
}

#[test]
fn ending_overlapping_root_does_not_remove_other_roots_parent_contexts() {
    let exporter = InMemorySpanExporter::default();
    let telemetry = Telemetry::with_exporter("overlap", exporter.clone());
    let processor = telemetry.span_processor();
    let a = TraceSession::new("A", processor.clone());
    let parent = a.span("A-parent", None);
    let b = TraceSession::new("B", processor.clone());
    let b_snapshot = b.snapshot();
    b.span("B-child", None).finish();
    b.finish();
    // Repeated completion must not evict another trace either.
    processor.on_trace_end(&b_snapshot);
    parent.child("A-late-child", None).finish();
    parent.finish();
    a.span("A-late-root-child", None).finish();
    a.finish();
    telemetry.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    let find = |name: &str| spans.iter().find(|span| span.name == name).unwrap();
    let a = find("A");
    let b = find("B");
    assert_ne!(a.span_context.trace_id(), b.span_context.trace_id());
    assert_eq!(
        find("A-late-child").parent_span_id,
        find("A-parent").span_context.span_id()
    );
    assert_eq!(
        find("A-late-root-child").parent_span_id,
        a.span_context.span_id()
    );
    for name in ["A-parent", "A-late-child", "A-late-root-child"] {
        assert_eq!(
            find(name).span_context.trace_id(),
            a.span_context.trace_id()
        );
    }
    assert_eq!(
        find("B-child").span_context.trace_id(),
        b.span_context.trace_id()
    );
    telemetry.shutdown().unwrap();
}
