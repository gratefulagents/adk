#![cfg(feature = "otel")]

use adk::telemetry::{Destination, Telemetry};
use opentelemetry::trace::{Span as _, Tracer};
use opentelemetry_sdk::trace::InMemorySpanExporter;

#[test]
fn sdk_endpoint_constructor_installs_global_but_host_exporters_remain_scoped() {
    let mut before = opentelemetry::global::tracer("before").start("before");
    assert!(!before.span_context().is_valid());
    before.end();
    let scoped = Telemetry::with_exporter("scoped", InMemorySpanExporter::default());
    let mut still_scoped = opentelemetry::global::tracer("before").start("before");
    assert!(!still_scoped.span_context().is_valid());
    still_scoped.end();
    scoped.shutdown().unwrap();
    let telemetry = Telemetry::with_endpoint("sdk-default", "http:///").unwrap();
    assert_eq!(telemetry.destination(), Some(&Destination::Stdout));
    let mut registered = opentelemetry::global::tracer("registered").start("registered");
    assert!(registered.span_context().is_valid());
    registered.end();
    telemetry.force_flush().unwrap();
    telemetry.shutdown().unwrap();
}
