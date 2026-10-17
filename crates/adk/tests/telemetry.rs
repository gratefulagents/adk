#![cfg(feature = "otel")]

use adk::{
    observability::{EventRecord, EventSink, ProgressSnapshot},
    telemetry::{Destination, Telemetry},
};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    trace::{SpanData, SpanExporter},
};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct RecordingExporter {
    inner: InMemorySpanExporter,
    resource: Arc<Mutex<Resource>>,
}
impl SpanExporter for RecordingExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        self.inner.export(batch).await
    }
    fn set_resource(&mut self, resource: &Resource) {
        *self.resource.lock().unwrap() = resource.clone();
    }
    fn shutdown(&mut self) -> OTelSdkResult {
        self.inner.shutdown()
    }
}

#[test]
fn pinned_endpoint_normalization_cases() {
    for (explicit, environment, expected) in [
        ("", None, Destination::Stdout),
        (" \t", Some(" \n"), Destination::Stdout),
        (
            "collector:4317",
            None,
            Destination::Grpc {
                authority: "collector:4317".into(),
                secure: false,
            },
        ),
        (
            " http://collector:4317/v1/traces ",
            None,
            Destination::Grpc {
                authority: "collector:4317".into(),
                secure: false,
            },
        ),
        (
            "HTTPS://collector:4317/path",
            None,
            Destination::Grpc {
                authority: "collector:4317".into(),
                secure: true,
            },
        ),
        (
            "",
            Some("https://env:4317/path"),
            Destination::Grpc {
                authority: "env:4317".into(),
                secure: true,
            },
        ),
        (
            "explicit:4317",
            Some("https://ignored:4317"),
            Destination::Grpc {
                authority: "explicit:4317".into(),
                secure: false,
            },
        ),
        (
            "grpc://collector:4317/path",
            None,
            Destination::Grpc {
                authority: "collector:4317".into(),
                secure: false,
            },
        ),
    ] {
        assert_eq!(Destination::resolve(explicit, environment), expected);
    }
}

#[tokio::test]
async fn exporter_resource_instrumentation_flush_and_shutdown() {
    let exporter = InMemorySpanExporter::default();
    let resource = Arc::new(Mutex::new(Resource::builder_empty().build()));
    let telemetry = Telemetry::with_exporter(
        "embedded-adk",
        RecordingExporter {
            inner: exporter.clone(),
            resource: resource.clone(),
        },
    );
    let bridge = telemetry.bridge();
    for (sequence, kind, data) in [
        (1, "agent_start", json!({"agent":"test"})),
        (2, "done", json!({"status":"completed"})),
    ] {
        bridge
            .emit(&EventRecord {
                schema_version: 1,
                run_id: "run".into(),
                sequence,
                timestamp_unix_ms: 1,
                kind: kind.into(),
                data,
                progress: ProgressSnapshot::default(),
            })
            .await
            .unwrap();
    }
    telemetry.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    assert_eq!(spans.len(), 2);
    assert!(
        spans
            .iter()
            .all(|span| span.instrumentation_scope.name() == "gratefulagents/agent")
    );
    let resource = resource.lock().unwrap().clone();
    assert_eq!(
        resource
            .get(&opentelemetry::Key::new("service.name"))
            .unwrap()
            .as_str(),
        "embedded-adk"
    );
    assert_eq!(
        resource.schema_url(),
        Some("https://opentelemetry.io/schemas/1.26.0")
    );
    telemetry.shutdown().unwrap();
    assert!(telemetry.force_flush().is_err());
}

#[tokio::test]
async fn explicit_otlp_constructs_without_collector() {
    let telemetry =
        Telemetry::with_endpoint("offline-construction", "http://127.0.0.1:9/path").unwrap();
    assert_eq!(
        telemetry.destination(),
        Some(&Destination::Grpc {
            authority: "127.0.0.1:9".into(),
            secure: false
        })
    );
    telemetry.shutdown().unwrap();
}

#[test]
fn otlp_without_runtime_returns_error_instead_of_panicking() {
    assert!(Telemetry::with_endpoint("offline", "http://127.0.0.1:9").is_err());
}
