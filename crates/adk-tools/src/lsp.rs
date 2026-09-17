//! Host-configured, read-only stdio LSP over confined sandbox process sessions.
use crate::{search_pattern::Pattern, workspace::Workspace};
use adk_core::{
    BoxFuture, Content, Context, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use adk_sandbox::Executor;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify};
#[path = "lsp_protocol.rs"]
mod protocol;
#[path = "lsp_results.rs"]
mod results;
use protocol::{DriverCompletion, Session};
pub use results::{Diagnostic, Hover, Location, Position, Range, ResultData, Symbol};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub id: String,
    /// Absolute, host-selected executable; never resolved from model arguments.
    pub command: PathBuf,
    pub args: Vec<String>,
    /// Subject to the executor's explicit environment allowlist.
    pub env: BTreeMap<String, String>,
    pub language_id: String,
    pub file_patterns: Vec<String>,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
    pub max_message_bytes: usize,
    pub max_output_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_messages: usize,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            command: PathBuf::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            language_id: String::new(),
            file_patterns: Vec::new(),
            startup_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(15),
            max_message_bytes: 1 << 20,
            max_output_bytes: 1 << 20,
            max_stderr_bytes: 64 << 10,
            max_messages: 16,
        }
    }
}
impl ServerConfig {
    fn normalized(mut self) -> Result<Self, String> {
        if !self.command.is_absolute() {
            return Err("configured language server command must be an absolute path".into());
        }
        let defaults = Self::default();
        if self.startup_timeout.is_zero() {
            self.startup_timeout = defaults.startup_timeout;
        }
        if self.request_timeout.is_zero() {
            self.request_timeout = defaults.request_timeout;
        }
        if self.max_message_bytes == 0 {
            self.max_message_bytes = defaults.max_message_bytes;
        }
        if self.max_output_bytes == 0 {
            self.max_output_bytes = defaults.max_output_bytes;
        }
        if self.max_stderr_bytes == 0 {
            self.max_stderr_bytes = defaults.max_stderr_bytes;
        }
        if self.max_messages == 0 {
            self.max_messages = defaults.max_messages;
        }
        Ok(self)
    }
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Request {
    pub operation: String,
    pub file_path: String,
    pub language_id: String,
    pub line: i64,
    pub character: i64,
    pub query: String,
}
pub trait ServerDiscoverer: Send + Sync {
    fn discover<'a>(
        &'a self,
        context: &'a Context,
        workspace: &'a Path,
        request: &'a Request,
    ) -> BoxFuture<'a, Result<Vec<ServerConfig>, String>>;
}
/// All launch configuration and discovery are trusted host inputs. No unconfined fallback.
pub struct Config {
    pub executor: Arc<Executor>,
    pub servers: Vec<ServerConfig>,
    pub discoverer: Option<Arc<dyn ServerDiscoverer>>,
}
#[derive(Default)]
struct Sessions {
    cached: BTreeMap<(PathBuf, String), Session>,
    // Active sessions leave the cache; their cleanup must remain owner-joinable.
    drivers: Vec<Arc<DriverCompletion>>,
}
pub struct LspTool {
    config: Config,
    definition: ToolDefinition,
    sessions: Mutex<Sessions>,
    closed: AtomicBool,
    close_notify: Notify,
}
pub fn tool(config: Config) -> Arc<LspTool> {
    Arc::new(LspTool::new(config))
}
impl LspTool {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            definition: crate::capabilities()
                .iter()
                .find(|c| c.name == "LSP")
                .and_then(|c| c.definition.clone())
                .expect("pinned LSP definition"),
            sessions: Mutex::new(Sessions::default()),
            closed: AtomicBool::new(false),
            close_notify: Notify::new(),
        }
    }
    /// Interrupt active operations and await every owned server's process-group cleanup.
    pub async fn close(&self) -> Result<(), String> {
        self.closed.store(true, Ordering::Release);
        self.close_notify.notify_waiters();
        let mut sessions = self.sessions.lock().await;
        let mut errors = Vec::new();
        sessions.cached.clear();
        for driver in &sessions.drivers {
            if let Err(error) = driver.wait().await {
                errors.push(error);
            }
        }
        sessions.drivers.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
    async fn bounded<T>(
        &self,
        context: &Context,
        timeout: Duration,
        future: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let closed = self.close_notify.notified();
        tokio::pin!(closed);
        closed.as_mut().enable();
        context.check_active().map_err(|e| e.to_string())?;
        if self.closed.load(Ordering::Acquire) {
            return Err("LSP tool is closed".into());
        }
        let timeout = context
            .deadline
            .map(|d| {
                d.saturating_duration_since(std::time::Instant::now())
                    .min(timeout)
            })
            .unwrap_or(timeout);
        tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => Err("LSP operation cancelled".into()),
            _ = &mut closed => Err("LSP tool is closed".into()),
            result = tokio::time::timeout(timeout, future) => result.map_err(|_| "LSP operation timed out".to_owned())?,
        }
    }
    async fn execute_request(
        &self,
        context: &ToolContext,
        request: Request,
    ) -> Result<String, String> {
        let operation = normalize_operation(&request.operation)?;
        let workspace = Workspace::new(&context.work_dir).map_err(|e| e.to_string())?;
        let mut candidates = self.config.servers.clone();
        if let Some(discoverer) = &self.config.discoverer {
            candidates.extend(
                self.bounded(
                    &context.operation,
                    Duration::from_secs(15),
                    discoverer.discover(&context.operation, &workspace.root, &request),
                )
                .await?,
            );
        }
        let mut matches = Vec::new();
        for config in candidates {
            let config = config.normalized()?;
            if matches_config(&config, &workspace, &request)? {
                matches.push(config);
            }
        }
        if matches.is_empty() {
            return Err("no configured language server matches the request".into());
        }
        if matches.len() != 1 {
            return Err("language-server selection is ambiguous".into());
        }
        let config = matches.pop().unwrap();
        let document = if operation != "workspaceSymbol" {
            Some(Document::open(&workspace, &request)?)
        } else {
            None
        };
        if matches!(
            operation,
            "definition" | "references" | "hover" | "implementation" | "typeDefinition"
        ) {
            document
                .as_ref()
                .unwrap()
                .position(request.line, request.character)?;
        }
        let key = (workspace.root.clone(), format!("{config:?}"));
        let mut sessions = self
            .bounded(&context.operation, config.request_timeout, async {
                Ok(self.sessions.lock().await)
            })
            .await?;
        let mut session = sessions.cached.remove(&key);
        if session.as_ref().is_some_and(Session::finished) {
            session.take().unwrap().close().await?;
        }
        let fresh = session.is_none();
        let mut session = match session {
            Some(session) => session,
            None => {
                let (session, driver) = Session::start(
                    &self.config.executor,
                    &context.operation,
                    &config,
                    &workspace,
                )?;
                sessions.drivers.push(driver);
                session
            }
        };
        if fresh {
            let initialized = self
                .bounded(
                    &context.operation,
                    config.startup_timeout,
                    initialize(&mut session, &workspace, &config),
                )
                .await;
            if let Err(error) = initialized {
                let cleanup = session.close().await;
                return Err(with_cleanup(error, cleanup));
            }
        }
        let result = self
            .bounded(
                &context.operation,
                config.request_timeout,
                operate(
                    &mut session,
                    &workspace,
                    &config,
                    &request,
                    operation,
                    document.as_ref(),
                ),
            )
            .await;
        match result {
            Ok(result) => {
                sessions.cached.insert(key, session);
                let content = crate::json_text(&result).map_err(|e| e.to_string())?;
                if content.len() > config.max_output_bytes {
                    return Err(format!(
                        "LSP result exceeds configured output limit of {} bytes",
                        config.max_output_bytes
                    ));
                }
                Ok(content)
            }
            Err(error) => {
                let cleanup = session.close().await;
                Err(with_cleanup(error, cleanup))
            }
        }
    }
}
fn with_cleanup(error: String, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; cleanup: {cleanup}"),
    }
}
impl Tool for LspTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let result = match serde_json::from_value::<Request>(call.arguments) {
                Ok(request) => self
                    .execute_request(context, request)
                    .await
                    .map_err(|e| format!("LSP error: {e}")),
                Err(error) => Err(format!("Invalid input: {error}")),
            };
            context.operation.check_active()?;
            let is_error = result.is_err();
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: result.unwrap_or_else(|error| error),
                }],
                is_error,
                should_pause: false,
            })
        })
    }
}
fn normalize_operation(operation: &str) -> Result<&str, String> {
    match operation {
        "goToDefinition" | "definition" => Ok("definition"),
        "findReferences" | "references" => Ok("references"),
        "hover" | "documentSymbol" | "workspaceSymbol" | "implementation" | "typeDefinition"
        | "diagnostics" => Ok(operation),
        _ => Err(format!(
            "unknown or non-read-only LSP operation: {operation}"
        )),
    }
}
fn matches_config(
    config: &ServerConfig,
    workspace: &Workspace,
    request: &Request,
) -> Result<bool, String> {
    if !config.language_id.is_empty()
        && !request.language_id.is_empty()
        && config.language_id != request.language_id
    {
        return Ok(false);
    }
    if config.file_patterns.is_empty() || request.file_path.is_empty() {
        return Ok(true);
    }
    let relative = workspace
        .relative(&request.file_path)
        .map_err(|e| e.to_string())?;
    let relative = relative.to_string_lossy();
    Ok(config.file_patterns.iter().any(|pattern| {
        let mut candidate = pattern.as_str();
        loop {
            // Go filepath.Match treats ** as *, except for the SDK's leading **/ fallback.
            let glob = candidate
                .split('/')
                .map(|part| if part == "**" { "*" } else { part })
                .collect::<Vec<_>>()
                .join("/");
            if Pattern::new(&glob).is_ok_and(|pattern| {
                pattern.matches(&relative)
                    || pattern.matches(relative.rsplit('/').next().unwrap_or(&relative))
            }) {
                return true;
            }
            let Some(rest) = candidate.strip_prefix("**/") else {
                return false;
            };
            candidate = rest;
        }
    }))
}
struct Document {
    path: String,
    uri: String,
    text: String,
}
impl Document {
    fn open(workspace: &Workspace, request: &Request) -> Result<Self, String> {
        if request.file_path.is_empty() {
            return Err(format!("filePath is required for {}", request.operation));
        }
        let relative = workspace
            .relative(&request.file_path)
            .map_err(|e| e.to_string())?;
        let file = workspace
            .read_file(&relative)
            .map_err(|e| format!("filePath rejected: {e}"))?;
        let mut bytes = Vec::new();
        file.take((16 << 20) + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 16 << 20 {
            return Err("filePath exceeds 16777216-byte LSP source limit".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| "filePath is not valid UTF-8")?;
        let path = workspace.root.join(relative);
        Ok(Self {
            uri: results::file_uri(&path)?,
            path: path.to_string_lossy().into_owned(),
            text,
        })
    }
    fn position(&self, line: i64, character: i64) -> Result<Value, String> {
        if line < 1 || character < 1 {
            return Err("line and character must be 1-based positive integers".into());
        }
        let selected = self
            .text
            .split('\n')
            .nth((line - 1) as usize)
            .ok_or("line is outside the file")?;
        let selected = selected.strip_suffix('\r').unwrap_or(selected);
        if character as u64 > selected.encode_utf16().count() as u64 + 1 {
            return Err("character is outside the line".into());
        }
        Ok(json!({"line":line-1,"character":character-1}))
    }
}
async fn initialize(
    session: &mut Session,
    workspace: &Workspace,
    config: &ServerConfig,
) -> Result<(), String> {
    let uri = results::file_uri(&workspace.root)?;
    let response = session.call("initialize", json!({
        "processId":null,"rootUri":uri,"workspaceFolders":[{"uri":uri,"name":workspace.root.file_name().unwrap_or_default().to_string_lossy()}],
        "capabilities":{"general":{"positionEncodings":["utf-16"]},"textDocument":{"documentSymbol":{"hierarchicalDocumentSymbolSupport":true}}}
    }), config.max_messages).await?;
    if !response.is_object() {
        return Err("invalid LSP initialize result".into());
    }
    if !response["capabilities"].is_null() && !response["capabilities"].is_object() {
        return Err("invalid LSP initialize capabilities".into());
    }
    if let Some(encoding) = response["capabilities"]
        .get("positionEncoding")
        .filter(|v| !v.is_null())
    {
        let encoding = encoding.as_str().ok_or("invalid LSP position encoding")?;
        if !encoding.is_empty() && !encoding.eq_ignore_ascii_case("utf-16") {
            return Err(format!(
                "LSP server selected unsupported position encoding {encoding}"
            ));
        }
    }
    session.pull_diagnostics = response["capabilities"]
        .get("diagnosticProvider")
        .is_some_and(|v| !v.is_null() && v != &Value::Bool(false));
    session.notify("initialized", json!({})).await
}
async fn operate(
    session: &mut Session,
    workspace: &Workspace,
    config: &ServerConfig,
    request: &Request,
    operation: &str,
    document: Option<&Document>,
) -> Result<ResultData, String> {
    if let Some(document) = document {
        session.track_document(&document.path);
        session.notify("textDocument/didOpen", json!({"textDocument":{"uri":document.uri,"languageId":if config.language_id.is_empty() { &request.language_id } else { &config.language_id },"version":1,"text":document.text}})).await?;
    }
    let result = async {
        if operation == "diagnostics" && !session.pull_diagnostics {
            return Ok(ResultData {
                operation: operation.into(),
                diagnostics: session.wait_diagnostics(1).await?,
                ..Default::default()
            });
        }
        let (method, mut params) = match operation {
            "workspaceSymbol" => (
                "workspace/symbol".to_owned(),
                json!({"query":request.query}),
            ),
            _ => (
                format!(
                    "textDocument/{}",
                    if operation == "diagnostics" {
                        "diagnostic"
                    } else {
                        operation
                    }
                ),
                json!({"textDocument":{"uri":document.unwrap().uri}}),
            ),
        };
        if matches!(
            operation,
            "definition" | "references" | "hover" | "implementation" | "typeDefinition"
        ) {
            params["position"] = document
                .unwrap()
                .position(request.line, request.character)?;
        }
        if operation == "references" {
            params["context"] = json!({"includeDeclaration":true});
        }
        let raw = match session.call(&method, params, config.max_messages).await {
            Ok(raw) => raw,
            Err(error) => {
                if operation == "diagnostics"
                    && let Some(diagnostics) = session.published(1)
                {
                    return Ok(ResultData {
                        operation: operation.into(),
                        diagnostics,
                        ..Default::default()
                    });
                }
                return Err(error);
            }
        };
        results::parse(
            operation,
            &raw,
            workspace,
            document.map(|d| d.path.as_str()).unwrap_or(""),
        )
    }
    .await;
    if let Some(document) = document {
        session
            .notify(
                "textDocument/didClose",
                json!({"textDocument":{"uri":document.uri}}),
            )
            .await?;
        session.track_document("");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_go_differential_corpus() {
        let corpus: Value =
            serde_json::from_str(include_str!("../../../fixtures/tools/lsp-cases.json")).unwrap();
        let expected: Value =
            serde_json::from_str(include_str!("../../../fixtures/tools/lsp-expected.json"))
                .unwrap();
        assert_eq!(corpus["sdk_commit"], expected["sdk_commit"]);
        let root = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(root.path()).unwrap();
        let root = workspace.root.to_str().unwrap();
        let cases = corpus["cases"].as_array().unwrap();
        let outputs = expected["cases"].as_array().unwrap();
        assert_eq!(cases.len(), outputs.len());
        for (case, output) in cases.iter().zip(outputs) {
            assert_eq!(case["name"], output["name"]);
            let operation = normalize_operation(case["operation"].as_str().unwrap()).unwrap();
            let raw =
                serde_json::from_str(&case["raw"].to_string().replace("@ROOT@", root)).unwrap();
            let result = results::parse(operation, &raw, &workspace, &format!("{root}/sample.txt"));
            assert_eq!(
                result.is_err(),
                output["is_error"].as_bool().unwrap(),
                "{}: {result:?}",
                case["name"]
            );
            if let Ok(result) = result {
                let stable: Value = serde_json::from_str(
                    &serde_json::to_string(&result)
                        .unwrap()
                        .replace(root, "@ROOT@"),
                )
                .unwrap();
                assert_eq!(stable, output["result"], "{}", case["name"]);
            } else {
                assert!(output["error"].is_string());
            }
        }
        let inputs = corpus["operations"].as_array().unwrap();
        let outputs = expected["operations"].as_array().unwrap();
        assert_eq!(inputs.len(), outputs.len());
        for (input, output) in inputs.iter().zip(outputs) {
            assert_eq!(input, &output["input"]);
            match normalize_operation(input.as_str().unwrap()) {
                Ok(operation) => {
                    assert_eq!(output["is_error"], false);
                    assert_eq!(operation, output["normalized"]);
                }
                Err(error) => {
                    assert_eq!(output["is_error"], true);
                    assert_eq!(error, output["error"]);
                }
            }
        }
        let definition = crate::capabilities()
            .iter()
            .find(|c| c.name == "LSP")
            .unwrap()
            .definition
            .as_ref()
            .unwrap()
            .clone();
        assert_eq!(
            serde_json::to_value(definition).unwrap(),
            expected["definition"]
        );
    }

    #[tokio::test]
    async fn bounded_interrupts_pending_work_without_spawning() {
        for mode in ["cancel", "deadline", "close", "timeout"] {
            let root = tempfile::tempdir().unwrap();
            let tool = tool(Config {
                executor: Arc::new(Executor::new(adk_sandbox::Config::new(root.path())).unwrap()),
                servers: Vec::new(),
                discoverer: None,
            });
            let token = Arc::new(adk_runtime::CancellationToken::new());
            let context = Context {
                run_id: "bounded-test".into(),
                cancellation: token.clone(),
                deadline: (mode == "deadline")
                    .then(|| std::time::Instant::now() + Duration::from_millis(20)),
            };
            let timeout = if mode == "timeout" {
                Duration::from_millis(20)
            } else {
                Duration::from_secs(2)
            };
            let pending = tool.bounded::<()>(&context, timeout, std::future::pending());
            let stop = async {
                tokio::task::yield_now().await;
                if mode == "cancel" {
                    token.cancel();
                }
                if mode == "close" {
                    tool.close().await.unwrap();
                }
            };
            let (result, _) = tokio::time::timeout(Duration::from_secs(1), async {
                tokio::join!(pending, stop)
            })
            .await
            .unwrap();
            assert_eq!(
                result.unwrap_err(),
                match mode {
                    "cancel" => "LSP operation cancelled",
                    "close" => "LSP tool is closed",
                    _ => "LSP operation timed out",
                }
            );
            tool.close().await.unwrap();
            assert_eq!(
                tool.bounded(&context, timeout, async { Ok(()) })
                    .await
                    .unwrap_err(),
                if mode == "cancel" {
                    "operation cancelled"
                } else if mode == "deadline" {
                    "deadline exceeded"
                } else {
                    "LSP tool is closed"
                }
            );
        }
    }

    #[test]
    fn server_patterns_match_go_filepath_semantics() {
        let root = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(root.path()).unwrap();
        for (pattern, path, expected) in [
            ("*.txt", "nested/file.txt", true),
            ("**/*.txt", "file.txt", true),
            ("**/src/*.txt", "src/file.txt", true),
            ("a/**/b.txt", "a/x/b.txt", true),
            ("a/**/b.txt", "a/x/y/b.txt", false),
            ("a/**/b.txt", "a/b.txt", false),
            ("[", "file.txt", false),
        ] {
            let config = ServerConfig {
                file_patterns: vec![pattern.into()],
                ..Default::default()
            };
            let request = Request {
                file_path: path.into(),
                ..Default::default()
            };
            assert_eq!(
                matches_config(&config, &workspace, &request).unwrap(),
                expected,
                "{pattern}: {path}"
            );
        }
    }
    #[test]
    fn positions_and_read_only_aliases() {
        let doc = Document {
            path: String::new(),
            uri: String::new(),
            text: "a😀b\r\n".into(),
        };
        assert_eq!(doc.position(1, 4).unwrap(), json!({"line":0,"character":3}));
        assert_eq!(doc.position(1, 5).unwrap(), json!({"line":0,"character":4}));
        assert!(doc.position(1, 6).is_err());
        assert!(doc.position(0, 1).is_err());
        assert!(doc.position(3, 1).is_err());
        assert_eq!(normalize_operation("goToDefinition").unwrap(), "definition");
        assert_eq!(normalize_operation("findReferences").unwrap(), "references");
        for operation in ["rename", "formatting", "codeAction", "executeCommand"] {
            assert!(normalize_operation(operation).is_err());
        }
    }
}
