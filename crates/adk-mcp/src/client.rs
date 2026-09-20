use crate::{BoxFuture, Error, Limits, PROTOCOL_VERSION, Transport, config::ServerConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct ServerPolicy {
    pub enabled: bool,
    pub allowed_origins: BTreeSet<String>,
    pub allowed_tools: Option<BTreeSet<String>>,
    pub read_only_tools: BTreeSet<String>,
    pub allowed_resources: Option<BTreeSet<String>>,
    pub allowed_prompts: Option<BTreeSet<String>>,
    pub allow_env: BTreeSet<String>,
}
#[derive(Clone, Default)]
pub struct HostPolicy {
    pub tenant_id: String,
    pub servers: BTreeMap<String, ServerPolicy>,
    pub hooks: Option<Arc<dyn HostHooks>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationContext {
    tenant_id: String,
    server: String,
    operation: String,
    tool: Option<String>,
    arguments_sha256: [u8; 32],
    request_sha256: [u8; 32],
}
impl OperationContext {
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    pub fn server(&self) -> &str {
        &self.server
    }
    pub fn operation(&self) -> &str {
        &self.operation
    }
    pub fn tool(&self) -> Option<&str> {
        self.tool.as_deref()
    }
    pub fn arguments_sha256(&self) -> [u8; 32] {
        self.arguments_sha256
    }
    pub fn request_sha256(&self) -> [u8; 32] {
        self.request_sha256
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    Attempted,
    Completed,
    Denied,
    Failed,
    OutcomeUnknown,
}

pub trait HostHooks: Send + Sync {
    fn approve<'a>(&'a self, _context: &'a OperationContext) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async { Ok(false) })
    }
    fn break_glass<'a>(
        &'a self,
        _context: &'a OperationContext,
        _reason: &'a str,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async { Ok(false) })
    }
    fn audit<'a>(
        &'a self,
        context: &'a OperationContext,
        outcome: AuditOutcome,
    ) -> BoxFuture<'a, Result<(), Error>>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolDescriptor {
    pub qualified_name: String,
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub display_description: String,
    pub input_schema: Value,
    pub read_only: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDescriptor {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub server: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PromptArgument {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PromptDescriptor {
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub arguments: Vec<PromptArgument>,
    #[serde(default)]
    pub server: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub tools: bool,
    pub resources: bool,
    pub prompts: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    New,
    Ready,
    Failed,
    Closed,
}

pub struct Client {
    transport: Box<dyn Transport>,
    server: String,
    config: ServerConfig,
    policy: HostPolicy,
    limits: Limits,
    state: State,
    capabilities: Capabilities,
    tools: Option<Vec<ToolDescriptor>>,
    resources: Option<BTreeMap<String, ResourceDescriptor>>,
    prompts: Option<BTreeMap<String, PromptDescriptor>>,
    discovery_cache_ttl: Duration,
    resources_discovered_at: Option<Instant>,
    prompts_discovered_at: Option<Instant>,
}
impl Client {
    pub fn new(
        transport: Box<dyn Transport>,
        server: impl Into<String>,
        config: ServerConfig,
        policy: HostPolicy,
    ) -> Result<Self, Error> {
        Self::with_limits(transport, server, config, policy, Limits::default())
    }
    pub fn with_limits(
        transport: Box<dyn Transport>,
        server: impl Into<String>,
        config: ServerConfig,
        policy: HostPolicy,
        limits: Limits,
    ) -> Result<Self, Error> {
        config.validate()?;
        let server = server.into();
        if server.trim().is_empty() || policy.tenant_id.trim().is_empty() {
            return Err(Error::Policy("tenant and server are required".into()));
        }
        if limits.max_pages == 0
            || limits.max_items == 0
            || limits.max_message_bytes == 0
            || limits.timeout.is_zero()
        {
            return Err(Error::Config("positive client bounds required".into()));
        }
        let client = Self {
            transport,
            server,
            config,
            policy,
            limits,
            state: State::New,
            capabilities: Capabilities::default(),
            tools: None,
            resources: None,
            prompts: None,
            discovery_cache_ttl: Duration::from_secs(30),
            resources_discovered_at: None,
            prompts_discovered_at: None,
        };
        client.check_server()?;
        Ok(client)
    }
    pub fn server_name(&self) -> &str {
        &self.server
    }
    pub fn diagnostics(&self) -> Option<String> {
        self.transport.diagnostics()
    }
    /// Sets resource/prompt cache lifetime (default 30 seconds). Zero disables reuse.
    /// Tool discovery remains pinned until an explicit reconnect.
    pub fn set_discovery_cache_ttl(&mut self, ttl: Duration) {
        self.discovery_cache_ttl = ttl;
        self.invalidate_discovery();
    }
    /// Clears resource and prompt discovery, leaving tool discovery pinned.
    pub fn invalidate_discovery(&mut self) {
        self.resources = None;
        self.prompts = None;
        self.resources_discovered_at = None;
        self.prompts_discovered_at = None;
    }
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
    pub fn is_ready(&self) -> bool {
        self.state == State::Ready
    }
    pub fn filtered_env(&self, inherited: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        self.config.filtered_env(inherited, &self.grant().allow_env)
    }
    fn grant(&self) -> &ServerPolicy {
        &self.policy.servers[&self.server]
    }
    fn check_server(&self) -> Result<(), Error> {
        let grant = self
            .policy
            .servers
            .get(&self.server)
            .ok_or_else(|| Error::Policy("server not granted by host".into()))?;
        if !self.config.enabled() || !grant.enabled {
            return Err(Error::Policy("server disabled".into()));
        }
        if self.config.is_remote() && !grant.allowed_origins.contains(&self.config.origin()?) {
            return Err(Error::Policy("remote origin not granted by host".into()));
        }
        Ok(())
    }
    fn ready(&self) -> Result<(), Error> {
        self.check_server()?;
        if self.state != State::Ready {
            return Err(Error::Closed);
        }
        Ok(())
    }
    fn context(&self, operation: &str, tool: Option<&str>, arguments: &Value) -> OperationContext {
        OperationContext {
            tenant_id: self.policy.tenant_id.clone(),
            server: self.server.clone(),
            operation: operation.into(),
            tool: tool.map(str::to_owned),
            arguments_sha256: Sha256::digest(
                serde_json::to_vec(arguments).expect("JSON Value serializes"),
            )
            .into(),
            request_sha256: Sha256::digest(
                serde_json::to_vec(&json!([
                    self.policy.tenant_id,
                    self.server,
                    operation,
                    tool,
                    arguments
                ]))
                .expect("JSON Value serializes"),
            )
            .into(),
        }
    }
    async fn audit(
        &mut self,
        context: &OperationContext,
        outcome: AuditOutcome,
    ) -> Result<(), Error> {
        if let Some(hooks) = &self.policy.hooks {
            hooks
                .audit(context, outcome)
                .await
                .map_err(|_| Error::Policy("audit unavailable".into()))?;
        }
        Ok(())
    }
    fn unknown(&self, method: &str) -> Error {
        Error::ReconciliationRequired {
            server: self.server.clone(),
            operation: method.into(),
        }
    }
    async fn exchange(
        &mut self,
        method: &str,
        params: Value,
        context: &OperationContext,
    ) -> Result<Value, Error> {
        self.check_server()?;
        if matches!(self.state, State::Failed | State::Closed) {
            return Err(Error::Closed);
        }
        if serde_json::to_vec(&params).map_err(|_| Error::Limit)?.len()
            > self.limits.max_message_bytes
        {
            return Err(Error::Limit);
        }
        self.audit(context, AuditOutcome::Attempted).await?;
        let previous = self.state;
        // Poison before polling: dropping an in-flight future must never permit session reuse.
        self.state = State::Failed;
        let result =
            tokio::time::timeout(self.limits.timeout, self.transport.request(method, params)).await;
        let mut reusable = matches!(
            &result,
            Ok(Ok(_)) | Ok(Err(Error::Remote { .. } | Error::Policy(_)))
        );
        let result = match result {
            Ok(Ok(value)) => {
                if serde_json::to_vec(&value).map_err(|_| Error::Limit)?.len()
                    > self.limits.max_message_bytes
                    || (method == "tools/call"
                        && (!value.get("content").is_some_and(Value::is_array)
                            || value.get("isError").is_some_and(|v| !v.is_boolean())
                            || crate::tools::validate_call_result(&value).is_err()))
                {
                    // A response that cannot convey the dispatched result is
                    // not evidence that the operation had no side effects.
                    reusable = false;
                    Err(self.unknown(method))
                } else {
                    Ok(value)
                }
            }
            Ok(Err(error @ Error::Remote { .. })) | Ok(Err(error @ Error::Policy(_))) => Err(error),
            Ok(Err(Error::ReconciliationRequired { .. })) | Err(_) => Err(self.unknown(method)),
            Ok(Err(error)) => Err(error),
        };
        let outcome = match &result {
            Ok(_) => AuditOutcome::Completed,
            Err(Error::ReconciliationRequired { .. }) => AuditOutcome::OutcomeUnknown,
            Err(_) => AuditOutcome::Failed,
        };
        if self.audit(context, outcome).await.is_err() {
            self.state = State::Failed;
            return Err(self.unknown(method));
        }
        if reusable {
            self.state = previous;
        }
        result
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let context = self.context(method, None, &params);
        self.exchange(method, params, &context).await
    }
    pub async fn initialize(&mut self) -> Result<Capabilities, Error> {
        if self.state != State::New {
            return Err(Error::Protocol(
                "session already initialized or closed".into(),
            ));
        }
        let response = self.request("initialize", json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"adk-mcp","version":env!("CARGO_PKG_VERSION")}})).await?;
        self.state = State::Failed;
        let version = response.get("protocolVersion").and_then(Value::as_str);
        if version != Some(PROTOCOL_VERSION)
            && !(version == Some("2024-11-05") && self.config.transport_type() != "streamable-http")
        {
            return Err(Error::Protocol("unsupported protocol version".into()));
        }
        let capabilities = response
            .get("capabilities")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Protocol("missing capabilities".into()))?;
        let mut negotiated = Capabilities::default();
        for (name, enabled) in [
            ("tools", &mut negotiated.tools),
            ("resources", &mut negotiated.resources),
            ("prompts", &mut negotiated.prompts),
        ] {
            if let Some(value) = capabilities.get(name) {
                if !value.is_object() {
                    return Err(Error::Protocol("malformed capability".into()));
                }
                *enabled = true;
            }
        }
        match tokio::time::timeout(
            self.limits.timeout,
            self.transport
                .notify("notifications/initialized", json!({})),
        )
        .await
        {
            Ok(Ok(())) => {}
            _ => return Err(self.unknown("notifications/initialized")),
        }
        self.capabilities = negotiated;
        self.state = State::Ready;
        Ok(negotiated)
    }
    pub async fn reconnect(
        &mut self,
        fresh_transport: Box<dyn Transport>,
    ) -> Result<Capabilities, Error> {
        self.state = State::Closed;
        let _ = tokio::time::timeout(self.limits.timeout, self.transport.close()).await;
        self.transport = fresh_transport;
        self.capabilities = Capabilities::default();
        self.tools = None;
        self.invalidate_discovery();
        self.state = State::New;
        self.initialize().await
    }
    pub async fn close(&mut self) -> Result<(), Error> {
        self.state = State::Closed;
        tokio::time::timeout(self.limits.timeout, self.transport.close())
            .await
            .map_err(|_| Error::Transport)?
    }
    async fn pages(&mut self, method: &str, field: &str) -> Result<Vec<Value>, Error> {
        self.ready()?;
        let mut cursor = None;
        let mut seen = BTreeSet::new();
        let mut items = Vec::new();
        for _ in 0..self.limits.max_pages {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |c| json!({"cursor":c}));
            let response = self.request(method, params).await?;
            let page = response
                .get(field)
                .and_then(Value::as_array)
                .ok_or_else(|| Error::Protocol("malformed discovery list".into()))?;
            if page.len() > self.limits.max_items.saturating_sub(items.len()) {
                return Err(Error::Limit);
            }
            items.extend(page.iter().cloned());
            match response.get("nextCursor") {
                None => return Ok(items),
                Some(Value::String(next)) if next.trim().is_empty() => return Ok(items),
                Some(Value::String(next)) if seen.insert(next.trim().to_owned()) => {
                    cursor = Some(next.trim().to_owned())
                }
                Some(Value::String(_)) => {
                    return Err(Error::Protocol("repeated discovery cursor".into()));
                }
                Some(_) => return Err(Error::Protocol("non-string discovery cursor".into())),
            }
        }
        Err(Error::Limit)
    }
    fn tool_allowed(&self, name: &str) -> bool {
        self.config.tool_allowed(name)
            && self
                .grant()
                .allowed_tools
                .as_ref()
                .is_none_or(|tools| tools.contains(name))
    }
    fn remote_read_only(&self, descriptor: &ToolDescriptor) -> bool {
        !self.config.is_remote()
            || (descriptor.read_only
                && self.grant().read_only_tools.contains(&descriptor.tool_name))
    }
    pub async fn list_tools(&mut self) -> Result<Vec<ToolDescriptor>, Error> {
        self.ready()?;
        if !self.capabilities.tools {
            return Ok(Vec::new());
        }
        if self.tools.is_none() {
            let items = self.pages("tools/list", "tools").await?;
            let mut tools = Vec::new();
            let mut original_names = BTreeSet::new();
            let mut qualified = BTreeSet::new();
            for item in items {
                let name = required_string(&item, "name")?;
                let description = optional_string(&item, "description")?;
                let hint = match item.get("annotations") {
                    None => false,
                    Some(Value::Object(a)) => match a.get("readOnlyHint") {
                        None => false,
                        Some(Value::Bool(b)) => *b,
                        _ => return Err(Error::Protocol("invalid read-only annotation".into())),
                    },
                    Some(_) => return Err(Error::Protocol("invalid annotations".into())),
                };
                if !original_names.insert(name.clone()) {
                    return Err(Error::Protocol("ambiguous tool name".into()));
                }
                let mut descriptor = ToolDescriptor {
                    qualified_name: qualified_tool_name(&self.server, &name),
                    server_name: self.server.clone(),
                    tool_name: name.clone(),
                    display_description: crate::tools::sanitize_description(
                        &self.server,
                        &name,
                        &description,
                    ),
                    description,
                    input_schema: normalize_input_schema(
                        item.get("inputSchema").cloned().unwrap_or(Value::Null),
                    ),
                    read_only: self.config.trust_read_only_hint() && hint,
                };
                if self.tool_allowed(&name) && self.remote_read_only(&descriptor) {
                    descriptor.qualified_name = crate::names::ensure_unique_tool_name(
                        &descriptor.qualified_name,
                        &mut qualified,
                    )
                    .ok_or_else(|| Error::Protocol("ambiguous qualified tool name".into()))?;
                }
                tools.push(descriptor);
            }
            self.tools = Some(tools);
        }
        let mut tools: Vec<_> = self
            .tools
            .as_ref()
            .expect("catalog loaded")
            .iter()
            .filter(|d| self.tool_allowed(&d.tool_name) && self.remote_read_only(d))
            .cloned()
            .collect();
        tools.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
        Ok(tools)
    }
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, Error> {
        self.call_tool_inner(name, arguments, None).await
    }
    pub async fn call_tool_with_break_glass(
        &mut self,
        name: &str,
        arguments: Value,
        reason: &str,
    ) -> Result<Value, Error> {
        self.call_tool_inner(name, arguments, Some(reason)).await
    }
    async fn call_tool_inner(
        &mut self,
        name: &str,
        arguments: Value,
        reason: Option<&str>,
    ) -> Result<Value, Error> {
        self.ready()?;
        if !arguments.is_object() {
            return Err(Error::Protocol("tool arguments must be an object".into()));
        }
        let descriptor = self
            .tools
            .as_ref()
            .and_then(|tools| tools.iter().find(|tool| tool.tool_name == name))
            .cloned()
            .ok_or_else(|| Error::Policy("tool not discovered".into()))?;
        let context = self.context("tools/call", Some(name), &arguments);
        if !self.remote_read_only(&descriptor) {
            self.audit(&context, AuditOutcome::Denied).await?;
            return Err(Error::Policy(
                "remote tool is not host-approved read-only".into(),
            ));
        }
        if !self.tool_allowed(name) {
            let approved = match (reason.filter(|r| !r.trim().is_empty()), &self.policy.hooks) {
                (Some(reason), Some(hooks)) => hooks
                    .break_glass(&context, reason)
                    .await
                    .map_err(|_| Error::Policy("break-glass unavailable".into()))?,
                _ => false,
            };
            if !approved {
                self.audit(&context, AuditOutcome::Denied).await?;
                return Err(Error::Policy("tool blocked by allowlist".into()));
            }
        }
        if !descriptor.read_only {
            let approved = match &self.policy.hooks {
                Some(hooks) => hooks
                    .approve(&context)
                    .await
                    .map_err(|_| Error::Policy("approval unavailable".into()))?,
                None => false,
            };
            if !approved {
                self.audit(&context, AuditOutcome::Denied).await?;
                return Err(Error::Policy("tool approval required".into()));
            }
        }
        let result = self
            .exchange(
                "tools/call",
                json!({"name":name,"arguments":arguments}),
                &context,
            )
            .await?;
        Ok(result)
    }
    pub async fn list_resources(&mut self) -> Result<Vec<ResourceDescriptor>, Error> {
        self.ready()?;
        if !self.capabilities.resources {
            return Ok(Vec::new());
        }
        if self
            .resources_discovered_at
            .is_none_or(|at| at.elapsed() >= self.discovery_cache_ttl)
        {
            self.resources = None;
            self.resources_discovered_at = None;
            let discovered_at = Instant::now();
            let items = self.pages("resources/list", "resources").await?;
            let mut resources = BTreeMap::new();
            for item in items {
                let mut descriptor: ResourceDescriptor = serde_json::from_value(item)
                    .map_err(|_| Error::Protocol("malformed resource descriptor".into()))?;
                if descriptor.uri.is_empty() || descriptor.name.is_empty() {
                    return Err(Error::Protocol("empty resource identity".into()));
                }
                descriptor.server = self.server.clone();
                if resources
                    .insert(descriptor.uri.clone(), descriptor)
                    .is_some()
                {
                    return Err(Error::Protocol("ambiguous resource URI".into()));
                }
            }
            self.resources = Some(resources);
            self.resources_discovered_at = Some(discovered_at);
        }
        Ok(self
            .resources
            .as_ref()
            .expect("catalog loaded")
            .values()
            .filter(|d| {
                self.grant()
                    .allowed_resources
                    .as_ref()
                    .is_none_or(|set| set.contains(&d.uri))
            })
            .cloned()
            .collect())
    }
    pub async fn read_resource(&mut self, uri: &str) -> Result<Value, Error> {
        self.ready()?;
        if !self.resources.as_ref().is_some_and(|r| r.contains_key(uri))
            || self
                .grant()
                .allowed_resources
                .as_ref()
                .is_some_and(|r| !r.contains(uri))
        {
            return Err(Error::Policy("resource not discovered or allowed".into()));
        }
        let result = self.request("resources/read", json!({"uri":uri})).await?;
        if !result.get("contents").is_some_and(Value::is_array) {
            return Err(Error::Protocol("malformed resource result".into()));
        }
        Ok(result)
    }
    pub async fn list_prompts(&mut self) -> Result<Vec<PromptDescriptor>, Error> {
        self.ready()?;
        if !self.capabilities.prompts {
            return Ok(Vec::new());
        }
        if self
            .prompts_discovered_at
            .is_none_or(|at| at.elapsed() >= self.discovery_cache_ttl)
        {
            self.prompts = None;
            self.prompts_discovered_at = None;
            let discovered_at = Instant::now();
            let items = self.pages("prompts/list", "prompts").await?;
            let mut prompts = BTreeMap::new();
            for item in items {
                let mut descriptor: PromptDescriptor = serde_json::from_value(item)
                    .map_err(|_| Error::Protocol("malformed prompt descriptor".into()))?;
                let mut names = BTreeSet::new();
                if descriptor.name.is_empty()
                    || descriptor
                        .arguments
                        .iter()
                        .any(|a| a.name.is_empty() || !names.insert(a.name.clone()))
                {
                    return Err(Error::Protocol(
                        "invalid prompt identity or arguments".into(),
                    ));
                }
                descriptor.server = self.server.clone();
                if prompts
                    .insert(descriptor.name.clone(), descriptor)
                    .is_some()
                {
                    return Err(Error::Protocol("ambiguous prompt name".into()));
                }
            }
            self.prompts = Some(prompts);
            self.prompts_discovered_at = Some(discovered_at);
        }
        Ok(self
            .prompts
            .as_ref()
            .expect("catalog loaded")
            .values()
            .filter(|d| {
                self.grant()
                    .allowed_prompts
                    .as_ref()
                    .is_none_or(|set| set.contains(&d.name))
            })
            .cloned()
            .collect())
    }
    pub async fn get_prompt(&mut self, name: &str, arguments: Value) -> Result<Value, Error> {
        self.ready()?;
        let descriptor = self
            .prompts
            .as_ref()
            .and_then(|p| p.get(name))
            .ok_or_else(|| Error::Policy("prompt not discovered".into()))?;
        if self
            .grant()
            .allowed_prompts
            .as_ref()
            .is_some_and(|p| !p.contains(name))
        {
            return Err(Error::Policy("prompt blocked by host".into()));
        }
        let args = arguments
            .as_object()
            .ok_or_else(|| Error::Protocol("prompt arguments must be an object".into()))?;
        if args.values().any(|v| !v.is_string())
            || descriptor
                .arguments
                .iter()
                .any(|a| a.required && !args.contains_key(&a.name))
            || args
                .keys()
                .any(|name| !descriptor.arguments.iter().any(|a| &a.name == name))
        {
            return Err(Error::Protocol("invalid prompt arguments".into()));
        }
        let result = self
            .request("prompts/get", json!({"name":name,"arguments":arguments}))
            .await?;
        if !result.get("messages").is_some_and(Value::is_array) {
            return Err(Error::Protocol("malformed prompt result".into()));
        }
        Ok(result)
    }
}

fn required_string(value: &Value, field: &str) -> Result<String, Error> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Error::Protocol(format!("missing or invalid {field}")))
}
fn optional_string(value: &Value, field: &str) -> Result<String, Error> {
    match value.get(field) {
        None => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(Error::Protocol(format!("invalid {field}"))),
    }
}
pub fn normalize_input_schema(schema: Value) -> Value {
    let Value::Object(mut object) = schema else {
        return json!({"type":"object","properties":{},"additionalProperties":true});
    };
    if object
        .get("type")
        .and_then(Value::as_str)
        .is_none_or(|s| s.is_empty() || s == "object")
    {
        object.insert("type".into(), "object".into());
        object.entry("properties").or_insert_with(|| json!({}));
    }
    Value::Object(object)
}
pub fn qualified_tool_name(server: &str, tool: &str) -> String {
    crate::names::qualified_tool_name(server, tool)
}

pub struct ClientManager {
    clients: BTreeMap<String, tokio::sync::Mutex<Client>>,
    definitions: Vec<adk_core::ToolDefinition>,
    routes: BTreeMap<String, (String, String)>,
    resources: bool,
}
impl ClientManager {
    pub async fn list_prompts(&self, server: Option<&str>) -> Result<Vec<PromptDescriptor>, Error> {
        if server.is_some_and(|s| !self.clients.contains_key(s)) {
            return Err(Error::Policy("unknown server".into()));
        }
        let mut prompts = Vec::new();
        let mut error = None;
        for (name, client) in &self.clients {
            if server.is_none_or(|s| s == name) {
                let mut client = client.lock().await;
                if !client.capabilities.prompts {
                    if server.is_some() {
                        return Err(Error::Protocol("server does not support prompts".into()));
                    }
                    continue;
                }
                match client.list_prompts().await {
                    Ok(items) => prompts.extend(items),
                    Err(e @ Error::ReconciliationRequired { .. }) => return Err(e),
                    Err(e) if server.is_some() => return Err(e),
                    Err(e) => {
                        error.get_or_insert(e);
                    }
                }
            }
        }
        if prompts.is_empty() {
            if let Some(error) = error {
                return Err(error);
            }
        }
        Ok(prompts)
    }
    pub async fn get_prompt(
        &self,
        server: &str,
        name: &str,
        arguments: Value,
    ) -> Result<Value, Error> {
        let client = self
            .clients
            .get(server)
            .ok_or_else(|| Error::Policy("unknown server".into()))?;
        let mut client = client.lock().await;
        if !client.capabilities.prompts {
            return Err(Error::Protocol("server does not support prompts".into()));
        }
        client.get_prompt(name, arguments).await
    }
    /// Clears resource/prompt discovery for one server, or every server when empty.
    pub async fn invalidate_discovery(&self, server: &str) {
        for (name, client) in &self.clients {
            if server.is_empty() || server == name {
                client.lock().await.invalidate_discovery();
            }
        }
    }
    pub async fn close(&self) -> Result<(), Error> {
        let mut error = None;
        for client in self.clients.values() {
            if let Err(e) = client.lock().await.close().await {
                error.get_or_insert(e);
            }
        }
        error.map_or(Ok(()), Err)
    }
    pub async fn reconnect(
        &self,
        server: &str,
        fresh_transport: Box<dyn Transport>,
    ) -> Result<(), Error> {
        let client = self
            .clients
            .get(server)
            .ok_or_else(|| Error::Policy("unknown server".into()))?;
        let mut client = client.lock().await;
        let old_tools = client.tools.clone();
        let old_capabilities = client.capabilities;
        client.reconnect(fresh_transport).await?;
        let result = client.list_tools().await;
        if result.is_err() || client.tools != old_tools || client.capabilities != old_capabilities {
            let _ = client.close().await;
            return Err(Error::Protocol(
                "reconnected capability catalog changed; construct a new manager".into(),
            ));
        }
        Ok(())
    }
    pub async fn new(mut clients: Vec<Client>) -> Result<Self, Error> {
        let mut manager = Self {
            clients: BTreeMap::new(),
            definitions: Vec::new(),
            routes: BTreeMap::new(),
            resources: false,
        };
        clients.sort_by(|a, b| a.server.cmp(&b.server));
        let mut qualified = BTreeSet::new();
        for mut client in clients {
            if manager.clients.contains_key(client.server_name()) {
                return Err(Error::Config("duplicate server name".into()));
            }
            if client.state == State::New {
                client.initialize().await?;
            }
            client.list_tools().await?;
            for tool in client.tools.iter().flatten().filter(|tool| {
                client.tool_allowed(&tool.tool_name) && client.remote_read_only(tool)
            }) {
                let name = crate::names::ensure_unique_tool_name(
                    &qualified_tool_name(&tool.server_name, &tool.tool_name),
                    &mut qualified,
                )
                .ok_or_else(|| Error::Protocol("ambiguous qualified tool name".into()))?;
                manager.routes.insert(
                    name.clone(),
                    (tool.server_name.clone(), tool.tool_name.clone()),
                );
                manager.definitions.push(adk_core::ToolDefinition {
                    name,
                    description: tool.display_description.clone(),
                    input_schema: tool
                        .input_schema
                        .clone()
                        .try_into()
                        .map_err(|_| Error::Protocol("invalid tool schema".into()))?,
                    read_only: tool.read_only,
                    requires_approval: !tool.read_only,
                });
            }
            manager.resources |= client.capabilities.resources;
            manager
                .clients
                .insert(client.server.clone(), tokio::sync::Mutex::new(client));
        }
        manager.definitions.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(manager)
    }
}
impl crate::tools::ToolManager for ClientManager {
    fn definitions(&self) -> Vec<adk_core::ToolDefinition> {
        self.definitions.clone()
    }
    fn has_resources(&self) -> bool {
        self.resources
    }
    fn call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            let (server, tool) = self
                .routes
                .get(name)
                .ok_or_else(|| Error::Policy("unknown qualified tool".into()))?;
            self.clients[server]
                .lock()
                .await
                .call_tool(tool, arguments)
                .await
        })
    }
    fn list_resources<'a>(
        &'a self,
        server: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            if server.is_some_and(|s| !self.clients.contains_key(s)) {
                return Err(Error::Policy("unknown server".into()));
            }
            let mut resources = Vec::new();
            let mut error = None;
            for (name, client) in &self.clients {
                if server.is_none_or(|s| s == name) {
                    let mut client = client.lock().await;
                    if !client.capabilities.resources {
                        if server.is_some() {
                            return Err(Error::Protocol(
                                "server does not support resources".into(),
                            ));
                        }
                        continue;
                    }
                    match client.list_resources().await {
                        Ok(items) => resources.extend(items),
                        Err(e @ Error::ReconciliationRequired { .. }) => return Err(e),
                        Err(e) if server.is_some() => return Err(e),
                        Err(e) => {
                            error.get_or_insert(e);
                        }
                    }
                }
            }
            if resources.is_empty() {
                if let Some(error) = error {
                    return Err(error);
                }
            }
            serde_json::to_value(resources)
                .map_err(|_| Error::Protocol("invalid resource catalog".into()))
        })
    }
    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            let client = self
                .clients
                .get(server)
                .ok_or_else(|| Error::Policy("unknown server".into()))?;
            client.lock().await.read_resource(uri).await
        })
    }
}
