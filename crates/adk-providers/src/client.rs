//! Owned, pull-based HTTP streams with no detached producer and no implicit retry.
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
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::SystemTime,
};

pub struct Provider {
    name: String,
    protocol: Protocol,
    session: Arc<Session>,
    client: Client,
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
            for key in ["reasoning", "text"] {
                if let Some(value) = source.get(key) {
                    body[key] = value.clone();
                }
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
            body["store"] = false.into();
            body["stream"] = true.into();
            body.as_object_mut()
                .unwrap()
                .remove("prompt_cache_retention");
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
        let endpoint = format!("{}{path}", scope.endpoint);
        for attempt in 0..=1 {
            let material = self.session.material(context).await?;
            let mut body = body.clone();
            if let Some(key) = body.get("prompt_cache_key").and_then(Value::as_str) {
                body["prompt_cache_key"] = crate::auth::cache_scope(scope, &material, key).into();
            }
            let headers =
                crate::auth::headers(scope, &material, self.protocol == Protocol::Anthropic)?;
            let response = crate::active(
                context,
                self.client
                    .post(&endpoint)
                    .headers(headers)
                    .json(&body)
                    .send(),
            )
            .await?
            .map_err(|e| RequestFailure::transport(&e).into_error())?;
            if response.status().as_u16() == 401
                && attempt == 0
                && matches!(
                    scope.mode,
                    AuthMode::OpenAiOAuth | AuthMode::AnthropicOAuth | AuthMode::CopilotOAuth
                )
            {
                drop(response);
                self.session.reject(context, &material.access_token).await?;
                continue;
            }
            if !response.status().is_success() {
                return Err(RequestFailure::http(
                    response.status().as_u16(),
                    response.headers(),
                    SystemTime::now(),
                )
                .into_error());
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
            if self.session.scope().mode == AuthMode::OpenAiOAuth {
                let mut stream = self.stream(context, request).await?;
                let mut complete = None;
                while let Some(event) = stream.next().await? {
                    if let ModelEvent::Complete { response } = event {
                        complete = Some(response);
                    }
                }
                return complete.ok_or_else(|| protocol_error("stream ended without completion"));
            }
            let response = self.send(context, &request, false).await?;
            wire::response(&read_json(context, response).await?, self.protocol)
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
async fn read_json(context: &Context, mut response: reqwest::Response) -> Result<Value, Error> {
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
    serde_json::from_slice(&bytes).map_err(|_| protocol_error("invalid provider response JSON"))
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
    text: String,
    reasoning: String,
    complete: bool,
    finish_reason: Option<String>,
    bytes: usize,
    response_call_ids: BTreeMap<String, String>,
}
impl StreamState {
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            body: json!({}),
            tools: BTreeMap::new(),
            argument_buffers: BTreeMap::new(),
            text: String::new(),
            reasoning: String::new(),
            complete: false,
            finish_reason: None,
            bytes: 0,
            response_call_ids: BTreeMap::new(),
        }
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }
    pub fn event(&mut self, data: &str) -> Result<Vec<ModelEvent>, Error> {
        if self.complete {
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
                "tool_calls":self.tools.values().collect::<Vec<_>>()},"finish_reason":self.finish_reason}]);
            return self.finish();
        }
        let event: Value =
            serde_json::from_str(data).map_err(|_| protocol_error("invalid SSE JSON"))?;
        if event.get("error").is_some_and(|v| !v.is_null()) || event["type"] == "error" {
            return Err(protocol_error("provider stream error"));
        }
        match self.protocol {
            Protocol::Responses => match event["type"].as_str() {
                Some("response.output_text.delta") => Ok(vec![ModelEvent::TextDelta {
                    delta: required(&event, "delta")?,
                }]),
                Some("response.reasoning_summary_text.delta" | "response.reasoning_text.delta") => {
                    Ok(vec![ModelEvent::ReasoningDelta {
                        delta: required(&event, "delta")?,
                    }])
                }
                Some("response.output_item.added") if event["item"]["type"] == "function_call" => {
                    let item = &event["item"];
                    self.response_call_ids
                        .insert(required(item, "id")?, required(item, "call_id")?);
                    Ok(Vec::new())
                }
                Some("response.function_call_arguments.delta") => {
                    let id = required(&event, "item_id")?;
                    let call_id = self
                        .response_call_ids
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| protocol_error("tool arguments before call item"))?;
                    Ok(vec![ModelEvent::ToolArgumentsDelta {
                        call_id,
                        delta: required(&event, "delta")?,
                    }])
                }
                Some("response.completed") => {
                    self.body = event["response"].clone();
                    self.finish()
                }
                Some("response.failed" | "response.incomplete") => {
                    Err(protocol_error("provider response did not complete"))
                }
                _ => Ok(Vec::new()),
            },
            Protocol::Chat => self.chat(event),
            Protocol::Anthropic => self.anthropic(event),
        }
    }
    fn finish(&mut self) -> Result<Vec<ModelEvent>, Error> {
        let response = wire::response(&self.body, self.protocol)?;
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
        if let Some(id) = event.get("id") {
            self.body["id"] = id.clone();
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
            if let Some(text) = delta["content"].as_str() {
                self.text.push_str(text);
                events.push(ModelEvent::TextDelta {
                    delta: text.to_owned(),
                });
            }
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
            if let Some(text) = ["reasoning", "reasoning_content", "reasoning_text"]
                .iter()
                .filter_map(|key| delta[key].as_str())
                .find(|text| !text.is_empty())
            {
                self.reasoning.push_str(text);
                events.push(ModelEvent::ReasoningDelta {
                    delta: text.to_owned(),
                });
            }
            if let Some(calls) = delta["tool_calls"].as_array() {
                for call in calls {
                    let index = call["index"]
                        .as_u64()
                        .ok_or_else(|| protocol_error("tool chunk missing index"))?;
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
                    || !message["id"].is_string()
                    || message.get("usage").is_some_and(|usage| !usage.is_object())
                    || self.body.get("id").is_some()
                {
                    return Err(protocol_error("invalid or repeated message start"));
                }
                self.body = message.clone();
            }
            Some("content_block_start") => {
                if !event["content_block"].is_object() || self.body.get("id").is_none() {
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
            }
            Some("content_block_delta") => {
                let index = event["index"]
                    .as_u64()
                    .ok_or_else(|| protocol_error("content block missing index"))?;
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
                if let Some(args) = self.argument_buffers.remove(&index) {
                    self.tools
                        .get_mut(&index)
                        .ok_or_else(|| protocol_error("content stop before start"))?["input"] =
                        serde_json::from_str(&args)
                            .map_err(|_| protocol_error("invalid streamed tool arguments"))?;
                }
            }
            Some("message_delta") => {
                if let Some(usage) = event["usage"].as_object() {
                    for (key, value) in usage {
                        self.body["usage"][key] = value.clone();
                    }
                }
                self.body["stop_reason"] = event["delta"]["stop_reason"].clone();
            }
            Some("message_stop") => {
                if !self.body["id"].is_string() || !self.body["stop_reason"].is_string() {
                    return Err(protocol_error("premature Anthropic terminal event"));
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
