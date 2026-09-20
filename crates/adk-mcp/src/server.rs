//! Policy-gated Streamable HTTP serving. Hosts authenticate requests and own all
//! authorization, approval, audit and quota decisions in the supplied callbacks.
//! Serve behind verified TLS outside loopback; never trust a client tenant header
//! without authenticating it. No executable `Tool` is accepted by this API.
use crate::{BoxFuture, Error, Limits, PROTOCOL_VERSION};
use adk_core::ToolDefinition;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, request::Parts, uri::Authority},
    response::Response,
    routing::any,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

pub const MAX_SESSIONS: usize = 1024;
pub const MAX_SESSIONS_PER_TENANT: usize = 128;
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerTransport {
    StreamableHttp,
    Stdio,
    LegacySse,
}

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub transport: ServerTransport,
    pub limits: Limits,
    /// Exact Host hostnames (without ports), never suffixes or forwarded headers.
    pub allowed_hosts: Vec<String>,
    /// Exact serialized origins. An empty list rejects all browser Origins.
    pub allowed_origins: Vec<String>,
}
impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            transport: ServerTransport::StreamableHttp,
            limits: Limits::default(),
            allowed_hosts: vec!["localhost".into(), "127.0.0.1".into(), "[::1]".into()],
            allowed_origins: vec![],
        }
    }
}

pub trait TenantResolver: Send + Sync {
    fn resolve_tenant<'a>(&'a self, request: &'a Parts) -> BoxFuture<'a, Result<String, Error>>;
}

/// Private, owned fields bind approval/audit to the exact request seen by policy.
#[derive(Debug)]
pub struct ServerToolRequest {
    tenant_id: String,
    tool: ToolDefinition,
    arguments: Vec<u8>,
    request_sha256: String,
}
impl ServerToolRequest {
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn tool(&self) -> &ToolDefinition {
        &self.tool
    }
    pub fn arguments(&self) -> &[u8] {
        &self.arguments
    }
    pub fn request_sha256(&self) -> &str {
        &self.request_sha256
    }
}

#[derive(Debug, Clone, Default)]
pub struct ServerToolResult {
    pub content: String,
    pub is_error: bool,
}

/// The only tool execution path. The host must apply native SDK policy here.
pub trait ServerToolPolicy: Send + Sync {
    fn execute_mcp_tool(
        &self,
        request: ServerToolRequest,
    ) -> BoxFuture<'_, Result<ServerToolResult, Error>>;
}

fn empty_string(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(String::is_empty)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDefinition {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "empty_string")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "empty_string")]
    pub mime_type: Option<String>,
}
pub trait ResourcePolicy: Send + Sync {
    fn read<'a>(&'a self, tenant: &'a str, uri: &'a str) -> BoxFuture<'a, Result<Value, Error>>;
}
pub struct ServerResource {
    pub definition: ResourceDefinition,
    pub policy: Arc<dyn ResourcePolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "empty_string")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "empty_string")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<PromptArgument>,
}
pub trait PromptPolicy: Send + Sync {
    fn get<'a>(
        &'a self,
        tenant: &'a str,
        arguments: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<Value, Error>>;
}
pub struct ServerPrompt {
    pub definition: PromptDefinition,
    pub policy: Arc<dyn PromptPolicy>,
}

struct SessionBinding {
    tenant: String,
    last_seen: Instant,
    initialized: bool,
}
#[derive(Default)]
struct Sessions {
    bindings: HashMap<String, SessionBinding>,
    closed: bool,
}
pub struct ServerMode {
    tools: Vec<ToolDefinition>,
    validators: HashMap<String, jsonschema::Validator>,
    policy: Arc<dyn ServerToolPolicy>,
    tenants: Arc<dyn TenantResolver>,
    resources: Vec<ServerResource>,
    prompts: Vec<ServerPrompt>,
    options: ServerOptions,
    sessions: Mutex<Sessions>,
}
impl ServerMode {
    pub fn new(
        mut tools: Vec<ToolDefinition>,
        policy: Arc<dyn ServerToolPolicy>,
        tenants: Arc<dyn TenantResolver>,
        mut resources: Vec<ServerResource>,
        mut prompts: Vec<ServerPrompt>,
        options: ServerOptions,
    ) -> Result<Self, Error> {
        if options.transport != ServerTransport::StreamableHttp {
            return Err(Error::Config(
                "server mode supports only Streamable HTTP; stdio and legacy SSE are unsupported"
                    .into(),
            ));
        }
        if options.limits.max_message_bytes < 256
            || options.limits.timeout.is_zero()
            || options.allowed_hosts.is_empty()
        {
            return Err(Error::Config(
                "server mode requires finite bounds and allowed hosts".into(),
            ));
        }
        if tools.len() > options.limits.max_items
            || resources.len() > options.limits.max_items
            || prompts.len() > options.limits.max_items
        {
            return Err(Error::Limit);
        }
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        if tools.iter().any(|t| t.name.trim().is_empty())
            || tools.windows(2).any(|p| p[0].name == p[1].name)
        {
            return Err(Error::Config(
                "tool names must be nonempty and unique".into(),
            ));
        }
        let mut validators = HashMap::new();
        for tool in &tools {
            let validator = jsonschema::options()
                .with_retriever(NoExternalSchemas)
                .build(tool.input_schema.as_value())
                .map_err(|_| {
                    Error::Config("invalid or externally referenced tool schema".into())
                })?;
            validators.insert(tool.name.clone(), validator);
        }
        resources.sort_by(|a, b| a.definition.uri.cmp(&b.definition.uri));
        if resources
            .iter()
            .any(|r| url::Url::parse(&r.definition.uri).is_err())
            || resources
                .windows(2)
                .any(|p| p[0].definition.uri == p[1].definition.uri)
        {
            return Err(Error::Config(
                "resources require unique absolute URIs".into(),
            ));
        }
        prompts.sort_by(|a, b| a.definition.name.cmp(&b.definition.name));
        if prompts.iter().any(|p| p.definition.name.trim().is_empty())
            || prompts
                .windows(2)
                .any(|p| p[0].definition.name == p[1].definition.name)
        {
            return Err(Error::Config(
                "prompt names must be nonempty and unique".into(),
            ));
        }
        Ok(Self {
            tools,
            validators,
            policy,
            tenants,
            resources,
            prompts,
            options,
            sessions: Mutex::new(Sessions::default()),
        })
    }

    /// Mount this router at a host-owned endpoint, or call `serve` for `/mcp`.
    pub fn handler(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/", any(handle))
            .with_state(self.clone())
    }

    pub async fn serve(self: Arc<Self>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
        let app = Router::new().route("/mcp", any(handle)).with_state(self);
        axum::serve(listener, app).await
    }

    /// Reject subsequent requests and release all session bindings. Already
    /// dispatched host operations are not replayed and may complete.
    pub fn close(&self) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.closed = true;
        sessions.bindings.clear();
    }

    pub fn session_count(&self) -> usize {
        let mut sessions = self.sessions.lock().unwrap();
        prune(&mut sessions);
        sessions.bindings.len()
    }

    fn gate(&self, headers: &HeaderMap) -> bool {
        let Some(host) = one_header(headers, "host") else {
            return false;
        };
        let Ok(authority) = host.parse::<Authority>() else {
            return false;
        };
        if !self
            .options
            .allowed_hosts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(authority.host()))
        {
            return false;
        }
        if headers.contains_key("origin") {
            let Some(origin) = one_header(headers, "origin") else {
                return false;
            };
            if !self.options.allowed_origins.iter().any(|o| o == origin) {
                return false;
            }
        }
        true
    }

    async fn request(&self, request: Request) -> Response {
        if self.sessions.lock().unwrap().closed {
            return status(StatusCode::SERVICE_UNAVAILABLE);
        }
        let (parts, body) = request.into_parts();
        if !self.gate(&parts.headers) {
            return status(StatusCode::FORBIDDEN);
        }
        let tenant = match self.tenants.resolve_tenant(&parts).await {
            Ok(t) if !t.trim().is_empty() && t.len() <= self.options.limits.max_message_bytes => {
                t.trim().to_owned()
            }
            _ => return status(StatusCode::UNAUTHORIZED),
        };
        let session_id = one_header(&parts.headers, "mcp-session-id");
        if parts.headers.contains_key("mcp-session-id") && session_id.is_none_or(str::is_empty) {
            return status(StatusCode::BAD_REQUEST);
        }
        if let Some(id) = session_id {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.closed {
                return status(StatusCode::SERVICE_UNAVAILABLE);
            }
            prune(&mut sessions);
            match sessions.bindings.get_mut(id) {
                Some(binding) if binding.tenant == tenant => binding.last_seen = Instant::now(),
                _ => return status(StatusCode::FORBIDDEN),
            }
            if parts.method == Method::DELETE {
                sessions.bindings.remove(id);
                return status(StatusCode::NO_CONTENT);
            }
        }
        if parts.method != Method::POST {
            return status(StatusCode::METHOD_NOT_ALLOWED);
        }
        if one_header(&parts.headers, "content-type").is_none_or(|v| {
            !v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        }) {
            return status(StatusCode::UNSUPPORTED_MEDIA_TYPE);
        }
        let accept = parts
            .headers
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !accept.split(',').any(|v| v.trim() == "application/json")
            || !accept.split(',').any(|v| v.trim() == "text/event-stream")
        {
            return status(StatusCode::NOT_ACCEPTABLE);
        }
        if parts.headers.contains_key("mcp-protocol-version")
            && one_header(&parts.headers, "mcp-protocol-version") != Some(PROTOCOL_VERSION)
        {
            return status(StatusCode::BAD_REQUEST);
        }
        let bytes = match to_bytes(body, self.options.limits.max_message_bytes).await {
            Ok(b) => b,
            Err(_) => return status(StatusCode::PAYLOAD_TOO_LARGE),
        };
        let message: Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => return self.rpc_error(Value::Null, -32700, "parse error"),
        };
        if let Value::Array(messages) = message {
            if messages.is_empty() || messages.len() > self.options.limits.max_items {
                return self.rpc_error(Value::Null, -32600, "empty or oversized batch");
            }
            // MCP initialization must not be part of a JSON-RPC batch. Reject
            // before processing any element, so no hidden session is created.
            if messages
                .iter()
                .any(|m| m.get("method").and_then(Value::as_str) == Some("initialize"))
            {
                return self.rpc_error(Value::Null, -32600, "initialize cannot be batched");
            }
            let mut output = vec![b'['];
            let mut count = 0;
            for message in messages {
                let response = self.dispatch(&tenant, session_id, message).await;
                if response.status() == StatusCode::ACCEPTED {
                    continue;
                }
                // A failed dispatched operation must keep its ambiguous HTTP
                // outcome, not be flattened into a definitive batch RPC error.
                if response.status() != StatusCode::OK {
                    return response;
                }
                let remaining = self
                    .options
                    .limits
                    .max_message_bytes
                    .saturating_sub(output.len() + 2);
                let body = match to_bytes(response.into_body(), remaining).await {
                    Ok(body) => body,
                    Err(_) => return status(StatusCode::PAYLOAD_TOO_LARGE),
                };
                if count > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(&body);
                count += 1;
            }
            if count == 0 {
                return status(StatusCode::ACCEPTED);
            }
            output.push(b']');
            return Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .header("cache-control", "no-store")
                .body(Body::from(output))
                .unwrap();
        }
        self.dispatch(&tenant, session_id, message).await
    }

    async fn dispatch(&self, tenant: &str, session_id: Option<&str>, message: Value) -> Response {
        let id = message.get("id").cloned();
        if !message.is_object()
            || message.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || message.get("method").and_then(Value::as_str).is_none()
            || id
                .as_ref()
                .is_some_and(|v| !v.is_string() && !v.is_i64() && !v.is_u64())
            || message
                .get("params")
                .is_some_and(|p| !p.is_null() && !p.is_object())
        {
            return self.rpc_error(Value::Null, -32600, "invalid request");
        }
        let method = message["method"].as_str().unwrap();
        let params = message
            .get("params")
            .filter(|p| !p.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
        if method == "initialize" {
            let Some(id) = id else {
                return status(StatusCode::BAD_REQUEST);
            };
            if session_id.is_some()
                || !params["protocolVersion"].is_string()
                || !params["capabilities"].is_object()
                || !params["clientInfo"]["name"].is_string()
                || !params["clientInfo"]["version"].is_string()
            {
                return self.rpc_error(id, -32602, "invalid initialize parameters");
            }
            let mut capabilities = json!({"tools": {"listChanged": false}});
            if !self.resources.is_empty() {
                capabilities["resources"] = json!({"subscribe": false, "listChanged": false});
            }
            if !self.prompts.is_empty() {
                capabilities["prompts"] = json!({"listChanged": false});
            }
            let mut response = self.rpc_result(
                id,
                json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": capabilities,
                "serverInfo": {"name": "adk-mcp", "version": env!("CARGO_PKG_VERSION")}}),
            );
            if response.status() != StatusCode::OK {
                return response;
            }
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.closed {
                return status(StatusCode::SERVICE_UNAVAILABLE);
            }
            prune(&mut sessions);
            if sessions.bindings.len() >= MAX_SESSIONS
                || sessions
                    .bindings
                    .values()
                    .filter(|b| b.tenant == tenant)
                    .count()
                    >= MAX_SESSIONS_PER_TENANT
            {
                return status(StatusCode::TOO_MANY_REQUESTS);
            }
            let session_id = uuid::Uuid::new_v4().to_string();
            response.headers_mut().insert(
                "mcp-session-id",
                HeaderValue::from_str(&session_id).unwrap(),
            );
            sessions.bindings.insert(
                session_id,
                SessionBinding {
                    tenant: tenant.to_owned(),
                    last_seen: Instant::now(),
                    initialized: false,
                },
            );
            return response;
        }
        let Some(session_id) = session_id else {
            return status(StatusCode::BAD_REQUEST);
        };
        {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.closed {
                return status(StatusCode::SERVICE_UNAVAILABLE);
            }
            let Some(binding) = sessions.bindings.get_mut(session_id) else {
                return status(StatusCode::FORBIDDEN);
            };
            if method == "notifications/initialized" && id.is_none() {
                binding.initialized = true;
                return status(StatusCode::ACCEPTED);
            }
            if !binding.initialized {
                return status(StatusCode::BAD_REQUEST);
            }
        }
        let Some(id) = id else {
            // Notifications must never invoke an execution callback.
            return status(StatusCode::ACCEPTED);
        };
        if matches!(method, "tools/list" | "resources/list" | "prompts/list") {
            let cursor = &params["cursor"];
            if !cursor.is_null() && !cursor.is_string() {
                // The pinned protocol SDK uses code 0 for typed parameter decoding failures.
                return self.rpc_error(id, 0, "invalid cursor");
            }
            if cursor.as_str().is_some_and(|cursor| !cursor.is_empty()) {
                return self.rpc_error(id, -32602, "invalid cursor");
            }
        }
        let result = match method {
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": self.tools.iter().map(|t| {
                    let mut tool = json!({"name": t.name, "inputSchema": t.input_schema, "annotations": {}});
                    if !t.description.is_empty() { tool["description"] = t.description.clone().into(); }
                    if t.read_only { tool["annotations"]["readOnlyHint"] = true.into(); }
                    tool
                }).collect::<Vec<_>>()})),
            "tools/call" => {
                let tool = self
                    .tools
                    .iter()
                    .find(|t| Some(t.name.as_str()) == params["name"].as_str());
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let Some(tool) = tool else {
                    return self.rpc_error(id, -32602, "unknown tool");
                };
                let empty = json!({});
                let validated_args = if args.is_null() { &empty } else { &args };
                if !validated_args.is_object()
                    || !self.validators[&tool.name].is_valid(validated_args)
                {
                    return self.rpc_error(id, -32602, "invalid tool arguments");
                }
                let arguments = serde_json::to_vec(&args).unwrap();
                let request_sha256 = request_digest(tenant, &tool.name, &arguments);
                match self
                    .policy
                    .execute_mcp_tool(ServerToolRequest {
                        tenant_id: tenant.to_owned(),
                        tool: tool.clone(),
                        arguments,
                        request_sha256,
                    })
                    .await
                {
                    Ok(r) => {
                        let mut result = json!({"content": [{"type": "text", "text": r.content}]});
                        if r.is_error {
                            result["isError"] = true.into();
                        }
                        Ok(result)
                    }
                    // A downstream outcome-unknown is not a definitive JSON-RPC
                    // error. Preserve ambiguity without reflecting callback text.
                    Err(Error::ReconciliationRequired { .. }) => {
                        return status(StatusCode::BAD_GATEWAY);
                    }
                    Err(_) => Ok(json!({
                        "content": [{"type": "text", "text": "tool execution denied or failed"}],
                        "isError": true
                    })),
                }
            }
            "resources/list" => Ok(
                json!({"resources": self.resources.iter().map(|r| &r.definition).collect::<Vec<_>>()}),
            ),
            "resources/read" => {
                if !params["uri"].is_null() && !params["uri"].is_string() {
                    // The pinned Go decoder reports a typed-field decode error
                    // with code 0, distinct from resource-not-found (-32002).
                    return self.rpc_error(id, 0, "invalid resource URI");
                }
                let Some(resource) = self
                    .resources
                    .iter()
                    .find(|r| Some(r.definition.uri.as_str()) == params["uri"].as_str())
                else {
                    return self.rpc_error(id, -32002, "resource not found");
                };
                match resource.policy.read(tenant, &resource.definition.uri).await {
                    Ok(result) => Ok(result),
                    Err(Error::ReconciliationRequired { .. }) => {
                        return status(StatusCode::BAD_GATEWAY);
                    }
                    Err(_) => Err("resource access denied or failed"),
                }
            }
            "prompts/list" => Ok(
                json!({"prompts": self.prompts.iter().map(|p| &p.definition).collect::<Vec<_>>()}),
            ),
            "prompts/get" => {
                let Some(prompt) = self
                    .prompts
                    .iter()
                    .find(|p| Some(p.definition.name.as_str()) == params["name"].as_str())
                else {
                    return self.rpc_error(id, -32602, "unknown prompt");
                };
                let args = params
                    .get("arguments")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let Some(args) = args.as_object() else {
                    return self.rpc_error(id, -32602, "invalid prompt arguments");
                };
                if args.values().any(|v| !v.is_string())
                    || prompt
                        .definition
                        .arguments
                        .iter()
                        .any(|a| a.required && !args.contains_key(&a.name))
                {
                    return self.rpc_error(id, -32602, "invalid prompt arguments");
                }
                match prompt.policy.get(tenant, args).await {
                    Ok(result) => Ok(result),
                    Err(Error::ReconciliationRequired { .. }) => {
                        return status(StatusCode::BAD_GATEWAY);
                    }
                    Err(_) => Err("prompt access denied or failed"),
                }
            }
            _ => return self.rpc_error(id, -32601, "method not found or unsupported cursor"),
        };
        match result {
            Ok(result) => self.rpc_result(id, result),
            Err(message) => self.rpc_error(id, -32603, message),
        }
    }

    fn rpc_result(&self, id: Value, result: Value) -> Response {
        self.json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }
    fn rpc_error(&self, id: Value, code: i64, message: &str) -> Response {
        self.json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
    }
    fn json(&self, value: Value) -> Response {
        let mut writer = BoundedWriter {
            bytes: vec![],
            limit: self.options.limits.max_message_bytes,
        };
        if serde_json::to_writer(&mut writer, &value).is_err() {
            return status(StatusCode::PAYLOAD_TOO_LARGE);
        }
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .header("cache-control", "no-store")
            .body(Body::from(writer.bytes))
            .unwrap()
    }
}

/// Explicitly refuse external retrieval even if an embedding application's
/// Cargo feature unification enables jsonschema's HTTP/file default resolvers.
struct NoExternalSchemas;
impl jsonschema::Retrieve for NoExternalSchemas {
    fn retrieve(
        &self,
        _: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("external schema retrieval denied".into())
    }
}

async fn handle(State(server): State<Arc<ServerMode>>, request: Request) -> Response {
    match tokio::time::timeout(server.options.limits.timeout, server.request(request)).await {
        Ok(response) => response,
        Err(_) => status(StatusCode::GATEWAY_TIMEOUT),
    }
}
fn prune(sessions: &mut Sessions) {
    sessions
        .bindings
        .retain(|_, b| b.last_seen.elapsed() < SESSION_TTL);
}
fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}
fn status(status: StatusCode) -> Response {
    Response::builder()
        .status(status)
        .header("cache-control", "no-store")
        .body(Body::empty())
        .unwrap()
}
fn request_digest(tenant: &str, tool: &str, args: &[u8]) -> String {
    let mut digest = Sha256::new();
    for value in [tenant.as_bytes(), tool.as_bytes(), args] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value);
    }
    format!("{:x}", digest.finalize())
}
struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("MCP response bound exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_expiration_and_digest_framing() {
        let mut sessions = Sessions::default();
        sessions.bindings.insert(
            "old".into(),
            SessionBinding {
                tenant: "a".into(),
                last_seen: Instant::now() - SESSION_TTL,
                initialized: true,
            },
        );
        sessions.bindings.insert(
            "new".into(),
            SessionBinding {
                tenant: "a".into(),
                last_seen: Instant::now(),
                initialized: true,
            },
        );
        prune(&mut sessions);
        assert_eq!(sessions.bindings.len(), 1);
        assert!(sessions.bindings.contains_key("new"));
        assert_ne!(
            request_digest("ab", "c", b"{}"),
            request_digest("a", "bc", b"{}")
        );
    }
}
