use crate::{BoxFuture, Error, Limits, Transport};
use reqwest::{
    Response,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    process::Stdio,
    sync::Arc,
    time::SystemTime,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    task::JoinHandle,
    time::timeout,
};
use url::Url;

fn validate_limits(limits: &Limits) -> Result<(), Error> {
    if limits.max_message_bytes == 0 || limits.max_items == 0 || limits.timeout.is_zero() {
        return Err(Error::Config("nonzero transport limits required".into()));
    }
    Ok(())
}

fn unknown(server: &str, method: &str) -> Error {
    Error::ReconciliationRequired {
        server: server.into(),
        operation: method.into(),
    }
}

fn message(method: &str, params: Value, id: Option<u64>, limit: usize) -> Result<Vec<u8>, Error> {
    let mut value = json!({"jsonrpc":"2.0", "method":method, "params":params});
    if let Some(id) = id {
        value["id"] = id.into();
    }
    let bytes =
        serde_json::to_vec(&value).map_err(|_| Error::Protocol("invalid request".into()))?;
    if bytes.len() > limit {
        return Err(Error::Limit);
    }
    Ok(bytes)
}

fn reply(value: &Value, id: u64) -> Result<Value, Error> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || value.get("id").and_then(Value::as_u64) != Some(id)
        || value.get("method").is_some()
    {
        return Err(Error::Protocol("unexpected response".into()));
    }
    match (value.get("result"), value.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) if error.get("message").is_some_and(Value::is_string) => error
            .get("code")
            .and_then(Value::as_i64)
            .map(|code| Err(Error::Remote { code }))
            .unwrap_or_else(|| Err(Error::Protocol("invalid remote error".into()))),
        _ => Err(Error::Protocol("invalid response".into())),
    }
}

struct Incoming {
    response: Option<Result<Value, Error>>,
    replies: Option<Value>,
}

fn incoming(value: &Value, id: u64, remaining: &mut usize) -> Result<Incoming, Error> {
    let batch = value.is_array();
    let items = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value));
    if items.is_empty() || items.len() > *remaining {
        return Err(Error::Limit);
    }
    *remaining -= items.len();
    let mut response = None;
    let mut replies = Vec::new();
    for value in items {
        if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(Error::Protocol("invalid JSON-RPC message".into()));
        }
        if let Some(method) = value.get("method") {
            let method = method
                .as_str()
                .ok_or_else(|| Error::Protocol("invalid method".into()))?;
            if value.get("result").is_some()
                || value.get("error").is_some()
                || value
                    .get("params")
                    .is_some_and(|p| !p.is_object() && !p.is_array())
            {
                return Err(Error::Protocol("invalid peer request".into()));
            }
            if let Some(peer_id) = value.get("id") {
                if !(peer_id.is_string() || peer_id.is_i64() || peer_id.is_u64())
                    || replies.iter().any(|r: &Value| r.get("id") == Some(peer_id))
                {
                    return Err(Error::Protocol("invalid peer request id".into()));
                }
                replies.push(if method == "ping" {
                    json!({"jsonrpc":"2.0", "id":peer_id, "result":{}})
                } else {
                    json!({"jsonrpc":"2.0", "id":peer_id,
                        "error":{"code":-32601,"message":"Method not found"}})
                });
            }
        } else {
            if response.is_some() {
                return Err(Error::Protocol("duplicate response".into()));
            }
            let result = reply(value, id);
            if !matches!(result, Ok(_) | Err(Error::Remote { .. })) {
                return Err(Error::Protocol("invalid response".into()));
            }
            response = Some(result);
        }
    }
    Ok(Incoming {
        response,
        replies: if replies.is_empty() {
            None
        } else if batch {
            Some(Value::Array(replies))
        } else {
            replies.pop()
        },
    })
}

fn peer_reply_bytes(value: &Value, limit: usize) -> Result<Vec<u8>, Error> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Protocol("invalid reply".into()))?;
    if bytes.len() > limit {
        return Err(Error::Limit);
    }
    Ok(bytes)
}

pub struct StdioTransport {
    server: String,
    session: Option<StdioSession>,
    limits: Limits,
    next_id: u64,
    poisoned: bool,
    closed: bool,
}

struct StdioSession {
    child: Option<Child>,
    #[cfg(unix)]
    process_group: Option<rustix::process::Pid>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: JoinHandle<()>,
}

impl StdioTransport {
    pub async fn connect(
        server: &str,
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
        limits: Limits,
    ) -> Result<Self, Error> {
        validate_limits(&limits)?;
        let mut command_builder = Command::new(command);
        #[cfg(unix)]
        command_builder.process_group(0);
        command_builder
            .args(args)
            .env_clear()
            .envs(env)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command_builder.spawn().map_err(|_| Error::Transport)?;
        #[cfg(unix)]
        let process_group = child
            .id()
            .and_then(|id| rustix::process::Pid::from_raw(id as i32));
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().ok_or(Error::Transport)?);
        let mut stderr_pipe = child.stderr.take().ok_or(Error::Transport)?;
        let stderr_limit = limits.max_stderr_bytes;
        let stderr = tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            let mut diagnostic = Vec::new();
            while let Ok(size) = stderr_pipe.read(&mut buffer).await {
                if size == 0 {
                    break;
                }
                // Never surface server-controlled diagnostics in errors or logs; keep only a bounded, control-free prefix.
                for byte in buffer[..size].iter().copied().filter(u8::is_ascii_graphic) {
                    if diagnostic.len() < stderr_limit {
                        diagnostic.push(byte);
                    }
                }
            }
        });
        Ok(Self {
            server: server.into(),
            session: Some(StdioSession {
                child: Some(child),
                #[cfg(unix)]
                process_group,
                stdin,
                stdout,
                stderr,
            }),
            limits,
            next_id: 1,
            poisoned: false,
            closed: false,
        })
    }

    async fn exchange(
        &mut self,
        method: &str,
        params: Value,
        response: bool,
    ) -> Result<Value, Error> {
        if self.closed || self.poisoned {
            return Err(Error::Closed);
        }
        let id = self.next_id;
        let mut bytes = message(
            method,
            params,
            response.then_some(id),
            self.limits.max_message_bytes,
        )?;
        bytes.push(b'\n');
        self.next_id = self.next_id.checked_add(1).ok_or(Error::Limit)?;
        // A dropped future leaves the session poisoned, including partially written requests.
        self.poisoned = true;
        let mut session = self.session.take().ok_or(Error::Closed)?;
        let result = timeout(self.limits.timeout, async {
            let stdin = session.stdin.as_mut().ok_or(Error::Closed)?;
            stdin
                .write_all(&bytes)
                .await
                .map_err(|_| Error::Transport)?;
            stdin.flush().await.map_err(|_| Error::Transport)?;
            if !response {
                return Ok(Value::Null);
            }
            let mut remaining = self.limits.max_items;
            while remaining > 0 {
                let mut line = Vec::new();
                loop {
                    let available = session
                        .stdout
                        .fill_buf()
                        .await
                        .map_err(|_| Error::Transport)?;
                    if available.is_empty() {
                        return Err(Error::Closed);
                    }
                    let count = available
                        .iter()
                        .position(|b| *b == b'\n')
                        .map_or(available.len(), |i| i + 1);
                    if line.len().saturating_add(count) > self.limits.max_message_bytes {
                        return Err(Error::Limit);
                    }
                    line.extend_from_slice(&available[..count]);
                    session.stdout.consume(count);
                    if line.last() == Some(&b'\n') {
                        break;
                    }
                }
                let value: Value = serde_json::from_slice(&line)
                    .map_err(|_| Error::Protocol("invalid JSON".into()))?;
                let incoming = incoming(&value, id, &mut remaining)?;
                if let Some(replies) = incoming.replies {
                    let mut bytes = peer_reply_bytes(&replies, self.limits.max_message_bytes)?;
                    bytes.push(b'\n');
                    stdin
                        .write_all(&bytes)
                        .await
                        .map_err(|_| Error::Transport)?;
                    stdin.flush().await.map_err(|_| Error::Transport)?;
                }
                if let Some(response) = incoming.response {
                    return response;
                }
            }
            Err(Error::Limit)
        })
        .await;
        match result {
            Ok(Ok(value)) => {
                self.session = Some(session);
                self.poisoned = false;
                Ok(value)
            }
            Ok(Err(error @ Error::Remote { .. })) => {
                self.session = Some(session);
                self.poisoned = false;
                Err(error)
            }
            _ => {
                let _ = session.kill();
                let _ = timeout(self.limits.timeout, session.child.as_mut().unwrap().wait()).await;
                Err(unknown(&self.server, method))
            }
        }
    }
}

impl Transport for StdioTransport {
    fn request<'a>(
        &'a mut self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(self.exchange(method, params, true))
    }
    fn notify<'a>(
        &'a mut self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move { self.exchange(method, params, false).await.map(|_| ()) })
    }
    fn close(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            self.poisoned = true;
            self.closed = true;
            if let Some(mut session) = self.session.take() {
                session.kill()?;
                timeout(self.limits.timeout, session.child.as_mut().unwrap().wait())
                    .await
                    .map_err(|_| Error::Transport)?
                    .map_err(|_| Error::Transport)?;
            }
            Ok(())
        })
    }
}

impl StdioSession {
    fn kill(&mut self) -> Result<(), Error> {
        self.stdin.take();
        self.stderr.abort();
        #[cfg(unix)]
        if let Some(group) = self.process_group.take() {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
        self.child
            .as_mut()
            .unwrap()
            .start_kill()
            .map_err(|_| Error::Transport)
    }
}

impl Drop for StdioSession {
    fn drop(&mut self) {
        let _ = self.kill();
        if let Some(mut child) = self.child.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
}

pub trait HeaderProvider: Send + Sync {
    fn headers<'a>(
        &'a self,
        tenant: &'a str,
        server: &'a str,
        endpoint: &'a Url,
    ) -> BoxFuture<'a, Result<HeaderMap, Error>>;
}

pub struct OAuthToken {
    pub access_token: String,
    pub audience: String,
    pub scopes: Vec<String>,
    pub expiry: SystemTime,
}

pub trait OAuthTokenProvider: Send + Sync {
    fn token<'a>(
        &'a self,
        tenant: &'a str,
        server: &'a str,
    ) -> BoxFuture<'a, Result<OAuthToken, Error>>;
}

#[derive(Clone)]
pub struct OAuthPolicy {
    pub provider: Arc<dyn OAuthTokenProvider>,
    pub audience: String,
    pub required_scopes: Vec<String>,
}

#[derive(Clone, Default)]
pub struct RemoteOptions {
    pub tenant_id: String,
    pub allow_private_network: bool,
    pub headers: Option<Arc<dyn HeaderProvider>>,
    pub oauth: Option<OAuthPolicy>,
    /// Additional host-owned CA certificates. Certificate and hostname verification
    /// remain enabled; repository configuration cannot install trust roots.
    pub root_certificates: Vec<reqwest::Certificate>,
}

fn validate_url(url: &Url, private: bool, endpoint: bool) -> Result<(), Error> {
    if !url.username().is_empty()
        || url.password().is_some()
        || (!endpoint && url.query().is_some())
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(Error::Policy(
            "remote URL must have a host and no userinfo, query, or fragment".into(),
        ));
    }
    if url.scheme() != "https" && !(private && url.scheme() == "http") {
        return Err(Error::Policy("HTTPS required".into()));
    }
    Ok(())
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Only global unicast, excluding transition, documentation and special-purpose ranges.
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

struct SseReader {
    response: Response,
    buffer: Vec<u8>,
    event: String,
    data: Vec<u8>,
    total: usize,
    limit: usize,
    pending_cr: bool,
}

impl SseReader {
    fn new(response: Response, limit: usize) -> Self {
        Self {
            response,
            buffer: Vec::new(),
            event: String::new(),
            data: Vec::new(),
            total: 0,
            limit,
            pending_cr: false,
        }
    }
    async fn next(&mut self) -> Result<(String, String), Error> {
        loop {
            if let Some(end) = self.buffer.iter().position(|b| *b == b'\n' || *b == b'\r') {
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                let delimiter = line[end];
                if self.pending_cr && end == 0 && delimiter == b'\n' {
                    self.pending_cr = false;
                    continue;
                }
                self.pending_cr = delimiter == b'\r';
                let line = std::str::from_utf8(&line[..end])
                    .map_err(|_| Error::Protocol("invalid SSE UTF-8".into()))?;
                if line.is_empty() {
                    if self.data.is_empty() {
                        self.event.clear();
                        continue;
                    }
                    self.data.pop();
                    let data = String::from_utf8(std::mem::take(&mut self.data))
                        .map_err(|_| Error::Protocol("invalid SSE data".into()))?;
                    return Ok((std::mem::take(&mut self.event), data));
                }
                let (field, value) = line.split_once(':').unwrap_or((line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "event" => self.event = value.into(),
                    "data" => {
                        self.data.extend_from_slice(value.as_bytes());
                        self.data.push(b'\n');
                    }
                    _ => {}
                }
            } else {
                let chunk = self
                    .response
                    .chunk()
                    .await
                    .map_err(|_| Error::Transport)?
                    .ok_or(Error::Closed)?;
                self.total = self.total.checked_add(chunk.len()).ok_or(Error::Limit)?;
                if self.total > self.limit {
                    return Err(Error::Limit);
                }
                self.buffer.extend_from_slice(&chunk);
            }
        }
    }
}

pub struct HttpTransport {
    server: String,
    url: Url,
    options: RemoteOptions,
    limits: Limits,
    legacy: Option<SseReader>,
    legacy_mode: bool,
    credential_history: Vec<String>,
    post_url: Url,
    session: Option<HeaderValue>,
    next_id: u64,
    poisoned: bool,
    closed: bool,
}

impl HttpTransport {
    pub async fn connect(
        server: &str,
        url: &str,
        legacy_sse: bool,
        options: RemoteOptions,
        limits: Limits,
    ) -> Result<Self, Error> {
        validate_limits(&limits)?;
        if options.tenant_id.trim().is_empty() {
            return Err(Error::Policy("remote tenant required".into()));
        }
        if options
            .oauth
            .as_ref()
            .is_some_and(|p| p.audience.trim().is_empty())
        {
            return Err(Error::Policy("OAuth audience required".into()));
        }
        let url = Url::parse(url).map_err(|_| Error::Config("invalid remote URL".into()))?;
        validate_url(&url, options.allow_private_network, false)?;
        let mut transport = Self {
            server: server.into(),
            post_url: url.clone(),
            url,
            options,
            limits,
            legacy: None,
            legacy_mode: legacy_sse,
            credential_history: Vec::new(),
            session: None,
            next_id: 1,
            poisoned: false,
            closed: false,
        };
        timeout(transport.limits.timeout, async {
            if legacy_sse {
                let (builder, secrets) = transport
                    .prepare(reqwest::Method::GET, &transport.url, None)
                    .await?;
                remember_credentials(
                    &mut transport.credential_history,
                    secrets,
                    &transport.limits,
                )?;
                let response = builder.send().await.map_err(|_| Error::Transport)?;
                if !response.status().is_success() || content_type(&response) != "text/event-stream"
                {
                    return Err(Error::Protocol("expected SSE stream".into()));
                }
                let mut reader = SseReader::new(response, transport.limits.max_message_bytes);
                let (event, data) = reader.next().await?;
                if event != "endpoint" {
                    return Err(Error::Protocol("expected SSE endpoint".into()));
                }
                let endpoint = transport
                    .url
                    .join(&data)
                    .map_err(|_| Error::Protocol("invalid SSE endpoint".into()))?;
                validate_url(&endpoint, transport.options.allow_private_network, true)?;
                if endpoint.origin() != transport.url.origin() {
                    return Err(Error::Policy("cross-origin SSE endpoint denied".into()));
                }
                remember_credentials(
                    &mut transport.credential_history,
                    endpoint
                        .query_pairs()
                        .map(|(_, value)| value.into_owned())
                        .chain(
                            endpoint
                                .query()
                                .into_iter()
                                .flat_map(|query| query.split('&'))
                                .filter_map(|pair| {
                                    pair.split_once('=').map(|(_, value)| value.to_owned())
                                }),
                        )
                        .collect(),
                    &transport.limits,
                )?;
                transport.post_url = endpoint;
                transport.legacy = Some(reader);
            } else {
                transport.checked_client(&transport.url).await?;
            }
            Ok::<(), Error>(())
        })
        .await
        .map_err(|_| Error::Transport)??;
        Ok(transport)
    }

    async fn checked_client(&self, url: &Url) -> Result<reqwest::Client, Error> {
        validate_url(
            url,
            self.options.allow_private_network,
            url == &self.post_url && self.legacy_mode,
        )?;
        let host = url
            .host_str()
            .ok_or_else(|| Error::Policy("missing host".into()))?
            .trim_start_matches('[')
            .trim_end_matches(']');
        let port = url
            .port_or_known_default()
            .ok_or_else(|| Error::Policy("missing port".into()))?;
        let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| Error::Transport)?
            .collect();
        if addresses.is_empty() {
            return Err(Error::Transport);
        }
        if !self.options.allow_private_network && addresses.iter().any(|a| !public_ip(a.ip())) {
            return Err(Error::Policy("non-public remote address denied".into()));
        }
        // A fresh client pins this request's checked DNS result; no pooled connection or implicit replay survives.
        let mut client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .http1_only()
            .pool_max_idle_per_host(0)
            .connect_timeout(self.limits.timeout)
            .read_timeout(self.limits.timeout)
            .resolve_to_addrs(host, &addresses);
        for certificate in &self.options.root_certificates {
            client = client.add_root_certificate(certificate.clone());
        }
        let client = client.build().map_err(|_| Error::Transport)?;
        Ok(client)
    }

    async fn prepare(
        &self,
        method: reqwest::Method,
        url: &Url,
        session: Option<&HeaderValue>,
    ) -> Result<(reqwest::RequestBuilder, Vec<String>), Error> {
        let client = self.checked_client(url).await?;
        let mut provider_url = url.clone();
        provider_url.set_query(None);
        let mut headers = if let Some(provider) = &self.options.headers {
            provider
                .headers(&self.options.tenant_id, &self.server, &provider_url)
                .await
                .map_err(|_| Error::Policy("header provider rejected request".into()))?
        } else {
            HeaderMap::new()
        };
        for name in headers.keys() {
            if matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "proxy-authorization"
                    | "proxy-connection"
                    | "upgrade"
                    | "te"
                    | "trailer"
                    | "mcp-session-id"
                    | "mcp-protocol-version"
                    | "content-type"
                    | "accept"
                    | "last-event-id"
                    | "cookie"
                    | "user-agent"
                    | "origin"
                    | "referer"
                    | "forwarded"
                    | "x-forwarded-for"
                    | "x-forwarded-host"
                    | "x-forwarded-proto"
                    | "x-original-url"
                    | "x-rewrite-url"
                    | "idempotency-key"
                    | "x-idempotency-key"
            ) {
                return Err(Error::Policy("reserved remote header".into()));
            }
        }
        if let Some(policy) = &self.options.oauth {
            let token = policy
                .provider
                .token(&self.options.tenant_id, &self.server)
                .await
                .map_err(|_| Error::Policy("OAuth provider rejected request".into()))?;
            if token.audience != policy.audience
                || token.expiry <= SystemTime::now()
                || token.access_token.is_empty()
                || policy
                    .required_scopes
                    .iter()
                    .any(|s| !token.scopes.contains(s))
                || headers.contains_key(reqwest::header::AUTHORIZATION)
            {
                return Err(Error::Policy("OAuth claims rejected".into()));
            }
            let mut value = HeaderValue::from_str(&format!("Bearer {}", token.access_token))
                .map_err(|_| Error::Policy("invalid OAuth token".into()))?;
            value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        for value in headers.values_mut() {
            value.set_sensitive(true);
        }
        let mut secrets = Vec::new();
        let mut header_bytes = 0usize;
        for value in headers.values() {
            header_bytes = header_bytes.saturating_add(value.as_bytes().len());
            if header_bytes > self.limits.max_message_bytes {
                return Err(Error::Limit);
            }
            let value = value
                .to_str()
                .map_err(|_| Error::Policy("non-text credential header".into()))?;
            if !value.is_empty() {
                secrets.push(value.to_owned());
            }
            if let Some((_, token)) = value.split_once(' ')
                && !token.is_empty()
            {
                secrets.push(token.to_owned());
            }
        }
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        headers.insert(
            HeaderName::from_static("mcp-protocol-version"),
            HeaderValue::from_static(crate::PROTOCOL_VERSION),
        );
        if let Some(session) = session {
            secrets.push(
                session
                    .to_str()
                    .map_err(|_| Error::Protocol("invalid session".into()))?
                    .to_owned(),
            );
            headers.insert(HeaderName::from_static("mcp-session-id"), session.clone());
        }
        Ok((
            client.request(method, url.clone()).headers(headers),
            secrets,
        ))
    }

    async fn exchange(
        &mut self,
        method: &str,
        params: Value,
        response_expected: bool,
    ) -> Result<Value, Error> {
        if self.closed || self.poisoned {
            return Err(Error::Closed);
        }
        let id = self.next_id;
        let bytes = message(
            method,
            params,
            response_expected.then_some(id),
            self.limits.max_message_bytes,
        )?;
        self.next_id = self.next_id.checked_add(1).ok_or(Error::Limit)?;
        self.poisoned = true;
        let mut legacy = self.legacy.take();
        // Exchange-local ownership releases the stream and history if this future is cancelled.
        let mut history = std::mem::take(&mut self.credential_history);
        let prepared = timeout(self.limits.timeout, async {
            let (builder, secrets) = self
                .prepare(reqwest::Method::POST, &self.post_url, self.session.as_ref())
                .await?;
            remember_credentials(&mut history, secrets, &self.limits)?;
            Ok::<_, Error>(builder)
        })
        .await
        .map_err(|_| Error::Transport)
        .and_then(|result| result);
        let builder = match prepared {
            Ok(builder) => builder,
            Err(error) => {
                self.legacy = legacy;
                self.credential_history = history;
                self.poisoned = false;
                return Err(error);
            }
        };
        let result = timeout(self.limits.timeout, async {
            let response = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes)
                .send()
                .await
                .map_err(|_| Error::Transport)?;
            if !response.status().is_success() {
                return Err(Error::Transport);
            }
            self.accept_session(&response, method == "initialize", &mut history)?;
            if !response_expected {
                read_body(response, self.limits.max_message_bytes).await?;
                return Ok(Value::Null);
            }
            if let Some(reader) = &mut legacy {
                read_body(response, self.limits.max_message_bytes).await?;
                reader.total = reader.buffer.len().saturating_add(reader.data.len());
                return self.sse_reply(reader, id, &mut history).await;
            }
            match content_type(&response) {
                "application/json" => {
                    let bytes = read_body(response, self.limits.max_message_bytes).await?;
                    let value = serde_json::from_slice(&bytes)
                        .map_err(|_| Error::Protocol("invalid JSON".into()))?;
                    let mut remaining = self.limits.max_items;
                    self.handle_incoming(value, id, &mut remaining, &mut history)
                        .await?
                        .ok_or_else(|| Error::Protocol("missing response".into()))?
                }
                "text/event-stream" => {
                    self.sse_reply(
                        &mut SseReader::new(response, self.limits.max_message_bytes),
                        id,
                        &mut history,
                    )
                    .await
                }
                _ => Err(Error::Protocol("unsupported response type".into())),
            }
        })
        .await;
        match result {
            Ok(Ok(value)) => {
                self.legacy = legacy;
                self.credential_history = history;
                self.poisoned = false;
                Ok(value)
            }
            Ok(Err(error @ Error::Remote { .. })) => {
                self.legacy = legacy;
                self.credential_history = history;
                self.poisoned = false;
                Err(error)
            }
            _ => Err(unknown(&self.server, method)),
        }
    }

    fn accept_session(
        &mut self,
        response: &Response,
        initialize: bool,
        history: &mut Vec<String>,
    ) -> Result<(), Error> {
        if let Some(session) = response.headers().get("mcp-session-id") {
            if self.session.as_ref().is_some_and(|old| old != session)
                || (self.session.is_none() && !initialize)
                || session.as_bytes().is_empty()
                || !session.as_bytes().iter().all(|b| (0x21..=0x7e).contains(b))
            {
                return Err(Error::Protocol("unexpected session id".into()));
            }
            remember_credentials(
                history,
                vec![
                    session
                        .to_str()
                        .map_err(|_| Error::Protocol("invalid session".into()))?
                        .to_owned(),
                ],
                &self.limits,
            )?;
            let mut session = session.clone();
            session.set_sensitive(true);
            self.session = Some(session);
        }
        Ok(())
    }

    async fn handle_incoming(
        &mut self,
        value: Value,
        id: u64,
        remaining: &mut usize,
        history: &mut Vec<String>,
    ) -> Result<Option<Result<Value, Error>>, Error> {
        check_reflection(&value, history)?;
        let incoming = incoming(&value, id, remaining)?;
        if let Some(replies) = incoming.replies {
            let bytes = peer_reply_bytes(&replies, self.limits.max_message_bytes)?;
            let (builder, secrets) = self
                .prepare(reqwest::Method::POST, &self.post_url, self.session.as_ref())
                .await?;
            remember_credentials(history, secrets, &self.limits)?;
            // A rotating provider can introduce a secret that was present in this envelope.
            check_reflection(&value, history)?;
            let response = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes)
                .send()
                .await
                .map_err(|_| Error::Transport)?;
            self.accept_session(&response, false, history)?;
            if response.status() != reqwest::StatusCode::ACCEPTED
                || !read_body(response, self.limits.max_message_bytes)
                    .await?
                    .is_empty()
            {
                return Err(Error::Protocol("peer reply not accepted".into()));
            }
        }
        check_reflection(&value, history)?;
        Ok(incoming.response)
    }

    async fn sse_reply(
        &mut self,
        reader: &mut SseReader,
        id: u64,
        history: &mut Vec<String>,
    ) -> Result<Value, Error> {
        let mut remaining = self.limits.max_items;
        while remaining > 0 {
            let (event, data) = reader.next().await?;
            if !event.is_empty() && event != "message" {
                return Err(Error::Protocol("unexpected SSE event".into()));
            }
            let value = serde_json::from_str(&data)
                .map_err(|_| Error::Protocol("invalid SSE JSON".into()))?;
            if let Some(response) = self
                .handle_incoming(value, id, &mut remaining, history)
                .await?
            {
                return response;
            }
        }
        Err(Error::Limit)
    }
}

fn content_type(response: &Response) -> &str {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(';').next())
        .unwrap_or("")
        .trim()
}

async fn read_body(mut response: Response, limit: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(Error::Limit);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn check_reflection(value: &Value, secrets: &[String]) -> Result<(), Error> {
    let contains = |text: &str| secrets.iter().any(|secret| text.contains(secret));
    match value {
        Value::String(text) if contains(text) => {
            return Err(Error::Policy("credential reflection rejected".into()));
        }
        Value::Number(_) | Value::Bool(_) | Value::Null if contains(&value.to_string()) => {
            return Err(Error::Policy("credential reflection rejected".into()));
        }
        Value::Array(items) => {
            for item in items {
                check_reflection(item, secrets)?;
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                if contains(key) {
                    return Err(Error::Policy("credential reflection rejected".into()));
                }
                check_reflection(item, secrets)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn remember_credentials(
    history: &mut Vec<String>,
    credentials: Vec<String>,
    limits: &Limits,
) -> Result<(), Error> {
    let mut additions = Vec::new();
    let mut bytes = history.iter().map(String::len).sum::<usize>();
    for credential in credentials {
        if credential.is_empty() || history.contains(&credential) || additions.contains(&credential)
        {
            continue;
        }
        bytes = bytes.checked_add(credential.len()).ok_or(Error::Limit)?;
        if bytes > limits.max_message_bytes
            || history.len().saturating_add(additions.len()) >= limits.max_items.min(64)
        {
            return Err(Error::Limit);
        }
        additions.push(credential);
    }
    history.extend(additions);
    Ok(())
}

impl Transport for HttpTransport {
    fn request<'a>(
        &'a mut self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(self.exchange(method, params, true))
    }
    fn notify<'a>(
        &'a mut self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move { self.exchange(method, params, false).await.map(|_| ()) })
    }
    fn close(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            if self.closed {
                return Ok(());
            }
            self.poisoned = true;
            self.closed = true;
            self.legacy.take();
            let mut history = std::mem::take(&mut self.credential_history);
            let session = self.session.take();
            self.post_url.set_query(None);
            if session.is_some() {
                let (builder, secrets) = timeout(
                    self.limits.timeout,
                    self.prepare(reqwest::Method::DELETE, &self.url, session.as_ref()),
                )
                .await
                .map_err(|_| Error::Transport)??;
                remember_credentials(&mut history, secrets, &self.limits)?;
                let result = timeout(self.limits.timeout, async {
                    let response = builder.send().await.map_err(|_| Error::Transport)?;
                    if !response.status().is_success()
                        && response.status() != reqwest::StatusCode::METHOD_NOT_ALLOWED
                    {
                        return Err(Error::Transport);
                    }
                    read_body(response, self.limits.max_message_bytes).await?;
                    Ok::<(), Error>(())
                })
                .await;
                if !matches!(result, Ok(Ok(()))) {
                    return Err(unknown(&self.server, "DELETE"));
                }
            }
            Ok(())
        })
    }
}
