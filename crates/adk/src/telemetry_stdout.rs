use opentelemetry::{
    Array, Context, KeyValue, Value,
    trace::{Span as _, SpanContext, SpanId, SpanKind, Status, TraceContextExt, TraceId},
};
use opentelemetry_sdk::{
    Resource,
    error::{OTelSdkError, OTelSdkResult},
    trace::{BatchConfigBuilder, BatchSpanProcessor, Span, SpanData, SpanExporter, SpanProcessor},
};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct};
use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ContextDocument {
    #[serde(rename = "TraceID")]
    trace_id: String,
    #[serde(rename = "SpanID")]
    span_id: String,
    trace_flags: String,
    trace_state: String,
    remote: bool,
}
impl From<&SpanContext> for ContextDocument {
    fn from(context: &SpanContext) -> Self {
        Self {
            trace_id: context.trace_id().to_string(),
            span_id: context.span_id().to_string(),
            trace_flags: format!("{:02x}", context.trace_flags().to_u8()),
            trace_state: context.trace_state().header(),
            remote: context.is_remote(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    parent: ContextDocument,
    children: u64,
}

#[derive(Debug)]
pub(super) struct JsonStdoutProcessor {
    batch: BatchSpanProcessor,
    active: Mutex<HashMap<(TraceId, SpanId), Metadata>>,
}
impl JsonStdoutProcessor {
    pub(super) fn new(writer: impl Write + Send + 'static) -> Self {
        let exporter = JsonExporter {
            writer: Mutex::new(Box::new(writer)),
            resource: Resource::builder_empty().build(),
        };
        let batch = BatchSpanProcessor::builder(exporter)
            .with_batch_config(
                BatchConfigBuilder::default()
                    .with_scheduled_delay(Duration::from_secs(5))
                    .build(),
            )
            .build();
        Self {
            batch,
            active: Mutex::new(HashMap::new()),
        }
    }
}
impl SpanProcessor for JsonStdoutProcessor {
    fn on_start(&self, span: &mut Span, context: &Context) {
        let Some(data) = span.exported_data() else {
            return;
        };
        let parent = context.span().span_context().clone();
        let parent = if data.parent_span_id == parent.span_id() {
            parent
        } else {
            SpanContext::empty_context()
        };
        let mut active = self.active.lock().expect("stdout processor poisoned");
        if let Some(metadata) = active.get_mut(&(parent.trace_id(), parent.span_id())) {
            metadata.children += 1;
        }
        active.insert(
            (
                span.span_context().trace_id(),
                span.span_context().span_id(),
            ),
            Metadata {
                parent: ContextDocument::from(&parent),
                children: 0,
            },
        );
    }
    fn on_end(&self, mut span: SpanData) {
        let metadata = self
            .active
            .lock()
            .expect("stdout processor poisoned")
            .remove(&(span.span_context.trace_id(), span.span_context.span_id()))
            .expect("recording span was started");
        // Rust's export data omits full parent context and child counts. Carry them
        // in the owned batch envelope, removing it before serializing attributes.
        span.attributes.push(KeyValue::new(
            "adk.stdout.envelope",
            serde_json::to_string(&metadata).expect("context metadata serializes"),
        ));
        self.batch.on_end(span);
    }
    fn force_flush(&self) -> OTelSdkResult {
        self.batch.force_flush()
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.batch.shutdown_with_timeout(timeout)
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.batch.set_resource(resource);
    }
}

struct JsonExporter {
    writer: Mutex<Box<dyn Write + Send>>,
    resource: Resource,
}
impl std::fmt::Debug for JsonExporter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JsonStdoutExporter")
    }
}
impl SpanExporter for JsonExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let mut writer = self.writer.lock().expect("stdout exporter poisoned");
        let mut errors = Vec::new();
        for (index, mut span) in batch.into_iter().enumerate() {
            let metadata = span.attributes.pop().expect("batch envelope is present");
            let Value::String(metadata) = metadata.value else {
                unreachable!("batch envelope is a string")
            };
            let metadata: Metadata =
                serde_json::from_str(metadata.as_str()).expect("batch envelope is valid");
            let result = render(&span, &metadata, &self.resource)
                .and_then(|bytes| writer.write_all(&bytes).map_err(|error| error.to_string()));
            if let Err(error) = result {
                errors.push(format!("failed to encode span {index}: {error}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(OTelSdkError::InternalFailure(errors.join("; ")))
        }
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.resource = resource.clone();
    }
}

struct Attribute<'a>(&'a KeyValue);
impl Serialize for Attribute<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_struct("Attribute", 2)?;
        object.serialize_field("Key", self.0.key.as_str())?;
        object.serialize_field("Value", &AttributeValue(&self.0.value))?;
        object.end()
    }
}
struct AttributeValue<'a>(&'a Value);
impl Serialize for AttributeValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_struct("Value", 2)?;
        match self.0 {
            Value::Bool(value) => {
                object.serialize_field("Type", "BOOL")?;
                object.serialize_field("Value", value)?;
            }
            Value::I64(value) => {
                object.serialize_field("Type", "INT64")?;
                object.serialize_field("Value", value)?;
            }
            Value::F64(value) => {
                if !value.is_finite() {
                    return Err(serde::ser::Error::custom("nonfinite OTel attribute"));
                }
                object.serialize_field("Type", "FLOAT64")?;
                object.serialize_field("Value", value)?;
            }
            Value::String(value) => {
                object.serialize_field("Type", "STRING")?;
                object.serialize_field("Value", value.as_str())?;
            }
            Value::Array(array) => match array {
                Array::Bool(values) => {
                    object.serialize_field("Type", "BOOLSLICE")?;
                    object.serialize_field("Value", values)?;
                }
                Array::I64(values) => {
                    object.serialize_field("Type", "INT64SLICE")?;
                    object.serialize_field("Value", values)?;
                }
                Array::F64(values) => {
                    if values.iter().any(|value| !value.is_finite()) {
                        return Err(serde::ser::Error::custom("nonfinite OTel attribute"));
                    }
                    object.serialize_field("Type", "FLOAT64SLICE")?;
                    object.serialize_field("Value", values)?;
                }
                Array::String(values) => {
                    object.serialize_field("Type", "STRINGSLICE")?;
                    object.serialize_field(
                        "Value",
                        &values
                            .iter()
                            .map(|value| value.as_str())
                            .collect::<Vec<_>>(),
                    )?;
                }
                _ => return Err(serde::ser::Error::custom("unsupported OTel array type")),
            },
            _ => return Err(serde::ser::Error::custom("unsupported OTel value type")),
        }
        object.end()
    }
}
fn attributes(values: &[KeyValue]) -> Option<Vec<Attribute<'_>>> {
    (!values.is_empty()).then(|| values.iter().map(Attribute).collect())
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct EventDocument<'a> {
    name: &'a str,
    attributes: Option<Vec<Attribute<'a>>>,
    dropped_attribute_count: u32,
    time: String,
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct LinkDocument<'a> {
    span_context: ContextDocument,
    attributes: Option<Vec<Attribute<'a>>>,
    dropped_attribute_count: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ScopeDocument<'a> {
    name: &'a str,
    version: &'a str,
    #[serde(rename = "SchemaURL")]
    schema_url: &'a str,
    attributes: Option<Vec<Attribute<'a>>>,
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct StatusDocument<'a> {
    code: &'a str,
    description: &'a str,
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct SpanDocument<'a> {
    name: &'a str,
    span_context: ContextDocument,
    parent: &'a ContextDocument,
    span_kind: u8,
    start_time: String,
    end_time: String,
    attributes: Option<Vec<Attribute<'a>>>,
    events: Option<Vec<EventDocument<'a>>>,
    links: Option<Vec<LinkDocument<'a>>>,
    status: StatusDocument<'a>,
    dropped_attributes: u32,
    dropped_events: u32,
    dropped_links: u32,
    child_span_count: u64,
    resource: Option<Vec<Attribute<'a>>>,
    instrumentation_scope: &'a ScopeDocument<'a>,
    instrumentation_library: &'a ScopeDocument<'a>,
}
fn timestamp(time: SystemTime) -> Result<String, String> {
    use chrono::Datelike;
    let nanos = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    let seconds =
        i64::try_from(nanos.div_euclid(1_000_000_000)).map_err(|error| error.to_string())?;
    let time = chrono::DateTime::<chrono::Utc>::from_timestamp(
        seconds,
        nanos.rem_euclid(1_000_000_000) as u32,
    )
    .filter(|time| (0..=9999).contains(&time.year()))
    .ok_or("timestamp outside Go JSON range")?;
    let mut value = time.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    value.pop();
    while value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    value.push('Z');
    Ok(value)
}
fn render(span: &SpanData, metadata: &Metadata, resource: &Resource) -> Result<Vec<u8>, String> {
    let mut resource = resource
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value.clone()))
        .collect::<Vec<_>>();
    resource.sort_by(|a, b| a.key.cmp(&b.key));
    let scope = &span.instrumentation_scope;
    let scope_attributes = scope
        .attributes()
        .map(|attribute| (attribute.key.clone(), attribute.value.clone()))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .map(|(key, value)| KeyValue::new(key, value))
        .collect::<Vec<_>>();
    let scope = ScopeDocument {
        name: scope.name(),
        version: scope.version().unwrap_or(""),
        schema_url: scope.schema_url().unwrap_or(""),
        attributes: attributes(&scope_attributes),
    };
    let document = SpanDocument {
        name: &span.name,
        span_context: ContextDocument::from(&span.span_context),
        parent: &metadata.parent,
        span_kind: match span.span_kind {
            SpanKind::Internal => 1,
            SpanKind::Server => 2,
            SpanKind::Client => 3,
            SpanKind::Producer => 4,
            SpanKind::Consumer => 5,
        },
        start_time: timestamp(span.start_time)?,
        end_time: timestamp(span.end_time)?,
        attributes: attributes(&span.attributes),
        events: if span.events.is_empty() {
            None
        } else {
            Some(
                span.events
                    .iter()
                    .map(|event| {
                        Ok(EventDocument {
                            name: &event.name,
                            attributes: attributes(&event.attributes),
                            dropped_attribute_count: event.dropped_attributes_count,
                            time: timestamp(event.timestamp)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            )
        },
        links: if span.links.is_empty() {
            None
        } else {
            Some(
                span.links
                    .iter()
                    .map(|link| LinkDocument {
                        span_context: ContextDocument::from(&link.span_context),
                        attributes: attributes(&link.attributes),
                        dropped_attribute_count: link.dropped_attributes_count,
                    })
                    .collect(),
            )
        },
        status: match &span.status {
            Status::Unset => StatusDocument {
                code: "Unset",
                description: "",
            },
            Status::Ok => StatusDocument {
                code: "Ok",
                description: "",
            },
            Status::Error { description } => StatusDocument {
                code: "Error",
                description,
            },
        },
        dropped_attributes: span.dropped_attributes_count,
        dropped_events: span.events.dropped_count,
        dropped_links: span.links.dropped_count,
        child_span_count: metadata.children,
        resource: attributes(&resource),
        instrumentation_scope: &scope,
        instrumentation_library: &scope,
    };
    let compact = adk_codec::snapshots::to_go_json(&document).map_err(|error| error.to_string())?;
    Ok(pretty_json(&compact))
}
fn pretty_json(compact: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in compact.iter().copied().enumerate() {
        if quoted {
            output.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => {
                quoted = true;
                output.push(byte);
            }
            b'{' | b'[' => {
                output.push(byte);
                if compact.get(index + 1) != Some(&(if byte == b'{' { b'}' } else { b']' })) {
                    depth += 1;
                    output.push(b'\n');
                    output.extend(std::iter::repeat_n(b'\t', depth));
                }
            }
            b'}' | b']' => {
                if compact.get(index.wrapping_sub(1))
                    != Some(&(if byte == b'}' { b'{' } else { b'[' }))
                {
                    depth -= 1;
                    output.push(b'\n');
                    output.extend(std::iter::repeat_n(b'\t', depth));
                }
                output.push(byte);
            }
            b',' => {
                output.push(byte);
                output.push(b'\n');
                output.extend(std::iter::repeat_n(b'\t', depth));
            }
            b':' => output.extend_from_slice(b": "),
            _ => output.push(byte),
        }
    }
    output.push(b'\n');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::{
        InstrumentationScope,
        trace::{Event, Link, TraceFlags, TraceState},
    };
    use opentelemetry_sdk::trace::{SpanEvents, SpanLinks};

    #[test]
    fn stdout_documents_and_pretty_bytes_match_pinned_go_exporter() {
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-stdout.json"))
                .unwrap();
        let trace_id = TraceId::from_hex("0102030405060708090a0b0c0d0e0f10").unwrap();
        let span_id = SpanId::from_hex("0102030405060708").unwrap();
        let parent_id = SpanId::from_hex("0807060504030201").unwrap();
        let state: TraceState = "vendor=opaque".parse().unwrap();
        let parent = SpanContext::new(
            trace_id,
            parent_id,
            TraceFlags::SAMPLED,
            true,
            state.clone(),
        );
        let context = SpanContext::new(trace_id, span_id, TraceFlags::SAMPLED, false, state);
        let start: SystemTime = "2025-01-02T03:04:05.123456Z"
            .parse::<chrono::DateTime<chrono::FixedOffset>>()
            .unwrap()
            .into();
        let resource = Resource::builder_empty()
            .with_schema_url(
                [KeyValue::new("service.name", "fixture")],
                "https://opentelemetry.io/schemas/1.26.0",
            )
            .build();
        let mut events = SpanEvents::default();
        events.events = vec![Event::new(
            "event",
            start,
            vec![KeyValue::new("event.attr", "value")],
            1,
        )];
        events.dropped_count = 4;
        let mut links = SpanLinks::default();
        links.links = vec![Link::new(
            parent.clone(),
            vec![KeyValue::new("link.attr", "value")],
            2,
        )];
        links.dropped_count = 5;
        let mut span = SpanData {
            span_context: context,
            parent_span_id: parent_id,
            parent_span_is_remote: true,
            span_kind: SpanKind::Client,
            name: "complete".into(),
            start_time: start,
            end_time: start + Duration::from_secs(1),
            attributes: vec![
                KeyValue::new("bool", true),
                KeyValue::new("int", -3_i64),
                KeyValue::new("float", 0.5),
                KeyValue::new("string", "<value>&\u{2028}"),
                KeyValue::new("bools", Value::Array(Array::Bool(vec![true, false]))),
                KeyValue::new("ints", Value::Array(Array::I64(vec![1, 2]))),
                KeyValue::new("floats", Value::Array(Array::F64(vec![0.25, 0.5]))),
                KeyValue::new(
                    "strings",
                    Value::Array(Array::String(vec!["a".into(), "b".into()])),
                ),
            ],
            dropped_attributes_count: 3,
            events,
            links,
            status: Status::Error {
                description: "failure".into(),
            },
            instrumentation_scope: InstrumentationScope::builder("gratefulagents/agent")
                .with_version("test")
                .with_schema_url("scope/schema")
                .with_attributes([
                    KeyValue::new("scope.attr", "old"),
                    KeyValue::new("scope.attr", "value"),
                ])
                .build(),
        };
        let complete = render(
            &span,
            &Metadata {
                parent: ContextDocument::from(&parent),
                children: 2,
            },
            &resource,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&complete).unwrap(),
            expected["records"][0]
        );
        span.name = "root".into();
        span.parent_span_id = SpanId::INVALID;
        span.parent_span_is_remote = false;
        span.span_kind = SpanKind::Internal;
        span.end_time = start;
        span.attributes.clear();
        span.events = SpanEvents::default();
        span.links = SpanLinks::default();
        span.dropped_attributes_count = 0;
        span.status = Status::Unset;
        span.instrumentation_scope = InstrumentationScope::builder("gratefulagents/agent").build();
        let metadata = Metadata {
            parent: ContextDocument::from(&SpanContext::empty_context()),
            children: 0,
        };
        let root = render(&span, &metadata, &resource).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&root).unwrap(),
            expected["records"][1]
        );
        assert_eq!(
            [complete, root].concat(),
            expected["json"].as_str().unwrap().as_bytes()
        );
        span.attributes = vec![KeyValue::new("bad", f64::NAN)];
        assert!(render(&span, &metadata, &resource).is_err());
        span.attributes = vec![KeyValue::new(
            "bad",
            Value::Array(Array::F64(vec![f64::INFINITY])),
        )];
        assert!(render(&span, &metadata, &resource).is_err());
    }

    #[test]
    fn stdout_timestamp_precision_and_pretty_string_boundaries() {
        for text in [
            "2025-01-02T03:04:05Z",
            "2025-01-02T03:04:05.0001Z",
            "1969-12-31T23:59:59.999999999Z",
        ] {
            let time: SystemTime = text
                .parse::<chrono::DateTime<chrono::Utc>>()
                .unwrap()
                .into();
            assert_eq!(timestamp(time).unwrap(), text);
        }
        for value in [
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!({"text":"escaped \" },[]\\ line\n ü", "nested":[{},[],true,false,null]}),
        ] {
            let compact = adk_codec::snapshots::to_go_json(&value).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&pretty_json(&compact)).unwrap(),
                value
            );
        }
    }
}
