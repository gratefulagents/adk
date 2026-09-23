//! Owned, pull-based HTTP streams with bounded auth/shape recovery and no transient replay.
use crate::{
    auth::{AuthMode, Session},
    error::RequestFailure,
    sse::Decoder,
    wire::{self, Protocol},
};
use adk_core::{
    BoxFuture, Context, Error, ErrorCategory, Model, ModelEvent, ModelRequest, ModelResponse,
    ModelStream, StreamingModel,
};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::SystemTime,
};

pub struct Provider {
    name: String,
    protocol: Protocol,
    session: Arc<Session>,
    client: Client,
    thinking_overrides: Mutex<BTreeMap<String, (String, bool)>>,
}
impl Provider {
    pub fn new(
        name: impl Into<String>,
        protocol: Protocol,
        session: Arc<Session>,
    ) -> Result<Self, Error> {
        if session.scope().mode == AuthMode::OpenAiOAuth && protocol != Protocol::Responses {
            return Err(crate::invalid("OpenAI OAuth requires Responses protocol"));
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| {
                Error::new(
                    ErrorCategory::Provider,
                    "cannot initialize provider transport",
                )
            })?;
        Ok(Self {
            name: name.into(),
            protocol,
            session,
            client,
            thinking_overrides: Mutex::new(BTreeMap::new()),
        })
    }
    /// Explicit native Responses compaction. No local summary is substituted when
    /// the selected protocol or endpoint does not support this operation.
    pub async fn compact(
        &self,
        context: &Context,
        request: ModelRequest,
    ) -> Result<ModelResponse, Error> {
        if self.protocol != Protocol::Responses {
            return Err(Error::new(
                ErrorCategory::Unsupported,
                "native compaction requires Responses protocol",
            ));
        }
        let source = wire::request(&request, self.protocol, false)?;
        let codex = self.session.scope().mode == AuthMode::OpenAiOAuth;
        let mut body = json!({"model":source["model"],"input":source["input"]});
        if !codex || !request.instructions.is_empty() {
            body["instructions"] = source["instructions"].clone();
        }
        if codex {
            body["tools"] = source.get("tools").cloned().unwrap_or_else(|| json!([]));
            body["parallel_tool_calls"] = (!request.tools.is_empty()).into();
            if let Some(value) = source.get("reasoning") {
                let mut reasoning = value.clone();
                if let Some(object) = reasoning.as_object_mut() {
                    object.remove("summary");
                }
                body["reasoning"] = reasoning;
            }
            if let Some(verbosity) = source.pointer("/text/verbosity") {
                body["text"] = json!({"verbosity":verbosity});
            }
        }
        let response = self.send_body(context, "/responses/compact", body).await?;
        let result = wire::response(&read_json(context, response).await?, self.protocol)?;
        if !result.items.iter().any(|item| matches!(item, adk_core::RunItem::Compaction { compaction } if !compaction.encrypted_content.trim().is_empty())) {
            return Err(protocol_error("native compaction response has no encrypted continuation"));
        }
        Ok(result)
    }
    async fn send(
        &self,
        context: &Context,
        request: &ModelRequest,
        stream: bool,
    ) -> Result<reqwest::Response, Error> {
        let mut body = wire::request(request, self.protocol, stream)?;
        let scope = self.session.scope();
        if scope.mode == AuthMode::OpenAiOAuth {
            crate::openai::codex(&mut body);
        }
        if scope.mode == AuthMode::CopilotOAuth && self.protocol == Protocol::Chat {
            crate::copilot::shape_chat(&mut body, request);
        }
        if self.protocol == Protocol::Anthropic {
            crate::anthropic::shape(&mut body, request, scope.mode);
        }
        self.send_body(context, self.protocol.path(), body).await
    }
    async fn send_body(
        &self,
        context: &Context,
        path: &str,
        body: Value,
    ) -> Result<reqwest::Response, Error> {
        let scope = self.session.scope();
        let mut body = body;
        let model = body["model"].as_str().unwrap_or_default().to_owned();
        if self.protocol == Protocol::Anthropic
            && let Some((kind, cap)) = self.thinking_overrides.lock().unwrap().get(&model).cloned()
        {
            if body["thinking"]["type"]
                .as_str()
                .is_some_and(|value| value != kind)
            {
                repair(&mut body, self.protocol, "thinking.type", &mut false);
            }
            if cap {
                repair(&mut body, self.protocol, "effort", &mut false);
            }
        }
        let mut repaired_anthropic = false;
        let mut rejected = false;
        let mut flipped = false;
        for _ in 0..5 {
            let material = self.session.material_for_request(context).await?;
            let mut resolved = scope.clone();
            if scope.mode == AuthMode::CopilotOAuth {
                resolved.endpoint = crate::copilot::request_endpoint(
                    &scope.endpoint,
                    material.access_token.expose(),
                );
                if self.protocol == Protocol::Anthropic {
                    resolved.endpoint = resolved
                        .endpoint
                        .trim_end_matches("/chat/completions")
                        .trim_end_matches("/v1")
                        .to_owned();
                }
            }
            let endpoint = format!("{}{path}", resolved.endpoint);
            let mut outgoing = body.clone();
            if let Some(key) = outgoing.get("prompt_cache_key").and_then(Value::as_str) {
                let key = crate::auth::cache_scope(&resolved, &material, key);
                if key.is_empty() {
                    outgoing.as_object_mut().unwrap().remove("prompt_cache_key");
                } else {
                    outgoing["prompt_cache_key"] = key.into();
                }
            }
            let mut headers =
                crate::auth::headers(&resolved, &material, self.protocol == Protocol::Anthropic)?;
            if self.protocol == Protocol::Anthropic && scope.mode != AuthMode::CopilotOAuth {
                let betas = crate::anthropic::beta(
                    body["model"].as_str().unwrap_or_default(),
                    scope.mode == AuthMode::AnthropicOAuth,
                );
                headers.insert(
                    "anthropic-beta",
                    reqwest::header::HeaderValue::from_str(&betas)
                        .map_err(|_| crate::invalid("invalid beta header"))?,
                );
            }
            let response = crate::active(
                context,
                self.client
                    .post(&endpoint)
                    .headers(headers)
                    .json(&outgoing)
                    .send(),
            )
            .await?
            .map_err(|e| RequestFailure::transport(&e).into_error())?;
            if response.status().as_u16() == 401
                && !rejected
                && matches!(
                    scope.mode,
                    AuthMode::OpenAiOAuth | AuthMode::AnthropicOAuth | AuthMode::CopilotOAuth
                )
            {
                rejected = true;
                drop(response);
                self.session.reject(context, &material.access_token).await?;
                continue;
            }
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let failure = RequestFailure::http(status, response.headers(), SystemTime::now());
                if status == 400 {
                    let bytes = read_bytes(context, response).await?;
                    if !repaired_anthropic
                        && repair(
                            &mut body,
                            self.protocol,
                            &String::from_utf8_lossy(&bytes),
                            &mut flipped,
                        )
                    {
                        if self.protocol == Protocol::Anthropic {
                            repaired_anthropic = true;
                            let mut overrides = self.thinking_overrides.lock().unwrap();
                            let previous_cap = overrides.get(&model).is_some_and(|(_, cap)| *cap);
                            overrides.insert(
                                model.clone(),
                                (
                                    body["thinking"]["type"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .to_owned(),
                                    previous_cap || !flipped,
                                ),
                            );
                        }
                        continue;
                    }
                }
                return Err(failure.into_error());
            }
            return Ok(response);
        }
        Err(Error::new(
            ErrorCategory::PermissionDenied,
            "provider authentication retry exhausted",
        ))
    }
}
impl Model for Provider {
    fn info(&self, model: &str) -> adk_core::ModelInfo {
        adk_core::ModelInfo {
            provider: self.name.clone(),
            model: model.into(),
            input_tokens_include_cache: Some(self.protocol != Protocol::Anthropic),
        }
    }
    fn provider(&self) -> &str {
        &self.name
    }
    fn retry_advice(&self, error: &Error) -> Option<adk_core::ModelRetryAdvice> {
        crate::error::retry_advice(error)
    }
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            if self.session.scope().mode == AuthMode::OpenAiOAuth
                || (self.session.scope().mode == AuthMode::CopilotOAuth
                    && self.protocol == Protocol::Chat)
            {
                let mut stream = self.stream(context, request).await?;
                let mut complete = None;
                while let Some(event) = stream.next().await? {
                    if let ModelEvent::Complete { response } = event {
                        complete = Some(response);
                    }
                }
                let mut response =
                    complete.ok_or_else(|| protocol_error("stream ended without completion"))?;
                if let Some(raw) = &response.raw {
                    // SDK non-stream calls drain the transport stream into a message,
                    // rather than exposing StreamAssembler's empty Type field.
                    response.snapshot_raw =
                        Some(crate::snapshot::document(raw, self.protocol, None)?);
                }
                return Ok(response);
            }
            let response = self.send(context, &request, false).await?;
            wire::response_json(&read_bytes(context, response).await?, self.protocol)
        })
    }
}
impl StreamingModel for Provider {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let response = self.send(context, &request, true).await?;
            Ok(Box::new(HttpStream {
                context: context.clone(),
                response: Some(response),
                decoder: Decoder::default(),
                state: StreamState::new(self.protocol),
                queue: VecDeque::new(),
                closed: false,
            }) as Box<dyn ModelStream>)
        })
    }
}
async fn read_json(context: &Context, response: reqwest::Response) -> Result<Value, Error> {
    serde_json::from_slice(&read_bytes(context, response).await?)
        .map_err(|_| protocol_error("invalid provider response JSON"))
}
async fn read_bytes(context: &Context, mut response: reqwest::Response) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = crate::active(context, response.chunk())
        .await?
        .map_err(|e| RequestFailure::transport(&e).into_error())?
    {
        if bytes.len().saturating_add(chunk.len()) > 16 * 1024 * 1024 {
            return Err(protocol_error("provider response exceeds limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn repair(body: &mut Value, protocol: Protocol, error: &str, flipped: &mut bool) -> bool {
    use crate::error::RequestRepair;
    match crate::error::request_repair(400, error) {
        Some(RequestRepair::ThinkingType) if protocol == Protocol::Anthropic && !*flipped => {
            let kind = body["thinking"]["type"].as_str().unwrap_or_default();
            if !matches!(kind, "adaptive" | "enabled") {
                return false;
            }
            if kind == "adaptive" {
                let budget = match body["output_config"]["effort"].as_str() {
                    Some("low") => 2048,
                    Some("high") => 8192,
                    Some("max" | "xhigh") => 24576,
                    _ => 4096,
                }
                .min(
                    body["max_tokens"]
                        .as_u64()
                        .unwrap_or(16384)
                        .saturating_sub(1024),
                );
                if budget < 1024 {
                    return false;
                }
                body["thinking"] = json!({"type":"enabled","budget_tokens":budget});
                if let Some(config) = body["output_config"].as_object_mut() {
                    config.remove("effort");
                }
            } else {
                let budget = body["thinking"]["budget_tokens"].as_u64().unwrap_or(0);
                body["thinking"] = json!({"type":"adaptive","display":"summarized"});
                body["output_config"]["effort"] = match budget {
                    0..=2048 => "low",
                    2049..=4096 => "medium",
                    4097..=8192 => "high",
                    _ => "max",
                }
                .into();
            }
            *flipped = true;
            true
        }
        Some(RequestRepair::AdaptiveEffort | RequestRepair::ReasoningEffort)
            if protocol == Protocol::Anthropic =>
        {
            if body["thinking"]["type"] != "adaptive"
                || !matches!(
                    body["output_config"]["effort"].as_str(),
                    Some("xhigh" | "max")
                )
            {
                return false;
            }
            body["output_config"]["effort"] = "high".into();
            true
        }
        Some(RequestRepair::ReasoningEffort) if protocol != Protocol::Anthropic => {
            let path = if body.get("reasoning_effort").is_some() {
                "/reasoning_effort"
            } else {
                "/reasoning/effort"
            };
            let Some(effort) = body.pointer_mut(path) else {
                return false;
            };
            let next = match effort.as_str() {
                Some("max") => "xhigh",
                Some("xhigh") => "high",
                Some("none") => "minimal",
                Some("minimal") => "low",
                _ => return false,
            };
            *effort = next.into();
            true
        }
        _ => false,
    }
}
fn protocol_error(message: &'static str) -> Error {
    Error::new(ErrorCategory::Provider, message)
}

struct HttpStream {
    context: Context,
    response: Option<reqwest::Response>,
    decoder: Decoder,
    state: StreamState,
    queue: VecDeque<ModelEvent>,
    closed: bool,
}
impl HttpStream {
    async fn read(&mut self) -> Result<Option<ModelEvent>, Error> {
        loop {
            self.context.check_active()?;
            if let Some(event) = self.queue.pop_front() {
                return Ok(Some(event));
            }
            if self.state.complete {
                self.response = None;
                self.closed = true;
                return Ok(None);
            }
            let chunk = crate::active(&self.context, self.response.as_mut().unwrap().chunk())
                .await?
                .map_err(|e| RequestFailure::transport(&e).into_error())?;
            let Some(chunk) = chunk else {
                self.decoder.finish()?;
                return Err(protocol_error(
                    "provider stream ended without terminal event",
                ));
            };
            for event in self.decoder.feed(&chunk)? {
                self.queue.extend(self.state.event(&event.data)?);
            }
        }
    }
}
impl ModelStream for HttpStream {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            if self.closed {
                return Ok(None);
            }
            let result = self.read().await;
            if result.is_err() {
                self.closed = true;
                self.response = None;
                self.queue.clear();
            }
            result
        })
    }
}

/// Protocol state can be tested without sockets. Terminal events are validated
/// here, rather than inferred from transport EOF.
pub struct StreamState {
    protocol: Protocol,
    body: Value,
    tools: BTreeMap<u64, Value>,
    argument_buffers: BTreeMap<u64, String>,
    snapshot_inputs: BTreeMap<u64, String>,
    snapshot_starts: BTreeMap<u64, Value>,
    snapshot_stop_reason: Option<&'static str>,
    message_started: bool,
    text: String,
    reasoning: String,
    complete: bool,
    finish_reason: Option<String>,
    bytes: usize,
    response_call_ids: BTreeMap<String, String>,
    response_deltas: BTreeSet<(u64, u64, bool)>,
    response_items: BTreeMap<u64, Value>,
    response_fragments: BTreeMap<(u64, u64, bool), String>,
    archived_tools: Vec<Value>,
    stopped_blocks: BTreeSet<u64>,
}
impl StreamState {
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            body: json!({}),
            tools: BTreeMap::new(),
            argument_buffers: BTreeMap::new(),
            snapshot_inputs: BTreeMap::new(),
            snapshot_starts: BTreeMap::new(),
            snapshot_stop_reason: None,
            message_started: false,
            text: String::new(),
            reasoning: String::new(),
            complete: false,
            finish_reason: None,
            bytes: 0,
            response_call_ids: BTreeMap::new(),
            response_deltas: BTreeSet::new(),
            response_items: BTreeMap::new(),
            response_fragments: BTreeMap::new(),
            archived_tools: Vec::new(),
            stopped_blocks: BTreeSet::new(),
        }
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }
    pub fn event(&mut self, data: &str) -> Result<Vec<ModelEvent>, Error> {
        if self.complete {
            if self.protocol == Protocol::Responses && data == "[DONE]" {
                return Ok(Vec::new());
            }
            return Err(protocol_error("data after provider completion"));
        }
        self.bytes = self.bytes.saturating_add(data.len());
        if self.bytes > 16 * 1024 * 1024 {
            return Err(protocol_error("provider stream exceeds limit"));
        }
        if data == "[DONE]" {
            if self.protocol != Protocol::Chat || self.finish_reason.is_none() {
                return Err(protocol_error("premature stream terminal marker"));
            }
            self.body["choices"] = json!([{"message":{"content":self.text,"reasoning_content":self.reasoning,
                "reasoning_opaque":self.body["reasoning_opaque"],"reasoning_details":self.body["reasoning_details"],
                "tool_calls":self.archived_tools.iter().chain(self.tools.values()).collect::<Vec<_>>()},"finish_reason":self.finish_reason}]);
            return self.finish();
        }
        let event: Value =
            serde_json::from_str(data).map_err(|_| protocol_error("invalid SSE JSON"))?;
        if self.protocol == Protocol::Responses && event["type"] == "error" {
            return Err(RequestFailure::http(
                502,
                &reqwest::header::HeaderMap::new(),
                SystemTime::now(),
            )
            .into_error());
        }
        if let Some(error) = crate::error::provider_error(&event) {
            return Err(error);
        }
        match self.protocol {
            Protocol::Responses => self.responses(event),
            Protocol::Chat => self.chat(event),
            Protocol::Anthropic => self.anthropic(event),
        }
    }
    fn responses(&mut self, event: Value) -> Result<Vec<ModelEvent>, Error> {
        let index = event["output_index"].as_u64().unwrap_or(0);
        let kind = event["type"].as_str().unwrap_or_default();
        let part = event["content_index"]
            .as_u64()
            .or_else(|| event["summary_index"].as_u64())
            .unwrap_or(0);
        let delta_key = (index, part, kind.contains("reasoning"));
        match kind {
            "response.created" | "response.in_progress" => {
                if let Some(response) = event["response"].as_object() {
                    for key in ["id", "model", "metadata"] {
                        if let Some(value) = response.get(key) {
                            self.body[key] = value.clone();
                        }
                    }
                }
                Ok(Vec::new())
            }
            "response.output_text.delta"
            | "response.refusal.delta"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta" => {
                self.response_deltas.insert(delta_key);
                let mut delta = required(&event, "delta")?;
                if kind == "response.refusal.delta"
                    && !self.response_fragments.contains_key(&delta_key)
                {
                    delta = format!("The model refused to respond: {delta}");
                }
                self.response_fragments
                    .entry(delta_key)
                    .or_default()
                    .push_str(&delta);
                Ok(vec![if !delta_key.2 {
                    ModelEvent::TextDelta { delta }
                } else {
                    ModelEvent::ReasoningDelta { delta }
                }])
            }
            "response.output_item.added" | "response.output_item.done" => {
                let item = &event["item"];
                if !item.is_object() {
                    return Err(protocol_error("invalid response output item"));
                }
                self.response_items.insert(index, item.clone());
                if item["type"] == "function_call" {
                    self.response_call_ids
                        .insert(required(item, "id")?, required(item, "call_id")?);
                }
                Ok(Vec::new())
            }
            "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
                if kind.ends_with(".done") && self.response_deltas.contains(&delta_key) {
                    return Ok(Vec::new());
                }
                let id = event["item_id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| {
                        event["output_index"]
                            .as_u64()
                            .and_then(|index| self.response_items.get(&index))
                            .and_then(|item| item["id"].as_str())
                            .map(str::to_owned)
                    })
                    .ok_or_else(|| protocol_error("tool arguments missing item identifier"))?;
                let call_id = self
                    .response_call_ids
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| protocol_error("tool arguments before call item"))?;
                self.response_deltas.insert(delta_key);
                let delta = required(
                    &event,
                    if kind.ends_with(".done") {
                        "arguments"
                    } else {
                        "delta"
                    },
                )?;
                self.response_fragments
                    .entry(delta_key)
                    .or_default()
                    .push_str(&delta);
                Ok(vec![ModelEvent::ToolArgumentsDelta { call_id, delta }])
            }
            "response.output_text.done"
            | "response.refusal.done"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.done" => {
                if !self.response_deltas.insert(delta_key) {
                    return Ok(Vec::new());
                }
                let mut delta = required(
                    &event,
                    if kind == "response.refusal.done" {
                        "refusal"
                    } else {
                        "text"
                    },
                )?;
                if delta.is_empty() {
                    return Ok(Vec::new());
                }
                if kind == "response.refusal.done" {
                    delta = format!("The model refused to respond: {delta}");
                }
                self.response_fragments.insert(delta_key, delta.clone());
                Ok(vec![if !delta_key.2 {
                    ModelEvent::TextDelta { delta }
                } else {
                    ModelEvent::ReasoningDelta { delta }
                }])
            }
            "response.completed" | "response.incomplete" => {
                let raw = &event["response"];
                self.snapshot_stop_reason = Some(
                    if raw["incomplete_details"]["reason"]
                        .as_str()
                        .is_some_and(|s| s.eq_ignore_ascii_case("max_output_tokens"))
                    {
                        "max_tokens"
                    } else if raw["output"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|item| item["type"] == "function_call")
                    {
                        "tool_use"
                    } else {
                        "end_turn"
                    },
                );
                let terminal = event["response"]
                    .as_object()
                    .ok_or_else(|| protocol_error("invalid response terminal event"))?;
                for (key, value) in terminal {
                    self.body[key] = value.clone();
                }
                for (&(index, _, reasoning), text) in &self.response_fragments {
                    self.response_items.entry(index).or_insert_with(|| {
                        if reasoning {
                            json!({"type":"reasoning","summary":[]})
                        } else {
                            json!({"type":"message","role":"assistant","content":[]})
                        }
                    });
                    if self.response_items[&index]["type"] == "function_call"
                        && self.response_items[&index]["arguments"]
                            .as_str()
                            .is_none_or(str::is_empty)
                    {
                        self.response_items.get_mut(&index).unwrap()["arguments"] =
                            text.clone().into();
                    }
                }
                for (&index, item) in &mut self.response_items {
                    let reasoning = item["type"] == "reasoning";
                    if !reasoning && item["type"] != "message" {
                        continue;
                    }
                    let field = if reasoning { "summary" } else { "content" };
                    if item[field].as_array().is_none_or(|parts| {
                        parts
                            .iter()
                            .all(|part| part["text"].as_str().is_none_or(str::is_empty))
                    }) {
                        let parts: Vec<_> = self.response_fragments.iter().filter(|((output, _, is_reasoning), _)| *output == index && *is_reasoning == reasoning).map(|(_, text)| json!({"type":if reasoning { "summary_text" } else { "output_text" },"text":text})).collect();
                        if !parts.is_empty() {
                            item[field] = parts.into();
                        }
                    }
                }
                if self
                    .body
                    .get("output")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty)
                    && !self.response_items.is_empty()
                {
                    self.body["output"] = self
                        .response_items
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
                        .into();
                }
                self.finish()
            }
            "response.failed" | "response.cancelled" => {
                Err(crate::error::response_error(&event, false)
                    .unwrap_or_else(|| protocol_error("provider response did not complete")))
            }
            _ => Ok(Vec::new()),
        }
    }
    fn finish(&mut self) -> Result<Vec<ModelEvent>, Error> {
        let mut response = wire::response(&self.body, self.protocol)?;
        if let Some(document) = &response.snapshot_raw {
            response.snapshot_raw = Some(crate::snapshot::stream_document(
                document,
                self.protocol,
                self.snapshot_stop_reason,
                self.tools.keys().enumerate().filter_map(|(index, key)| {
                    self.snapshot_starts.get(key).map(|start| {
                        (
                            index,
                            start,
                            self.snapshot_inputs.get(key).map(String::as_str),
                        )
                    })
                }),
            )?);
        }
        self.complete = true;
        let mut events: Vec<_> = response
            .items
            .iter()
            .cloned()
            .map(|item| ModelEvent::ItemDone { item })
            .collect();
        events.push(ModelEvent::Complete { response });
        Ok(events)
    }
    fn chat(&mut self, event: Value) -> Result<Vec<ModelEvent>, Error> {
        for key in ["id", "model", "metadata", "end_turn"] {
            if let Some(value) = event.get(key) {
                self.body[key] = value.clone();
            }
        }
        if event["usage"].is_object() {
            self.body["usage"] = event["usage"].clone();
        }
        let Some(choices) = event["choices"].as_array() else {
            return Err(protocol_error("chat chunk missing choices"));
        };
        let mut events = Vec::new();
        for choice in choices {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                return Err(protocol_error("multiple chat choices are unsupported"));
            }
            let delta = &choice["delta"];
            if let Some(opaque) = delta["reasoning_opaque"].as_str() {
                self.body["reasoning_opaque"] = opaque.into();
            }
            if let Some(details) = delta["reasoning_details"].as_array() {
                if !self.body["reasoning_details"].is_array() {
                    self.body["reasoning_details"] = json!([]);
                }
                self.body["reasoning_details"]
                    .as_array_mut()
                    .unwrap()
                    .extend(details.iter().cloned());
            }
            let reasoning = ["reasoning", "reasoning_content", "reasoning_text"]
                .iter()
                .filter_map(|key| delta[key].as_str())
                .find(|text| !text.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| wire::reasoning_details_text(&delta["reasoning_details"]));
            if !reasoning.is_empty() {
                self.reasoning.push_str(&reasoning);
                events.push(ModelEvent::ReasoningDelta { delta: reasoning });
            }
            if let Some(text) = delta["content"].as_str().filter(|text| !text.is_empty()) {
                self.text.push_str(text);
                events.push(ModelEvent::TextDelta {
                    delta: text.to_owned(),
                });
            }
            if let Some(refusal) = delta["refusal"].as_str().filter(|text| !text.is_empty()) {
                let text = if self.text.is_empty() {
                    format!("The model refused to respond: {refusal}")
                } else {
                    refusal.to_owned()
                };
                self.text.push_str(&text);
                events.push(ModelEvent::TextDelta { delta: text });
            }
            if let Some(calls) = delta["tool_calls"].as_array() {
                for call in calls {
                    let index = call["index"]
                        .as_u64()
                        .ok_or_else(|| protocol_error("tool chunk missing index"))?;
                    if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty())
                        && self
                            .tools
                            .get(&index)
                            .and_then(|tool| tool["id"].as_str())
                            .is_some_and(|previous| !previous.is_empty() && previous != id)
                    {
                        self.archived_tools.push(self.tools.remove(&index).unwrap());
                    }
                    let tool = self.tools.entry(index).or_insert_with(
                        || json!({"id":"","type":"function","function":{"name":"","arguments":""}}),
                    );
                    if let Some(id) = call["id"].as_str() {
                        tool["id"] = id.into();
                    }
                    if let Some(name) = call["function"]["name"].as_str() {
                        let combined = format!(
                            "{}{name}",
                            tool["function"]["name"].as_str().unwrap_or_default()
                        );
                        tool["function"]["name"] = combined.into();
                    }
                    if let Some(args) = call["function"]["arguments"].as_str() {
                        let combined = format!(
                            "{}{args}",
                            tool["function"]["arguments"].as_str().unwrap_or_default()
                        );
                        tool["function"]["arguments"] = combined.into();
                        if tool["id"].as_str().unwrap_or_default().is_empty() {
                            return Err(protocol_error("tool arguments arrived before call ID"));
                        }
                        events.push(ModelEvent::ToolArgumentsDelta {
                            call_id: required(tool, "id")?,
                            delta: args.to_owned(),
                        });
                    }
                }
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                self.finish_reason = Some(reason.to_owned());
            }
        }
        Ok(events)
    }
    fn anthropic(&mut self, event: Value) -> Result<Vec<ModelEvent>, Error> {
        let mut events = Vec::new();
        match event["type"].as_str() {
            Some("message_start") => {
                let message = &event["message"];
                if !message.is_object()
                    || message
                        .get("usage")
                        .is_some_and(|usage| !usage.is_null() && !usage.is_object())
                    || self.message_started
                {
                    return Err(protocol_error("invalid or repeated message start"));
                }
                self.message_started = true;
                self.body = message.clone();
            }
            Some("content_block_start") => {
                if !event["content_block"].is_object() || !self.message_started {
                    return Err(protocol_error("invalid content block start"));
                }
                let index = event["index"]
                    .as_u64()
                    .ok_or_else(|| protocol_error("content block missing index"))?;
                if self
                    .tools
                    .insert(index, event["content_block"].clone())
                    .is_some()
                {
                    return Err(protocol_error("duplicate content block"));
                }
                self.snapshot_starts
                    .insert(index, event["content_block"].clone());
            }
            Some("content_block_delta") => {
                let index = event["index"]
                    .as_u64()
                    .ok_or_else(|| protocol_error("content block missing index"))?;
                if self.stopped_blocks.contains(&index) {
                    return Err(protocol_error("content delta after block stop"));
                }
                let block = self
                    .tools
                    .get_mut(&index)
                    .ok_or_else(|| protocol_error("content delta before block start"))?;
                let delta = &event["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        let text = required(delta, "text")?;
                        block["text"] =
                            format!("{}{}", block["text"].as_str().unwrap_or_default(), text)
                                .into();
                        events.push(ModelEvent::TextDelta { delta: text });
                    }
                    Some("thinking_delta") => {
                        let text = required(delta, "thinking")?;
                        block["thinking"] =
                            format!("{}{}", block["thinking"].as_str().unwrap_or_default(), text)
                                .into();
                        events.push(ModelEvent::ReasoningDelta { delta: text });
                    }
                    Some("signature_delta") => {
                        block["signature"] = format!(
                            "{}{}",
                            block["signature"].as_str().unwrap_or_default(),
                            required(delta, "signature")?
                        )
                        .into()
                    }
                    Some("compaction_delta") => {
                        if block["type"] != "compaction" {
                            return Err(protocol_error("compaction delta has wrong block type"));
                        }
                        block["content"] = format!(
                            "{}{}",
                            block["content"].as_str().unwrap_or_default(),
                            delta["content"].as_str().unwrap_or_default()
                        )
                        .into();
                        if let Some(value) = delta["encrypted_content"]
                            .as_str()
                            .filter(|value| !value.is_empty())
                        {
                            block["encrypted_content"] = value.into();
                        }
                    }
                    Some("compaction_encrypted_content") => {
                        block["encrypted_content"] = required(delta, "encrypted_content")?.into();
                    }
                    Some("input_json_delta") => {
                        let args = required(delta, "partial_json")?;
                        self.argument_buffers
                            .entry(index)
                            .or_default()
                            .push_str(&args);
                        events.push(ModelEvent::ToolArgumentsDelta {
                            call_id: required(block, "id")?,
                            delta: args,
                        });
                    }
                    _ => return Err(protocol_error("unsupported Anthropic content delta")),
                }
            }
            Some("content_block_stop") => {
                let index = event["index"]
                    .as_u64()
                    .ok_or_else(|| protocol_error("content block missing index"))?;
                if !self.tools.contains_key(&index) || !self.stopped_blocks.insert(index) {
                    return Err(protocol_error("content stop without active block"));
                }
                if let Some(args) = self.argument_buffers.remove(&index) {
                    self.tools
                        .get_mut(&index)
                        .ok_or_else(|| protocol_error("content stop before start"))?["input"] =
                        serde_json::from_str(&args)
                            .map_err(|_| protocol_error("invalid streamed tool arguments"))?;
                    self.snapshot_inputs.insert(index, args);
                }
            }
            Some("message_delta") => {
                if let Some(usage) = event["usage"].as_object() {
                    for (key, value) in usage {
                        self.body["usage"][key] = value.clone();
                    }
                }
                self.body["stop_reason"] = event["delta"]["stop_reason"].clone();
                if let Some(end_turn) = event["delta"].get("end_turn") {
                    self.body["end_turn"] = end_turn.clone();
                }
            }
            Some("message_stop") => {
                if !self.message_started {
                    return Err(protocol_error("premature Anthropic terminal event"));
                }
                if self
                    .tools
                    .keys()
                    .any(|index| !self.stopped_blocks.contains(index))
                {
                    return Err(protocol_error("unfinished content block"));
                }
                if !self.argument_buffers.is_empty() {
                    return Err(protocol_error("unfinished tool arguments"));
                }
                self.body["content"] = self.tools.values().cloned().collect::<Vec<_>>().into();
                return self.finish();
            }
            Some("ping") => {}
            _ => return Err(protocol_error("unsupported Anthropic stream event")),
        }
        Ok(events)
    }
}
fn required(value: &Value, key: &str) -> Result<String, Error> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| protocol_error("stream event missing required field"))
}
