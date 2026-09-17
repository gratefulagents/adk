//! Chromium command-line browser adapter. The host owns executable discovery and
//! a confined runner; public-only browser networking is deliberately unsupported.
#[path = "browser_process.rs"]
mod process;
pub use process::ManagedRunner;

use crate::{capabilities, html, network, search::go_string, workspace::Workspace, write};
use adk_core::{
    AccessMode, BoxFuture, Content, Error, ErrorCategory, Tool, ToolCall, ToolContext,
    ToolDefinition, ToolOutput,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Trusted process request, never deserialized from model input. Implementations
/// must enforce the access mode and writable grants and reap on cancellation.
pub struct Request {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub work_dir: PathBuf,
    pub access: AccessMode,
    pub writable_paths: Vec<PathBuf>,
    pub scratch: Option<Arc<adk_sandbox::ScratchDirectory>>,
    pub timeout: Duration,
}
pub struct Execution {
    pub output: Vec<u8>,
    pub exit_code: i32,
    pub timed_out: bool,
}
pub trait Runner: Send + Sync {
    fn run<'a>(
        &'a self,
        context: &'a ToolContext,
        request: Request,
    ) -> BoxFuture<'a, Result<Execution, String>>;
}
pub struct Config {
    pub runner: Arc<dyn Runner>,
    /// Absolute path resolved by the trusted host, or None if Chrome is absent.
    pub executable: Option<PathBuf>,
    pub screenshot_dir: PathBuf,
    pub access: AccessMode,
    pub allow_private_network_urls: bool,
}
pub fn tool(config: Config) -> Arc<dyn Tool> {
    let mode = if config.access == AccessMode::ReadOnly {
        "read_only"
    } else {
        "write"
    };
    let definition = capabilities()
        .iter()
        .find(|c| c.name == "Browser" && c.mode == mode)
        .expect("browser catalog")
        .definition
        .clone()
        .unwrap();
    Arc::new(Browser { config, definition })
}
struct Browser {
    config: Config,
    definition: ToolDefinition,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    action: String,
    url: String,
    output_path: String,
    width: i64,
    height: i64,
}
fn viewport(width: i64, height: i64) -> Result<(i64, i64), String> {
    let width = if width == 0 { 1280 } else { width };
    let height = if height == 0 { 720 } else { height };
    if !(100..=4096).contains(&width) {
        return Err("width must be between 100 and 4096 pixels".into());
    }
    if !(100..=4096).contains(&height) {
        return Err("height must be between 100 and 4096 pixels".into());
    }
    Ok((width, height))
}
fn title(html: &str) -> String {
    // The SDK searches UTF-8 byte offsets in the lowercase copy, not a DOM.
    let lower = crate::skills::go_lower(html);
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(end) = lower[start..].find('>') else {
        return String::new();
    };
    let start = start + end + 1;
    let Some(end) = lower[start..].find("</title>") else {
        return String::new();
    };
    go_string(html.as_bytes().get(start..start + end).unwrap_or_default())
        .trim()
        .to_owned()
}
impl Browser {
    async fn invoke(&self, context: &ToolContext, mut args: Value) -> Result<String, String> {
        if args.is_null() {
            args = json!({});
        }
        if let Some(fields) = args.as_object_mut() {
            fields.retain(|_, v| !v.is_null());
        }
        let input: Input =
            serde_json::from_value(args).map_err(|e| format!("Invalid input: {e}"))?;
        if input.url.is_empty() {
            return Err("url is required".into());
        }
        if !self.config.allow_private_network_urls {
            return Err("Browser public-only networking cannot safely contain redirects, DNS changes, or page subresources. Use WebFetch, or explicitly set AllowPrivateNetworkURLs=true to allow unrestricted/private browser networking.".into());
        }
        let url = network::go_url(&input.url)?;
        let screenshot = input.action == "screenshot";
        if self.config.access == AccessMode::ReadOnly && screenshot {
            return Err("screenshot requires workspace-write access".into());
        }
        let (width, height) = viewport(input.width, input.height)?;
        if !matches!(
            input.action.as_str(),
            "screenshot" | "navigate" | "get_text"
        ) {
            return Err(format!(
                "Unknown action {:?}. Supported: screenshot, navigate, get_text. For click/type/evaluate, use the Playwright MCP server.",
                input.action
            ));
        }
        let executable=self.config.executable.clone().ok_or("No Chromium/Chrome binary found. Install Chromium or configure the Playwright MCP server for browser automation.")?;
        let mut args = vec![
            "--headless".into(),
            "--disable-gpu".into(),
            "--disable-dev-shm-usage".into(),
            "--no-sandbox".into(),
        ];
        let mut temporary = None;
        let mut destination = None;
        if screenshot {
            if context.work_dir.as_os_str().is_empty() {
                return Err("workDir is required for screenshots".into());
            }
            let implicit = input.output_path.is_empty();
            let root = if implicit {
                if self.config.screenshot_dir.as_os_str().is_empty() {
                    std::env::temp_dir().join("agentsdk-browser")
                } else {
                    self.config.screenshot_dir.clone()
                }
            } else {
                context.work_dir.clone()
            };
            if implicit {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&root)
                    .map_err(|e| format!("Failed to create ephemeral screenshot directory: {e}"))?;
            }
            let workspace =
                Workspace::new(&root).map_err(|e| format!("Failed to resolve output path: {e}"))?;
            let output = if implicit {
                format!(
                    "agentsdk-browser-screenshot-{}.png",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                )
            } else {
                input.output_path
            };
            let absolute = write::resolve_existing(&write::clean(&workspace.root.join(&output)))
                .map_err(|e| format!("Failed to resolve output path: {e}"))?;
            let relative = absolute
                .strip_prefix(&workspace.root)
                .map_err(|_| "Failed to resolve output path: path is outside the workspace root")?
                .to_owned();
            write::make_parents_mode(
                &workspace,
                relative.parent().unwrap_or(Path::new(".")),
                0o700,
            )
            .map_err(|e| format!("Failed to create output directory: {e}"))?;
            let temp = Arc::new(
                adk_sandbox::ScratchDirectory::new()
                    .map_err(|e| format!("Failed to create private screenshot directory: {e}"))?,
            );
            args.push(format!(
                "--screenshot={}",
                temp.path().join("screenshot.png").display()
            ));
            args.push(format!("--window-size={width},{height}"));
            args.push("--hide-scrollbars".into());
            temporary = Some(temp);
            destination = Some((workspace, relative, absolute, implicit));
        } else {
            args.push("--dump-dom".into());
            args.push(format!("--window-size={width},{height}"));
        }
        args.push(url.to_string());
        let request = Request {
            executable,
            args,
            work_dir: context.work_dir.clone(),
            access: if screenshot {
                AccessMode::WorkspaceWrite
            } else {
                AccessMode::ReadOnly
            },
            writable_paths: temporary.iter().map(|t| t.path().to_owned()).collect(),
            scratch: temporary.clone(),
            timeout: Duration::from_secs(30),
        };
        let prefix = match input.action.as_str() {
            "screenshot" => "Screenshot failed",
            "navigate" => "Navigation failed",
            _ => "Failed to get page text",
        };
        let execution = tokio::time::timeout(
            Duration::from_secs(30),
            self.config.runner.run(context, request),
        )
        .await
        .map_err(|_| format!("{prefix}: context deadline exceeded\n"))?
        .map_err(|e| format!("{prefix}: {e}\n"))?;
        if execution.exit_code != 0 || execution.timed_out {
            return Err(format!("{prefix}: <nil>\n{}", go_string(&execution.output)));
        }
        if let Some((workspace, relative, absolute, implicit)) = destination {
            let temp = Workspace::new(temporary.as_ref().unwrap().path())
                .map_err(|e| format!("Failed to read temporary screenshot file: {e}"))?;
            let mut bytes = Vec::new();
            temp.read_file(Path::new("screenshot.png"))
                .and_then(|mut f| f.read_to_end(&mut bytes))
                .map_err(|e| format!("Failed to read temporary screenshot file: {e}"))?;
            write::atomic_write_default(&workspace, &relative, &bytes, None, 0o600)
                .map_err(|e| format!("Failed to save screenshot: {e}"))?;
            return Ok(format!(
                "Screenshot saved to {} ({width}x{height})",
                if implicit {
                    absolute.display().to_string()
                } else {
                    relative.display().to_string()
                }
            ));
        }
        let output = go_string(&execution.output);
        if input.action == "navigate" {
            let title = title(&output);
            return Ok(format!(
                "Navigated to {url}\nTitle: {}",
                if title.is_empty() {
                    "(no title)"
                } else {
                    &title
                }
            ));
        }
        let text = html::to_text(&output);
        if text.len() > 50000 {
            Ok(format!(
                "{}\n\n--- Content truncated at 50000 characters ---",
                go_string(&text.as_bytes()[..50000])
            ))
        } else {
            Ok(text)
        }
    }
}
impl Tool for Browser {
    fn for_access(&self, access: AccessMode) -> Option<Arc<dyn Tool>> {
        (access == AccessMode::ReadOnly && self.config.access != AccessMode::ReadOnly).then(|| {
            tool(Config {
                runner: self.config.runner.clone(),
                executable: self.config.executable.clone(),
                screenshot_dir: self.config.screenshot_dir.clone(),
                access,
                allow_private_network_urls: self.config.allow_private_network_urls,
            })
        })
    }
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
            let deadline = async {
                match context.operation.deadline {
                    Some(d) => tokio::time::sleep_until(d.into()).await,
                    None => std::future::pending().await,
                }
            };
            let result = tokio::select! {biased;
                _=context.operation.cancellation.cancelled()=>return Err(Error::new(ErrorCategory::Cancelled,"operation cancelled")),
                _=deadline=>return Err(Error::new(ErrorCategory::DeadlineExceeded,"deadline exceeded")),
                result=self.invoke(context,call.arguments)=>result,
            };
            let (text, is_error) = match result {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            Ok(ToolOutput {
                content: vec![Content::Text { text }],
                is_error,
                should_pause: false,
            })
        })
    }
}

/// Non-owning convenience adapter. Use `ManagedRunner` when host shutdown must
/// await cleanup of cancelled or abandoned tool calls.
impl Runner for adk_sandbox::Executor {
    fn run<'a>(
        &'a self,
        context: &'a ToolContext,
        request: Request,
    ) -> BoxFuture<'a, Result<Execution, String>> {
        Box::pin(async move {
            let expected: Vec<_> = request
                .scratch
                .iter()
                .map(|scratch| scratch.path().to_owned())
                .collect();
            if request.writable_paths != expected {
                return Err("browser writable paths must match owned scratch grants".into());
            }
            let mut command = adk_sandbox::Request::new(request.executable);
            command.args = request.args;
            command.cwd = request.work_dir;
            command.access = request.access;
            command.network = adk_sandbox::Network::Allow;
            command.timeout = Some(request.timeout);
            command.scratch = request.scratch.into_iter().collect();
            let result = adk_sandbox::Executor::run(self, &context.operation, command)
                .await
                .map_err(|e| e.to_string())?;
            context
                .operation
                .check_active()
                .map_err(|e| e.to_string())?;
            if result.truncated {
                return Err("Browser output exceeded the sandbox limit".into());
            }
            adk_security::check_secrets(&String::from_utf8_lossy(&result.stdout))
                .map_err(|e| e.to_string())?;
            adk_security::check_secrets(&String::from_utf8_lossy(&result.stderr))
                .map_err(|e| e.to_string())?;
            let mut output = result.stdout;
            output.extend(result.stderr);
            Ok(Execution {
                output,
                exit_code: result.status.code().unwrap_or(-1),
                timed_out: result.completion == adk_sandbox::Completion::TimedOut,
            })
        })
    }
}
