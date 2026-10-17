#![cfg(feature = "otel")]

use adk::telemetry::{Destination, Telemetry};
use opentelemetry::{
    Context, KeyValue,
    trace::{
        Span as _, SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, Tracer,
        TracerProvider,
    },
};
use serde_json::Value;
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn stdout_export_tracks_actual_parent_context_and_children_without_leaking_envelope() {
    let buffer = Buffer::default();
    let telemetry = Telemetry::with_stdout_writer("embedded", buffer.clone());
    assert_eq!(telemetry.destination(), Some(&Destination::Stdout));
    let tracer = telemetry.provider().tracer("fixture");
    let remote = SpanContext::new(
        TraceId::from_hex("0102030405060708090a0b0c0d0e0f10").unwrap(),
        SpanId::from_hex("0102030405060708").unwrap(),
        TraceFlags::SAMPLED,
        true,
        "vendor=opaque".parse().unwrap(),
    );
    let context = Context::new().with_remote_span_context(remote);
    let root = tracer.start_with_context("root", &context);
    let root_id = root.span_context().span_id().to_string();
    let context = context.with_span(root);
    let mut child = tracer.start_with_context("child", &context);
    child.set_attribute(KeyValue::new("adk.stdout.envelope", "user attribute"));
    child.end();
    context.span().end();
    let mut late = tracer.start_with_context("late", &context);
    late.end();
    telemetry.force_flush().unwrap();
    telemetry.shutdown().unwrap();
    let bytes = buffer.0.lock().unwrap();
    let records = serde_json::Deserializer::from_slice(&bytes)
        .into_iter::<Value>()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| record["Name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["child", "root", "late"]
    );
    assert_eq!(records[0]["Parent"]["SpanID"], root_id);
    assert_eq!(records[0]["Parent"]["TraceState"], "vendor=opaque");
    assert_eq!(records[0]["Parent"]["Remote"], false);
    assert_eq!(records[1]["Parent"]["SpanID"], "0102030405060708");
    assert_eq!(records[1]["Parent"]["Remote"], true);
    assert_eq!(records[1]["ChildSpanCount"], 1);
    assert_eq!(records[2]["Parent"]["SpanID"], root_id);
    assert_eq!(records[2]["ChildSpanCount"], 0);
    assert_eq!(records[0]["Attributes"].as_array().unwrap().len(), 1);
    assert_eq!(
        records[0]["Attributes"][0]["Value"]["Value"],
        "user attribute"
    );
    assert_eq!(records[1]["Resource"][0]["Key"], "service.name");
    assert_eq!(records[1]["Resource"][0]["Value"]["Value"], "embedded");
}

#[test]
fn stdout_force_flush_reports_writer_failure() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fixture writer failure",
            ))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let telemetry = Telemetry::with_stdout_writer("broken", Broken);
    telemetry.provider().tracer("fixture").start("span").end();
    let error = telemetry
        .force_flush()
        .expect_err("write failure must reach flush caller");
    assert!(format!("{error:?}").contains("fixture writer failure"));
    telemetry.shutdown().unwrap();
}
