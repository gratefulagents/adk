// This replay includes native Unix paths and their JSON rendering from Go.
// Windows still compiles the library and runs registry/schema/fail-closed tests.
#![cfg(unix)]
#![deny(warnings)]
use adk_core::*;
use adk_tools::capabilities;
use adk_tools::git::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize)]
struct Step {
    program: String,
    argv: Vec<String>,
    cwd: String,
    output: String,
    #[serde(default)]
    error: String,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    tool: String,
    input: Value,
    #[serde(default)]
    setup: String,
    #[serde(default)]
    base: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    store: String,
    steps: Vec<Step>,
    content: String,
    is_error: bool,
    artifacts: Vec<String>,
    exclude: String,
    removed: bool,
}
struct FakeRunner {
    root: PathBuf,
    steps: Mutex<VecDeque<Step>>,
    attribution: Option<String>,
    deny: bool,
}
impl CommandRunner for FakeRunner {
    fn run<'a>(
        &'a self,
        context: &'a ToolContext,
        command: Command,
    ) -> BoxFuture<'a, Result<CommandOutput, Error>> {
        Box::pin(async move {
            assert_eq!(context.operation.run_id, "trusted-run");
            assert_eq!(context.idempotency_key.as_deref(), Some("trusted-call"));
            context.operation.check_active()?;
            if self.deny {
                return Err(Error::new(
                    ErrorCategory::PermissionDenied,
                    "sandbox refused",
                ));
            }
            let step = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected command");
            assert_eq!(
                command.program,
                if step.program == "git" {
                    Program::Git
                } else {
                    Program::Gh
                }
            );
            let expand = |s: &str| s.replace("/WORK", &self.root.to_string_lossy());
            assert_eq!(
                command.argv,
                step.argv.iter().map(|s| expand(s)).collect::<Vec<_>>()
            );
            assert_eq!(command.cwd, PathBuf::from(expand(&step.cwd)));
            let network = command.argv.iter().any(|s| s == "clone" || s == "push");
            assert_eq!(
                command.timeout,
                Duration::from_secs(if network { 600 } else { 60 })
            );
            if command.program == Program::Git && command.argv.get(4).is_some_and(|s| s == "clone")
            {
                fs::create_dir_all(Path::new(command.argv.last().unwrap()).join(".git")).unwrap();
            }
            Ok(CommandOutput {
                output: step.output,
                error: (!step.error.is_empty()).then_some(step.error),
            })
        })
    }
    fn commit_message<'a>(
        &'a self,
        ctx: &'a ToolContext,
        message: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            ctx.operation.check_active()?;
            Ok(match &self.attribution {
                Some(trailer) => format!("{message}\n\n{trailer}"),
                None => message.into(),
            })
        })
    }
}
#[derive(Default)]
struct Sink {
    artifacts: Mutex<Vec<String>>,
    warnings: Mutex<usize>,
    fail: bool,
}
impl ArtifactSink for Sink {
    fn record<'a>(
        &'a self,
        ctx: &'a ToolContext,
        kind: ArtifactKind,
        url: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            assert_eq!(ctx.operation.run_id, "trusted-run");
            self.artifacts.lock().unwrap().push(format!(
                "{}:{url}",
                if kind == ArtifactKind::PullRequest {
                    "pr"
                } else {
                    "issue"
                }
            ));
            if self.fail {
                Err(Error::new(ErrorCategory::Host, "artifact offline"))
            } else {
                Ok(())
            }
        })
    }
    fn warning(&self, _: ArtifactKind, _: &Error) {
        *self.warnings.lock().unwrap() += 1;
    }
}
struct Repositories;
fn io_error(e: std::io::Error) -> Error {
    Error::new(ErrorCategory::Tool, e.to_string())
}
impl RepositoryHost for Repositories {
    fn resolve<'a>(
        &'a self,
        ctx: &'a ToolContext,
        input: &'a str,
    ) -> BoxFuture<'a, Result<PathBuf, Error>> {
        Box::pin(async move {
            let path = Path::new(input);
            let mut target = if path.is_absolute() {
                PathBuf::new()
            } else {
                ctx.work_dir.clone()
            };
            for c in path.components() {
                match c {
                    Component::ParentDir => {
                        target.pop();
                    }
                    Component::CurDir => {}
                    _ => target.push(c),
                }
            }
            if !target.starts_with(&ctx.work_dir) {
                return Err(Error::new(
                    ErrorCategory::Tool,
                    format!(
                        "path {input} is outside the workspace root {} - use a relative path like {:?} instead",
                        ctx.work_dir.display(),
                        path.file_name().unwrap().to_string_lossy()
                    ),
                ));
            }
            let mut existing = target.as_path();
            while !existing.exists() {
                existing = existing.parent().unwrap();
            }
            let canonical = existing.canonicalize().map_err(io_error)?;
            if !canonical.starts_with(&ctx.work_dir) {
                return Err(Error::new(
                    ErrorCategory::PermissionDenied,
                    "symlink escape",
                ));
            }
            Ok(target)
        })
    }
    fn create_dir_all<'a>(
        &'a self,
        _: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move { fs::create_dir_all(path).map_err(io_error) })
    }
    fn exists<'a>(
        &'a self,
        _: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move { Ok(path.exists()) })
    }
    fn is_git_repository<'a>(
        &'a self,
        _: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move { Ok(path.join(".git").is_dir()) })
    }
    fn only_git_entry<'a>(
        &'a self,
        _: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            Ok(fs::read_dir(path).is_ok_and(|entries| {
                entries
                    .into_iter()
                    .all(|e| e.is_ok_and(|e| e.file_name() == ".git"))
            }))
        })
    }
    fn remove_all<'a>(
        &'a self,
        _: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if path.exists() {
                fs::remove_dir_all(path).map_err(io_error)?;
            }
            Ok(())
        })
    }
    fn ensure_exclude<'a>(
        &'a self,
        _: &'a ToolContext,
        repo: &'a Path,
        pattern: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let path = repo.join(".git/info/exclude");
            fs::create_dir_all(path.parent().unwrap()).map_err(io_error)?;
            let mut content = fs::read_to_string(&path).unwrap_or_default();
            if !content.lines().any(|line| line == pattern) {
                content.push_str(pattern);
                content.push('\n');
                fs::write(path, content).map_err(io_error)?;
            }
            Ok(())
        })
    }
}
fn context(root: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "trusted-run".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: ToolPolicy {
            access: AccessMode::WorkspaceWrite,
            ..Default::default()
        },
        idempotency_key: Some("trusted-call".into()),
    }
}
fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: "call".into(),
        name: name.into(),
        arguments,
    }
}
fn text(output: &ToolOutput) -> &str {
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("expected text"),
    }
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("../../../fixtures/tools/git-expected.json")).unwrap()
}
fn runner(root: &Path, steps: Vec<Step>) -> Arc<FakeRunner> {
    Arc::new(FakeRunner {
        root: root.into(),
        steps: Mutex::new(steps.into()),
        attribution: None,
        deny: false,
    })
}
fn options() -> Options {
    Options {
        git_remote_writes: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn pinned_go_fixture_replay_exact_results_commands_artifacts_and_filesystem() {
    for c in cases() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let dest = root.join("repos/repo");
        match c.setup.as_str() {
            "root_git" => fs::create_dir_all(root.join(".git")).unwrap(),
            "existing" | "incomplete" | "working" | "subrepo" => {
                fs::create_dir_all(dest.join(".git")).unwrap();
                if c.setup == "working" {
                    fs::write(dest.join("notes.txt"), "keep").unwrap();
                }
            }
            "plain" => fs::create_dir_all(&dest).unwrap(),
            _ => {}
        }
        let runner = runner(root, c.steps);
        let sink = Arc::new(Sink::default());
        let tools = tools(
            runner.clone(),
            Arc::new(Repositories),
            Some(sink.clone()),
            Options {
                default_base_branch: c.base,
                default_branch_name: c.branch,
                repository_store_dir: c.store,
                ..options()
            },
        );
        let tool = tools
            .iter()
            .find(|t| t.definition().name == c.tool)
            .unwrap();
        let out = tool
            .execute(&context(root), call(&c.tool, c.input))
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let actual = text(&out).replace(root.to_str().unwrap(), "/WORK");
        assert_eq!(actual, c.content, "{}", c.name);
        assert_eq!(out.is_error, c.is_error, "{}", c.name);
        assert!(!out.should_pause);
        assert!(
            runner.steps.lock().unwrap().is_empty(),
            "{}: missing commands",
            c.name
        );
        assert_eq!(*sink.artifacts.lock().unwrap(), c.artifacts, "{}", c.name);
        assert_eq!(
            fs::read_to_string(root.join(".git/info/exclude")).unwrap_or_default(),
            c.exclude,
            "{}",
            c.name
        );
        assert_eq!(!dest.exists(), c.removed, "{}", c.name);
        if c.setup == "working" {
            assert_eq!(fs::read_to_string(dest.join("notes.txt")).unwrap(), "keep");
        }
    }
}

#[tokio::test]
async fn owner_gates_and_operation_cancellation_prevent_commands() {
    let temp = tempfile::tempdir().unwrap();
    for name in [
        "create_pull_request",
        "create_github_issue",
        "attach_repository",
    ] {
        let runner = runner(temp.path(), vec![]);
        let tool = tools(runner.clone(), Arc::new(Repositories), None, options())
            .into_iter()
            .find(|t| t.definition().name == name)
            .unwrap();
        let mut ctx = context(temp.path());
        ctx.policy.access = AccessMode::ReadOnly;
        let error = tool.execute(&ctx, call(name, json!({}))).await.unwrap_err();
        assert_eq!(error.info.category, ErrorCategory::PermissionDenied);
        ctx.policy.access = AccessMode::FullAccess;
        ctx.policy.denied_tools.insert(name.into());
        assert_eq!(
            tool.execute(&ctx, call(name, json!({})))
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::PermissionDenied
        );
        ctx.policy.denied_tools.clear();
        ctx.operation.deadline = Some(Instant::now());
        assert_eq!(
            tool.execute(&ctx, call(name, json!({})))
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::DeadlineExceeded
        );
        ctx.operation.deadline = None;
        let cancellation = Arc::new(adk_runtime::CancellationToken::new());
        cancellation.cancel();
        ctx.operation.cancellation = cancellation;
        assert_eq!(
            tool.execute(&ctx, call(name, json!({})))
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::Cancelled
        );
    }
    let runner = runner(temp.path(), vec![]);
    let tool = tools(runner, Arc::new(Repositories), None, Options::default())
        .into_iter()
        .find(|t| t.definition().name == "create_pull_request")
        .unwrap();
    assert_eq!(
        tool.execute(
            &context(temp.path()),
            call("create_pull_request", json!({}))
        )
        .await
        .unwrap_err()
        .info
        .category,
        ErrorCategory::PermissionDenied
    );
}

#[tokio::test]
async fn host_attribution_and_nonfatal_artifact_failure() {
    let temp = tempfile::tempdir().unwrap();
    let mut c = cases()
        .into_iter()
        .find(|c| c.name == "pr-dirty-title-draft")
        .unwrap();
    let trailer = "Co-authored-by: trusted bot <bot@example.com>";
    c.steps
        .iter_mut()
        .find(|s| s.argv.first().is_some_and(|s| s == "commit"))
        .unwrap()
        .argv[3]
        .push_str(&format!("\n\n{trailer}"));
    let runner = Arc::new(FakeRunner {
        root: temp.path().into(),
        steps: Mutex::new(c.steps.into()),
        attribution: Some(trailer.into()),
        deny: false,
    });
    let sink = Arc::new(Sink {
        fail: true,
        ..Default::default()
    });
    let tool = tools(
        runner.clone(),
        Arc::new(Repositories),
        Some(sink.clone()),
        options(),
    )
    .into_iter()
    .find(|t| t.definition().name == c.tool)
    .unwrap();
    let result = tool
        .execute(&context(temp.path()), call(&c.tool, c.input))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(*sink.warnings.lock().unwrap(), 1);
    assert!(runner.steps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn runner_authorization_errors_are_not_pr_recovery_or_incomplete_clone_signals() {
    let temp = tempfile::tempdir().unwrap();
    let runner = Arc::new(FakeRunner {
        root: temp.path().into(),
        steps: Mutex::new(VecDeque::new()),
        attribution: None,
        deny: true,
    });
    for tool in tools(runner, Arc::new(Repositories), None, options()) {
        let name = tool.definition().name.clone();
        let args = json!({"title":"Bug", "repository":"acme/repo"});
        let error = tool
            .execute(&context(temp.path()), call(&name, args))
            .await
            .unwrap_err();
        assert_eq!(error.info.category, ErrorCategory::PermissionDenied);
    }
}

#[test]
fn definitions_registry_and_timeout_parity() {
    let temp = tempfile::tempdir().unwrap();
    let tools = tools(
        runner(temp.path(), vec![]),
        Arc::new(Repositories),
        None,
        options(),
    );
    let config = adk_tools::Config {
        features: adk_tools::Features::Strict(
            ["GitHubPullRequest", "GitHubIssue", "AttachRepository"]
                .map(String::from)
                .into(),
        ),
        access: AccessMode::WorkspaceWrite,
        git_remote_writes: true,
        ..Default::default()
    };
    let registry = adk_tools::Registry::build(&config, tools).unwrap();
    assert_eq!(registry.names().count(), 3);
    for name in registry.names() {
        assert_eq!(
            registry.get(name).unwrap().definition(),
            capabilities()
                .iter()
                .find(|c| c.name == name)
                .unwrap()
                .definition
                .as_ref()
                .unwrap()
        );
    }
    let config = adk_tools::Config {
        git_remote_writes: false,
        ..config
    };
    assert!(
        !adk_tools::select(&config)
            .unwrap()
            .iter()
            .any(|c| c.name == "create_pull_request")
    );
    assert_eq!(adk_tools::select(&config).unwrap().len(), 2);
    for (args, seconds) in [
        (
            "-c protocol.allow=never -c protocol.https.allow=always clone --depth 1",
            600,
        ),
        ("fetch origin main", 600),
        ("push -u origin work", 600),
        ("-C repo pull --ff-only", 600),
        ("ls-remote origin", 600),
        ("status --porcelain", 60),
        ("rev-parse HEAD", 60),
        ("--no-pager log --oneline", 60),
    ] {
        assert_eq!(
            git_command_timeout(
                &args
                    .split_whitespace()
                    .map(String::from)
                    .collect::<Vec<_>>()
            ),
            Duration::from_secs(seconds)
        );
    }
}

struct CancelCloneRunner {
    cancellation: Arc<adk_runtime::CancellationToken>,
}
impl CommandRunner for CancelCloneRunner {
    fn run<'a>(
        &'a self,
        _: &'a ToolContext,
        command: Command,
    ) -> BoxFuture<'a, Result<CommandOutput, Error>> {
        Box::pin(async move {
            assert_eq!(command.argv[4], "clone");
            fs::create_dir_all(Path::new(command.argv.last().unwrap()).join(".git")).unwrap();
            self.cancellation.cancel();
            Err(Error::new(ErrorCategory::Cancelled, "operation cancelled"))
        })
    }
    fn commit_message<'a>(
        &'a self,
        _: &'a ToolContext,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>> {
        panic!("unexpected commit")
    }
}

#[tokio::test]
async fn cancelled_inflight_clone_cleans_partial_checkout_without_retry() {
    let temp = tempfile::tempdir().unwrap();
    let cancellation = Arc::new(adk_runtime::CancellationToken::new());
    let runner = Arc::new(CancelCloneRunner {
        cancellation: cancellation.clone(),
    });
    let mut ctx = context(temp.path());
    ctx.operation.cancellation = cancellation;
    let tool = tools(runner, Arc::new(Repositories), None, options())
        .into_iter()
        .find(|t| t.definition().name == "attach_repository")
        .unwrap();
    assert_eq!(
        tool.execute(
            &ctx,
            call("attach_repository", json!({"repository":"acme/repo"}))
        )
        .await
        .unwrap_err()
        .info
        .category,
        ErrorCategory::Cancelled
    );
    assert!(!temp.path().join("repos/repo").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn host_rejects_symlink_repo_paths_before_commands() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join("escape")).unwrap();
    for tool in tools(
        runner(temp.path(), vec![]),
        Arc::new(Repositories),
        None,
        options(),
    ) {
        if tool.definition().name == "attach_repository" {
            continue;
        }
        let args = json!({"title":"Bug","repo_path":"escape"});
        assert_eq!(
            tool.execute(&context(temp.path()), call(&tool.definition().name, args))
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::PermissionDenied
        );
    }
}

#[tokio::test]
async fn missing_workspace_created_through_host_hook() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("new-workspace");
    let c = cases()
        .into_iter()
        .find(|c| c.name == "attach-repo-alias")
        .unwrap();
    let runner = runner(&root, c.steps);
    let tool = tools(runner.clone(), Arc::new(Repositories), None, options())
        .into_iter()
        .find(|t| t.definition().name == "attach_repository")
        .unwrap();
    let out = tool
        .execute(&context(&root), call("attach_repository", c.input))
        .await
        .unwrap();
    assert!(!out.is_error);
    assert_eq!(
        text(&out).replace(root.to_str().unwrap(), "/WORK"),
        c.content
    );
    assert!(runner.steps.lock().unwrap().is_empty());
}
