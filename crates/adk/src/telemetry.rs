//! Host-owned telemetry exporters with the SDK endpoint-selection defaults.

#[path = "telemetry_spans.rs"]
mod spans;
pub use spans::SpanProcessor;
#[path = "telemetry_stdout.rs"]
mod stdout;

use crate::observability::otel::OtelBridge;
use adk_core::{Error, ErrorCategory};
use opentelemetry::{Context, KeyValue, trace::TracerProvider};
use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::{
    Resource,
    trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracer, SdkTracerProvider, SpanExporter},
};
use std::{sync::Arc, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    Stdout,
    Grpc { authority: String, secure: bool },
}
impl Destination {
    /// An empty explicit endpoint falls back to the supplied environment value.
    pub fn resolve(endpoint: &str, environment: Option<&str>) -> Self {
        let endpoint = if endpoint.trim().is_empty() {
            environment.unwrap_or("")
        } else {
            endpoint
        };
        let endpoint = endpoint.trim();
        let (authority, secure) = match endpoint.split_once("://") {
            Some((scheme, authority)) => (authority, scheme.eq_ignore_ascii_case("https")),
            None => (endpoint, false),
        };
        let authority = authority.split('/').next().unwrap_or("");
        if authority.is_empty() {
            Self::Stdout
        } else {
            Self::Grpc {
                authority: authority.into(),
                secure,
            }
        }
    }
}

/// Shutdown must occur after all attached observation pipelines have finished.
/// SDK endpoint constructors install globally; host-supplied exporters remain scoped.
pub struct Telemetry {
    provider: SdkTracerProvider,
    bridge: Arc<OtelBridge<SdkTracer>>,
    destination: Option<Destination>,
    spans: Arc<SpanProcessor>,
}
impl Telemetry {
    pub fn new(service_name: &str) -> Result<Self, Error> {
        Self::with_endpoint(service_name, "")
    }

    /// Application-level SDK constructor: replaces the global tracer provider.
    /// OTLP construction requires an entered Tokio runtime; network delivery is asynchronous.
    pub fn with_endpoint(service_name: &str, endpoint: &str) -> Result<Self, Error> {
        let destination = Destination::resolve(
            endpoint,
            std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok().as_deref(),
        );
        let mut telemetry = match &destination {
            Destination::Stdout => Self::with_stdout_writer(service_name, std::io::stdout()),
            Destination::Grpc { authority, secure } => {
                if tokio::runtime::Handle::try_current().is_err() {
                    return Err(Error::new(
                        ErrorCategory::InvalidInput,
                        "OTLP exporter requires an entered Tokio runtime",
                    ));
                }
                let mut builder = opentelemetry_otlp::SpanExporter::builder()
                    .with_tonic()
                    .with_endpoint(format!(
                        "{}://{authority}",
                        if *secure { "https" } else { "http" }
                    ));
                if *secure {
                    builder = builder.with_tls_config(
                        opentelemetry_otlp::tonic_types::transport::ClientTlsConfig::new()
                            .with_native_roots(),
                    );
                }
                let exporter = builder.build().map_err(|error| {
                    Error::new(ErrorCategory::Host, "create OTLP exporter").with_source(error)
                })?;
                Self::with_exporter(service_name, exporter)
            }
        };
        telemetry.destination = Some(destination);
        telemetry.install_global();
        Ok(telemetry)
    }
    pub fn with_exporter(service_name: &str, exporter: impl SpanExporter + 'static) -> Self {
        let processor = BatchSpanProcessor::builder(exporter)
            .with_batch_config(
                BatchConfigBuilder::default()
                    .with_scheduled_delay(Duration::from_secs(5))
                    .build(),
            )
            .build();
        Self::with_processor(service_name, processor)
    }
    pub fn with_stdout_writer(
        service_name: &str,
        writer: impl std::io::Write + Send + 'static,
    ) -> Self {
        let mut telemetry =
            Self::with_processor(service_name, stdout::JsonStdoutProcessor::new(writer));
        telemetry.destination = Some(Destination::Stdout);
        telemetry
    }
    fn with_processor(
        service_name: &str,
        processor: impl opentelemetry_sdk::trace::SpanProcessor + 'static,
    ) -> Self {
        let resource = Resource::builder_empty()
            .with_schema_url(
                [KeyValue::new("service.name", service_name.to_owned())],
                "https://opentelemetry.io/schemas/1.26.0",
            )
            .build();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(processor)
            .with_resource(resource)
            .build();
        let bridge = Arc::new(OtelBridge::new(
            provider.tracer("gratefulagents/agent"),
            Context::new(),
        ));
        let spans = Arc::new(SpanProcessor::new(provider.tracer("gratefulagents/agent")));
        Self {
            spans,
            provider,
            bridge,
            destination: None,
        }
    }
    pub fn span_processor(&self) -> Arc<SpanProcessor> {
        self.spans.clone()
    }
    pub fn bridge(&self) -> Arc<OtelBridge<SdkTracer>> {
        self.bridge.clone()
    }
    pub fn provider(&self) -> &SdkTracerProvider {
        &self.provider
    }
    pub fn destination(&self) -> Option<&Destination> {
        self.destination.as_ref()
    }
    pub fn install_global(&self) {
        opentelemetry::global::set_tracer_provider(self.provider.clone());
    }
    pub fn force_flush(&self) -> Result<(), Error> {
        self.provider
            .force_flush()
            .map_err(|error| Error::new(ErrorCategory::Host, "flush telemetry").with_source(error))
    }
    pub fn shutdown(&self) -> Result<(), Error> {
        self.provider.shutdown().map_err(|error| {
            Error::new(ErrorCategory::Host, "shutdown telemetry").with_source(error)
        })
    }
}
