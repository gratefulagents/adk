use super::{
    ServerConfig,
    results::{self, Diagnostic},
};
use crate::workspace::Workspace;
use adk_core::{BoxFuture, Cancellation, Context};
use adk_sandbox::{Executor, ProcessSession, SessionInput, SessionOutput};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{Notify, mpsc, oneshot},
    task::JoinHandle,
};

const MAX_HEADER: usize = 8 << 10;
#[derive(Default)]
struct Framer {
    bytes: Vec<u8>,
}
impl Framer {
    fn feed(
        &mut self,
        bytes: &[u8],
        limit: usize,
        queue: &mut VecDeque<Value>,
        max_messages: usize,
    ) -> Result<(), String> {
        self.bytes.extend_from_slice(bytes);
        loop {
            let end = self
                .bytes
                .windows(4)
                .position(|p| p == b"\r\n\r\n")
                .map(|n| n + 4)
                .or_else(|| {
                    self.bytes
                        .windows(2)
                        .position(|p| p == b"\n\n")
                        .map(|n| n + 2)
                });
            let Some(end) = end else {
                if self.bytes.len() > MAX_HEADER {
                    return Err("LSP header exceeds 8192 bytes".into());
                }
                return Ok(());
            };
            if end > MAX_HEADER {
                return Err("LSP header exceeds 8192 bytes".into());
            }
            let header = std::str::from_utf8(&self.bytes[..end]).map_err(|e| e.to_string())?;
            let mut length = None;
            for line in header.lines().filter(|line| !line.is_empty()) {
                let (name, value) = line.split_once(':').ok_or("invalid LSP header")?;
                if name.trim().eq_ignore_ascii_case("Content-Length") {
                    if length.is_some() {
                        return Err("duplicate LSP Content-Length".into());
                    }
                    length = Some(
                        value
                            .trim()
                            .parse::<usize>()
                            .map_err(|_| "invalid LSP Content-Length")?,
                    );
                }
            }
            let length = length.ok_or("LSP message has no Content-Length")?;
            if length > limit {
                return Err(format!("LSP message exceeds {limit} bytes"));
            }
            if self.bytes.len() - end < length {
                return Ok(());
            }
            let message: Value = serde_json::from_slice(&self.bytes[end..end + length])
                .map_err(|e| e.to_string())?;
            if !message.is_object() {
                return Err("invalid LSP message".into());
            }
            if queue.len() >= max_messages {
                return Err("LSP message queue limit exceeded".into());
            }
            queue.push_back(message);
            self.bytes.drain(..end + length);
        }
    }
}
struct Never;
impl Cancellation for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
struct Write {
    message: Value,
    done: oneshot::Sender<Result<(), String>>,
}
#[derive(Default)]
struct Published {
    path: String,
    value: Option<(Option<i64>, Vec<Diagnostic>)>,
}
#[derive(Default)]
struct Shared {
    published: Mutex<Published>,
    changed: Notify,
    error: Mutex<Option<String>>,
}
enum CompletionState {
    Running(JoinHandle<Result<(), String>>),
    Finished(Result<(), String>),
}
pub(super) struct DriverCompletion {
    state: tokio::sync::Mutex<CompletionState>,
}
impl DriverCompletion {
    fn new(task: JoinHandle<Result<(), String>>) -> Arc<Self> {
        Arc::new(Self {
            state: tokio::sync::Mutex::new(CompletionState::Running(task)),
        })
    }
    fn finished(&self) -> bool {
        self.state.try_lock().is_ok_and(|state| match &*state {
            CompletionState::Running(task) => task.is_finished(),
            CompletionState::Finished(_) => true,
        })
    }
    pub async fn wait(&self) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if let CompletionState::Running(task) = &mut *state {
            // Borrow the handle so cancelling a waiter cannot detach the driver.
            let result = task.await.map_err(|e| e.to_string()).and_then(|r| r);
            *state = CompletionState::Finished(result);
        }
        match &*state {
            CompletionState::Finished(result) => result.clone(),
            CompletionState::Running(_) => unreachable!(),
        }
    }
}
pub(super) struct Session {
    writes: mpsc::Sender<Write>,
    messages: mpsc::Receiver<Value>,
    shared: Arc<Shared>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<Arc<DriverCompletion>>,
    next_id: u64,
    pub pull_diagnostics: bool,
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
impl Session {
    pub fn start(
        executor: &Executor,
        context: &Context,
        config: &ServerConfig,
        workspace: &Workspace,
    ) -> Result<(Self, Arc<DriverCompletion>), String> {
        let mut request = adk_sandbox::Request::new(&config.command);
        request.args = config.args.clone();
        request.env = config.env.clone();
        request.cwd = workspace.root.clone();
        // A reused server must not inherit the first request's deadline/cancellation.
        let process_context = Context {
            run_id: context.run_id.clone(),
            cancellation: Arc::new(Never),
            deadline: None,
        };
        let mut process = executor
            .start_session(&process_context, request)
            .map_err(|e| e.to_string())?;
        let input = process.take_input().ok_or("LSP process has no input")?;
        let (writes, receive_writes) = mpsc::channel(1);
        let (send_messages, messages) = mpsc::channel(config.max_messages);
        let (stop, stopped) = oneshot::channel();
        let shared = Arc::new(Shared::default());
        let driver = Driver {
            process,
            input,
            framer: Framer::default(),
            pending: VecDeque::new(),
            stderr: Vec::new(),
            stderr_truncated: false,
            shared: shared.clone(),
            config: config.clone(),
            workspace: Workspace::new(&workspace.root).map_err(|e| e.to_string())?,
        };
        let task = DriverCompletion::new(tokio::spawn(driver.run(
            receive_writes,
            send_messages,
            stopped,
        )));
        Ok((
            Self {
                writes,
                messages,
                shared,
                stop: Some(stop),
                task: Some(task.clone()),
                next_id: 0,
                pull_diagnostics: false,
            },
            task,
        ))
    }
    pub fn finished(&self) -> bool {
        self.task.as_ref().is_none_or(|task| task.finished())
    }
    pub async fn close(mut self) -> Result<(), String> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = &self.task {
            task.wait().await?;
        }
        Ok(())
    }
    fn error(&self) -> String {
        self.shared
            .error
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "LSP session is closed".into())
    }
    async fn write(&self, message: Value) -> Result<(), String> {
        let (done, result) = oneshot::channel();
        self.writes
            .send(Write { message, done })
            .await
            .map_err(|_| self.error())?;
        result.await.map_err(|_| self.error())?
    }
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
    }
    pub async fn call(
        &mut self,
        method: &str,
        params: Value,
        max_messages: usize,
    ) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        for _ in 0..max_messages {
            let message = self.messages.recv().await.ok_or_else(|| self.error())?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error").filter(|v| !v.is_null()) {
                return Err(format!(
                    "LSP {method} failed ({}): {}",
                    error["code"], error["message"]
                ));
            }
            return message
                .get("result")
                .cloned()
                .ok_or("LSP response has no result".into());
        }
        Err(format!(
            "LSP server sent more than {max_messages} messages while handling {method}"
        ))
    }
    pub fn track_document(&self, path: &str) {
        *self.shared.published.lock().unwrap() = Published {
            path: path.into(),
            value: None,
        };
    }
    pub fn published(&self, version: i64) -> Option<Vec<Diagnostic>> {
        self.shared
            .published
            .lock()
            .unwrap()
            .value
            .as_ref()
            .filter(|(v, _)| v.is_none_or(|v| v >= version))
            .map(|(_, diagnostics)| diagnostics.clone())
    }
    pub async fn wait_diagnostics(&mut self, version: i64) -> Result<Vec<Diagnostic>, String> {
        loop {
            let notified = self.shared.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(value) = self.published(version) {
                return Ok(value);
            }
            tokio::select! {
                _ = &mut notified => {},
                message = self.messages.recv() => {
                    if message.is_none() { return Err(self.error()); }
                }
            }
        }
    }
}
struct Driver {
    process: ProcessSession,
    input: SessionInput,
    framer: Framer,
    pending: VecDeque<Value>,
    stderr: Vec<u8>,
    stderr_truncated: bool,
    shared: Arc<Shared>,
    config: ServerConfig,
    workspace: Workspace,
}
impl Driver {
    async fn run(
        mut self,
        mut writes: mpsc::Receiver<Write>,
        messages: mpsc::Sender<Value>,
        mut stop: oneshot::Receiver<()>,
    ) -> Result<(), String> {
        let result = tokio::select! {
            result = self.drive(&mut writes, &messages) => result,
            _ = &mut stop => Ok(()),
        };
        drop(self.input);
        let cleanup = match self.process.cancel_and_wait().await {
            Ok(output) => Ok(Some(output)),
            Err(adk_sandbox::Error::Cancelled | adk_sandbox::Error::TimedOut) => Ok(None),
            Err(error) => Err(error.to_string()),
        };
        let mut error = result.err();
        match &cleanup {
            Ok(Some(output)) => {
                let keep = output.stderr.len().min(
                    self.config
                        .max_stderr_bytes
                        .saturating_sub(self.stderr.len()),
                );
                self.stderr.extend_from_slice(&output.stderr[..keep]);
                self.stderr_truncated |= keep != output.stderr.len();
                if output.truncated {
                    let framing = "LSP process output truncated; framing is lost";
                    error = Some(
                        error
                            .map(|error| format!("{error}; {framing}"))
                            .unwrap_or_else(|| framing.into()),
                    );
                }
            }
            Ok(None) => {}
            Err(cleanup) => error = Some(cleanup.clone()),
        }
        if let Some(error) = error {
            let stderr = String::from_utf8_lossy(&self.stderr);
            *self.shared.error.lock().unwrap() = Some(format!(
                "{error}{}{}{}",
                if stderr.is_empty() { "" } else { ": " },
                stderr,
                if self.stderr_truncated {
                    " [stderr truncated]"
                } else {
                    ""
                }
            ));
        }
        drop(messages);
        cleanup.map(|_| ())
    }
    fn output(&mut self, output: SessionOutput) -> Result<(), String> {
        let keep = output.stderr.len().min(
            self.config
                .max_stderr_bytes
                .saturating_sub(self.stderr.len()),
        );
        self.stderr.extend_from_slice(&output.stderr[..keep]);
        self.stderr_truncated |= keep != output.stderr.len();
        if output.truncated {
            return Err("LSP process output truncated; framing is lost".into());
        }
        self.framer.feed(
            &output.stdout,
            self.config.max_message_bytes,
            &mut self.pending,
            self.config.max_messages,
        )?;
        if output.finished {
            return Err("LSP server exited".into());
        }
        Ok(())
    }
    async fn send(&mut self, message: Value) -> Result<(), String> {
        let body = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        if body.len() > self.config.max_message_bytes {
            return Err(format!(
                "outbound LSP message exceeds {} bytes",
                self.config.max_message_bytes
            ));
        }
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend(body);
        // Keep draining while stdin is backpressured, without cancelling a partial write.
        let input = &mut self.input;
        let write = input.write_all(&frame);
        tokio::pin!(write);
        loop {
            tokio::select! {
                result = &mut write => return result.map_err(|e| e.to_string()),
                output = self.process.next_output() => {
                    let keep = output.stderr.len().min(self.config.max_stderr_bytes.saturating_sub(self.stderr.len()));
                    self.stderr.extend_from_slice(&output.stderr[..keep]);
                    self.stderr_truncated |= keep != output.stderr.len();
                    if output.truncated { return Err("LSP process output truncated; framing is lost".into()); }
                    self.framer.feed(&output.stdout, self.config.max_message_bytes, &mut self.pending, self.config.max_messages)?;
                    if output.finished { return Err("LSP server exited".into()); }
                }
            }
        }
    }
    async fn drive(
        &mut self,
        writes: &mut mpsc::Receiver<Write>,
        messages: &mpsc::Sender<Value>,
    ) -> Result<(), String> {
        loop {
            if let Some(message) = self.pending.pop_front() {
                if let Some(method) = message.get("method").and_then(Value::as_str) {
                    if let Some(id) = message.get("id") {
                        let reply = match method {
                            "workspace/applyEdit" => {
                                json!({"jsonrpc":"2.0","id":id,"result":{"applied":false,"failureReason":"client is read-only"}})
                            }
                            "workspace/configuration" => {
                                json!({"jsonrpc":"2.0","id":id,"result":[]})
                            }
                            _ => {
                                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("client does not support server request {method}")}})
                            }
                        };
                        tokio::time::timeout(self.config.request_timeout, self.send(reply))
                            .await
                            .map_err(|_| "LSP server reply timed out")??;
                    } else if method == "textDocument/publishDiagnostics" {
                        let params = &message["params"];
                        if let Some(path) = params["uri"]
                            .as_str()
                            .and_then(|uri| results::confined_uri(&self.workspace, uri))
                        {
                            let mut published = self.shared.published.lock().unwrap();
                            if published.path == path {
                                let version = params
                                    .get("version")
                                    .filter(|value| !value.is_null())
                                    .map(|value| {
                                        value.as_i64().ok_or("invalid published diagnostic version")
                                    })
                                    .transpose()?;
                                let diagnostics = params
                                    .get("diagnostics")
                                    .ok_or("published diagnostics missing diagnostics")?;
                                published.value =
                                    Some((version, results::diagnostics(diagnostics, &path)?));
                                self.shared.changed.notify_waiters();
                            }
                        }
                    }
                } else {
                    messages
                        .try_send(message)
                        .map_err(|_| "LSP response queue limit exceeded")?;
                }
                continue;
            }
            tokio::select! {
                command = writes.recv() => {
                    let Some(command) = command else { return Ok(()); };
                    let result = self.send(command.message).await;
                    let _ = command.done.send(result.clone());
                    result?;
                }
                output = self.process.next_output() => self.output(output)?,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn session_drop_signals_stop_and_close_awaits_cleanup() {
        for explicit in [false, true] {
            let (writes, _commands) = mpsc::channel(1);
            let (_responses, messages) = mpsc::channel(1);
            let (stop, stopped) = oneshot::channel();
            let (cleaned, cleanup) = oneshot::channel();
            let task = tokio::spawn(async move {
                stopped.await.unwrap();
                tokio::task::yield_now().await;
                cleaned.send(()).unwrap();
                Err("cleanup evidence".into())
            });
            let session = Session {
                writes,
                messages,
                shared: Arc::new(Shared::default()),
                stop: Some(stop),
                task: Some(DriverCompletion::new(task)),
                next_id: 0,
                pull_diagnostics: false,
            };
            if explicit {
                assert_eq!(session.close().await.unwrap_err(), "cleanup evidence");
            } else {
                drop(session);
            }
            tokio::time::timeout(std::time::Duration::from_secs(1), cleanup)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn owner_close_retains_abandoned_session_and_cleanup_waits() {
        let root = tempfile::tempdir().unwrap();
        let tool = super::super::LspTool::new(super::super::Config {
            executor: Arc::new(Executor::new(adk_sandbox::Config::new(root.path())).unwrap()),
            servers: Vec::new(),
            discoverer: None,
        });
        let mut releases = Vec::new();
        for mode in ["dropped", "closing", "cached"] {
            let (writes, _commands) = mpsc::channel(1);
            let (_responses, messages) = mpsc::channel(1);
            let (stop, stopped) = oneshot::channel();
            let (release, released) = oneshot::channel();
            releases.push(release);
            let driver = DriverCompletion::new(tokio::spawn(async move {
                stopped.await.unwrap();
                released.await.unwrap();
                Err(format!("{mode} cleanup"))
            }));
            let session = Session {
                writes,
                messages,
                shared: Arc::new(Shared::default()),
                stop: Some(stop),
                task: Some(driver.clone()),
                next_id: 0,
                pull_diagnostics: false,
            };
            tool.sessions.lock().await.drivers.push(driver);
            match mode {
                "dropped" => drop(session),
                "closing" => {
                    let mut close = Box::pin(session.close());
                    std::future::poll_fn(|cx| {
                        assert!(close.as_mut().poll(cx).is_pending());
                        std::task::Poll::Ready(())
                    })
                    .await;
                    drop(close);
                }
                _ => {
                    tool.sessions
                        .lock()
                        .await
                        .cached
                        .insert((root.path().into(), mode.into()), session);
                }
            }
        }
        let mut close = Box::pin(tool.close());
        std::future::poll_fn(|cx| {
            assert!(close.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(close);
        for release in releases {
            release.send(()).unwrap();
        }
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), tool.close())
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error, "dropped cleanup; closing cleanup; cached cleanup");
        tool.close().await.unwrap();
    }

    #[tokio::test]
    async fn transport_channel_eof_and_error_responses_fail_closed() {
        for response in [
            None,
            Some(json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"missing"}})),
            Some(json!({"jsonrpc":"2.0","id":1})),
        ] {
            let (writes, mut commands) = mpsc::channel::<Write>(1);
            let (responses, messages) = mpsc::channel(1);
            let server = tokio::spawn(async move {
                let command = commands.recv().await.unwrap();
                assert_eq!(command.message["method"], "textDocument/hover");
                command.done.send(Ok(())).unwrap();
                if let Some(response) = response {
                    responses.send(response).await.unwrap();
                }
            });
            let mut session = Session {
                writes,
                messages,
                shared: Arc::new(Shared::default()),
                stop: None,
                task: None,
                next_id: 0,
                pull_diagnostics: false,
            };
            let error = session
                .call("textDocument/hover", json!({}), 1)
                .await
                .unwrap_err();
            assert!(
                error.contains("closed") || error.contains("-32601") || error.contains("no result"),
                "{error}"
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn initialize_operations_and_document_notifications() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("sample.txt"), "a😀b\r\n").unwrap();
        let workspace = Workspace::new(root.path()).unwrap();
        let config = ServerConfig {
            language_id: "plain".into(),
            ..Default::default()
        };
        let (writes, mut commands) = mpsc::channel::<Write>(1);
        let (responses, messages) = mpsc::channel(16);
        let mut session = Session {
            writes,
            messages,
            shared: Arc::new(Shared::default()),
            stop: None,
            task: None,
            next_id: 0,
            pull_diagnostics: false,
        };
        let uri = results::file_uri(&root.path().join("sample.txt")).unwrap();
        let server = tokio::spawn(async move {
            let mut opened = false;
            let mut initialized = false;
            let mut operations = Vec::new();
            while let Some(command) = commands.recv().await {
                let message = command.message;
                let method = message["method"].as_str().unwrap();
                let params = &message["params"];
                let range =
                    json!({"start":{"line":0,"character":0},"end":{"line":0,"character":3}});
                let result = match method {
                    "initialize" => {
                        assert_eq!(
                            params["capabilities"]["general"]["positionEncodings"],
                            json!(["utf-16"])
                        );
                        Some(
                            json!({"capabilities":{"positionEncoding":"utf-16","diagnosticProvider":true}}),
                        )
                    }
                    "initialized" => {
                        initialized = true;
                        None
                    }
                    "textDocument/didOpen" => {
                        assert!(initialized && !opened);
                        assert_eq!(params["textDocument"]["text"], "a😀b\r\n");
                        assert_eq!(params["textDocument"]["version"], 1);
                        assert_eq!(params["textDocument"]["languageId"], "plain");
                        opened = true;
                        None
                    }
                    "textDocument/didClose" => {
                        assert!(opened);
                        opened = false;
                        None
                    }
                    "workspace/symbol" => {
                        assert!(!opened);
                        operations.push(method.to_owned());
                        Some(json!([]))
                    }
                    "textDocument/documentSymbol" => {
                        assert!(opened);
                        operations.push(method.to_owned());
                        Some(json!([]))
                    }
                    "textDocument/diagnostic" => {
                        assert!(opened);
                        operations.push(method.to_owned());
                        Some(json!({"kind":"full","items":[]}))
                    }
                    "textDocument/hover" => {
                        assert!(opened);
                        assert_eq!(params["position"], json!({"line":0,"character":3}));
                        operations.push(method.to_owned());
                        Some(json!({"contents":"hover"}))
                    }
                    "textDocument/definition"
                    | "textDocument/references"
                    | "textDocument/implementation"
                    | "textDocument/typeDefinition" => {
                        assert!(opened);
                        assert_eq!(params["position"], json!({"line":0,"character":3}));
                        if method == "textDocument/references" {
                            assert_eq!(params["context"]["includeDeclaration"], true);
                        }
                        operations.push(method.to_owned());
                        Some(json!({"uri":uri,"range":range}))
                    }
                    _ => panic!("unexpected or mutating method {method}"),
                };
                command.done.send(Ok(())).unwrap();
                if let Some(result) = result {
                    responses
                        .send(json!({"jsonrpc":"2.0","id":message["id"],"result":result}))
                        .await
                        .unwrap();
                }
            }
            assert!(!opened);
            assert_eq!(operations.len(), 10);
        });
        super::super::initialize(&mut session, &workspace, &config)
            .await
            .unwrap();
        assert!(session.pull_diagnostics);
        for operation in [
            "goToDefinition",
            "definition",
            "findReferences",
            "references",
            "hover",
            "documentSymbol",
            "workspaceSymbol",
            "implementation",
            "typeDefinition",
            "diagnostics",
        ] {
            let request = super::super::Request {
                operation: operation.into(),
                file_path: "sample.txt".into(),
                line: 1,
                character: 4,
                ..Default::default()
            };
            let document = (operation != "workspaceSymbol")
                .then(|| super::super::Document::open(&workspace, &request).unwrap());
            let normalized = super::super::normalize_operation(operation).unwrap();
            let result = super::super::operate(
                &mut session,
                &workspace,
                &config,
                &request,
                normalized,
                document.as_ref(),
            )
            .await
            .unwrap();
            assert_eq!(result.operation, normalized);
            if normalized == "hover" {
                assert_eq!(result.hover.unwrap().contents, "hover");
            }
            if !result.locations.is_empty() {
                assert_eq!(result.locations[0].range.start.line, 1);
            }
        }
        drop(session);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn published_diagnostics_require_current_version() {
        let (writes, _commands) = mpsc::channel(1);
        let (_responses, messages) = mpsc::channel(1);
        let shared = Arc::new(Shared::default());
        let mut session = Session {
            writes,
            messages,
            shared: shared.clone(),
            stop: None,
            task: None,
            next_id: 0,
            pull_diagnostics: false,
        };
        session.track_document("file");
        shared.published.lock().unwrap().value = Some((Some(0), Vec::new()));
        assert!(session.published(1).is_none());
        let publish = async {
            tokio::task::yield_now().await;
            shared.published.lock().unwrap().value = Some((Some(1), Vec::new()));
            shared.changed.notify_waiters();
        };
        let (diagnostics, _) = tokio::join!(session.wait_diagnostics(1), publish);
        assert!(diagnostics.unwrap().is_empty());
        session.track_document("");
        assert!(session.published(1).is_none());
    }
    #[test]
    fn fragmented_bounded_framing() {
        let mut parser = Framer::default();
        let mut queue = VecDeque::new();
        for byte in b"Content-Length: 2\r\n\r\n{}Content-Length: 8\n\n{\"id\":1}" {
            parser.feed(&[*byte], 8, &mut queue, 2).unwrap();
        }
        assert_eq!(queue, [json!({}), json!({"id":1})]);
        assert!(parser.bytes.is_empty());
        assert_eq!(
            parser
                .feed(b"Content-Length: 4\n\nnull", 8, &mut queue, 3)
                .unwrap_err(),
            "invalid LSP message"
        );
    }
    #[test]
    fn framing_rejects_oversize_missing_duplicate_and_flood() {
        for bytes in [
            b"Content-Length: 99\r\n\r\n".as_slice(),
            b"Other: 2\n\n",
            b"Content-Length: 2\nContent-Length: 2\n\n{}",
            b"Content-Length: -1\n\n",
        ] {
            assert!(
                Framer::default()
                    .feed(bytes, 8, &mut VecDeque::new(), 2)
                    .is_err()
            );
        }
        assert!(
            Framer::default()
                .feed(&vec![b'a'; MAX_HEADER + 1], 8, &mut VecDeque::new(), 2)
                .is_err()
        );
        assert!(
            Framer::default()
                .feed(
                    b"Content-Length: 2\n\n{}Content-Length: 2\n\n{}",
                    8,
                    &mut VecDeque::new(),
                    1
                )
                .is_err()
        );
    }
}
