//! SDK Git workflows with host-owned command, attribution, filesystem and artifact boundaries.
//! There is deliberately no process-spawning or ambient filesystem fallback.
use adk_core::{
    BoxFuture, Content, Error, ErrorCategory, Tool, ToolCall, ToolContext, ToolDecision,
    ToolDefinition, ToolOutput,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    Git,
    Gh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub program: Program,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

/// Nonzero process exits are outputs; sandbox, authorization and transport failures are `Err`.
/// Git output combines stdout/stderr. GH returns stdout on success, stdout + stderr on failure.
#[derive(Clone, Debug, Default)]
pub struct CommandOutput {
    pub output: String,
    pub error: Option<String>,
}

/// The host must enforce sandbox/approval/remote-write policy, cancellation and the shorter of
/// the command timeout and operation deadline. Never execute argv through a shell. Git config,
/// hooks, credential helpers and transports also need confinement; cwd alone is not a sandbox.
pub trait CommandRunner: Send + Sync {
    fn run<'a>(
        &'a self,
        context: &'a ToolContext,
        command: Command,
    ) -> BoxFuture<'a, Result<CommandOutput, Error>>;
    /// Hosts retaining commands must retain clone cleanup through completion too.
    fn run_clone<'a>(
        &'a self,
        context: &'a ToolContext,
        command: Command,
        repositories: Arc<dyn RepositoryHost>,
        dest: PathBuf,
    ) -> BoxFuture<'a, Result<CommandOutput, Error>> {
        Box::pin(async move {
            let out: Result<CommandOutput, Error> = async {
                context.operation.check_active()?;
                let out = self.run(context, command).await?;
                context.operation.check_active()?;
                Ok(out)
            }
            .await;
            let cleanup = match &out {
                Ok(out) => out.error.is_some(),
                Err(error) => matches!(
                    error.info.category,
                    ErrorCategory::Cancelled
                        | ErrorCategory::DeadlineExceeded
                        | ErrorCategory::Tool
                ),
            };
            if cleanup {
                let _ = repositories.remove_all(context, &dest).await;
            }
            out
        })
    }
    /// Apply the host's commit-attribution policy, or return the unchanged SDK message.
    fn commit_message<'a>(
        &'a self,
        context: &'a ToolContext,
        message: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactKind {
    PullRequest,
    Issue,
}

pub trait ArtifactSink: Send + Sync {
    fn record<'a>(
        &'a self,
        context: &'a ToolContext,
        kind: ArtifactKind,
        url: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>>;
    /// Recording failures are warnings, not failures of an already-created GitHub artifact.
    fn warning(&self, kind: ArtifactKind, error: &Error);
}

/// All paths are host-validated and operations must remain confined at use time, including
/// symlink races, `.git` indirection, deletion and exclude writes. Operational filesystem
/// errors use ErrorCategory::Tool; permission/cancellation/infrastructure errors propagate.
pub trait RepositoryHost: Send + Sync {
    fn resolve<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a str,
    ) -> BoxFuture<'a, Result<PathBuf, Error>>;
    fn create_dir_all<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<(), Error>>;
    fn exists<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>>;
    /// SDK repositories have a directory at `.git`, not a worktree `.git` file.
    fn is_git_repository<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>>;
    /// Return false on an unreadable directory, as well as on any entry other than `.git`.
    fn only_git_entry<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>>;
    fn remove_all<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<(), Error>>;
    /// Append a missing exact line to `.git/info/exclude`, creating parents as necessary.
    fn ensure_exclude<'a>(
        &'a self,
        context: &'a ToolContext,
        repo: &'a Path,
        pattern: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>>;
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub git_remote_writes: bool,
    pub default_base_branch: String,
    pub default_branch_name: String,
    pub repository_store_dir: String,
}

pub fn tools(
    runner: Arc<dyn CommandRunner>,
    repositories: Arc<dyn RepositoryHost>,
    sink: Option<Arc<dyn ArtifactSink>>,
    options: Options,
) -> Vec<Arc<dyn Tool>> {
    crate::capabilities()
        .iter()
        .filter(|c| {
            matches!(
                c.name.as_str(),
                "create_pull_request" | "create_github_issue" | "attach_repository"
            )
        })
        .map(|c| {
            Arc::new(GitTool {
                definition: c.definition.clone().expect("Git definition"),
                runner: runner.clone(),
                repositories: repositories.clone(),
                sink: sink.clone(),
                options: options.clone(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

struct GitTool {
    definition: ToolDefinition,
    runner: Arc<dyn CommandRunner>,
    repositories: Arc<dyn RepositoryHost>,
    sink: Option<Arc<dyn ArtifactSink>>,
    options: Options,
}

#[derive(Default)]
struct Input {
    title: String,
    body: String,
    base_branch: String,
    repo_path: String,
    draft: bool,
    labels: Vec<String>,
    assignees: Vec<String>,
    repository: String,
    repo: String,
    branch_name: String,
    alias: String,
}

// Match encoding/json's null defaults, case-insensitive field lookup and type errors.
fn parse_input(value: Value, name: &str) -> Result<Input, String> {
    let mut input = Input::default();
    if value.is_null() {
        return Ok(input);
    }
    let structure = match name {
        "create_pull_request" => "createPullRequestInput",
        "create_github_issue" => "createIssueInput",
        _ => "attachRepositoryInput",
    };
    let typ = |v: &Value| match v {
        Value::Array(_) => "array",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::Object(_) => "object",
        _ => "string",
    };
    let Some(fields) = value.as_object() else {
        return Err(format!(
            "json: cannot unmarshal {} into Go value of type git.{structure}",
            typ(&value)
        ));
    };
    for (key, v) in fields {
        if v.is_null() {
            continue;
        }
        let key: String = key
            .chars()
            .map(|c| match c {
                'ſ' => 's',
                'K' => 'k',
                _ => c.to_ascii_lowercase(),
            })
            .collect();
        let valid = match name {
            "create_pull_request" => {
                ["title", "body", "base_branch", "repo_path", "draft"].contains(&key.as_str())
            }
            "create_github_issue" => {
                ["title", "body", "labels", "assignees", "repo_path"].contains(&key.as_str())
            }
            _ => ["repository", "repo", "base_branch", "branch_name", "alias"]
                .contains(&key.as_str()),
        };
        if !valid {
            continue;
        }
        let err = |v: &Value, target: &str| {
            format!(
                "json: cannot unmarshal {} into Go struct field {structure}.{key} of type {target}",
                typ(v)
            )
        };
        match key.as_str() {
            "draft" => input.draft = v.as_bool().ok_or_else(|| err(v, "bool"))?,
            "labels" | "assignees" => {
                let items = v.as_array().ok_or_else(|| err(v, "[]string"))?;
                let values = items
                    .iter()
                    .map(|v| {
                        if v.is_null() {
                            Ok(String::new())
                        } else {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| err(v, "string"))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if key == "labels" {
                    input.labels = values;
                } else {
                    input.assignees = values;
                }
            }
            _ => {
                let s = v.as_str().ok_or_else(|| err(v, "string"))?.to_owned();
                match key.as_str() {
                    "title" => input.title = s,
                    "body" => input.body = s,
                    "base_branch" => input.base_branch = s,
                    "repo_path" => input.repo_path = s,
                    "repository" => input.repository = s,
                    "repo" => input.repo = s,
                    "branch_name" => input.branch_name = s,
                    "alias" => input.alias = s,
                    _ => unreachable!(),
                }
            }
        }
    }
    Ok(input)
}

#[derive(Serialize, Default)]
struct Output {
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue_url: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    repository: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    absolute_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    base_branch: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    branch_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    note: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    error: String,
}
fn result(out: Output) -> ToolOutput {
    let is_error = out.status == "error";
    let text = serde_json::to_string(&out)
        .expect("Git output serialization")
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    ToolOutput {
        content: vec![Content::Text { text }],
        is_error,
        should_pause: false,
    }
}
fn failure(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::Tool, message)
}
fn first<'a>(values: &[&'a str]) -> &'a str {
    values
        .iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or("")
}
fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

impl Tool for GitTool {
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
            let name = self.definition.name.as_str();
            if context.policy.decision(&self.definition) == ToolDecision::Deny
                || (name == "create_pull_request" && !self.options.git_remote_writes)
            {
                return Err(Error::new(
                    ErrorCategory::PermissionDenied,
                    format!("{name} is not authorized"),
                ));
            }
            let input = parse_input(call.arguments, name)
                .map_err(|e| failure(format!("Invalid input: {e}")));
            let executed = match input {
                Err(e) => Err(e),
                Ok(input) => match name {
                    "create_pull_request" => self.pull_request(context, input).await,
                    "create_github_issue" => self.issue(context, input).await,
                    _ => self.attach(context, input).await,
                },
            };
            context.operation.check_active()?;
            match executed {
                Ok(out) => Ok(result(out)),
                Err(e) if e.info.category == ErrorCategory::Tool => Ok(result(Output {
                    pr_url: (name == "create_pull_request").then(String::new),
                    issue_url: (name == "create_github_issue").then(String::new),
                    status: "error".into(),
                    error: e.to_string(),
                    ..Default::default()
                })),
                Err(e) => Err(e),
            }
        })
    }
}

pub fn git_command_timeout(args: &[String]) -> Duration {
    let mut iter = args.iter();
    let subcommand = loop {
        match iter.next().map(String::as_str) {
            Some("-c" | "-C") => {
                iter.next();
            }
            Some(arg) if arg.starts_with('-') => {}
            sub => break sub,
        }
    };
    Duration::from_secs(
        if matches!(
            subcommand,
            Some("clone" | "fetch" | "pull" | "push" | "ls-remote")
        ) {
            600
        } else {
            60
        },
    )
}

impl GitTool {
    async fn run(
        &self,
        ctx: &ToolContext,
        cwd: &Path,
        program: Program,
        argv: Vec<String>,
    ) -> Result<CommandOutput, Error> {
        ctx.operation.check_active()?;
        let timeout = if program == Program::Git {
            git_command_timeout(&argv)
        } else {
            Duration::from_secs(60)
        };
        let out = self
            .runner
            .run(
                ctx,
                Command {
                    program,
                    argv,
                    cwd: cwd.to_owned(),
                    timeout,
                },
            )
            .await?;
        ctx.operation.check_active()?;
        Ok(out)
    }
    async fn git(
        &self,
        ctx: &ToolContext,
        cwd: &Path,
        argv: &[&str],
    ) -> Result<CommandOutput, Error> {
        self.run(ctx, cwd, Program::Git, strings(argv)).await
    }
    async fn gh(
        &self,
        ctx: &ToolContext,
        cwd: &Path,
        argv: &[&str],
    ) -> Result<CommandOutput, Error> {
        self.run(ctx, cwd, Program::Gh, strings(argv)).await
    }
    async fn repo_dir(&self, ctx: &ToolContext, path: &str) -> Result<PathBuf, Error> {
        let path = path.trim();
        if path.is_empty() || path == "." {
            return Ok(ctx.work_dir.clone());
        }
        let dir = self
            .repositories
            .resolve(ctx, path)
            .await
            .map_err(|e| prefix(e, "repo_path rejected: "))?;
        if !self.repositories.is_git_repository(ctx, &dir).await? {
            return Err(failure(format!(
                "repo_path rejected: {path} is not a git repository"
            )));
        }
        Ok(dir)
    }
    async fn record(&self, ctx: &ToolContext, kind: ArtifactKind, url: &str) {
        if let Some(sink) = &self.sink
            && let Err(e) = sink.record(ctx, kind, url).await
        {
            sink.warning(kind, &e);
        }
    }
    async fn guard_branch(&self, ctx: &ToolContext, dir: &Path, base: &str) -> Result<(), Error> {
        let out = self
            .git(ctx, dir, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await?;
        if let Some(e) = out.error {
            return Err(failure(format!(
                "refusing to push: could not determine the current branch: {e}\n{}",
                out.output
            )));
        }
        let branch = out.output.trim();
        if branch.is_empty() {
            return Err(failure(
                "refusing to push: could not determine the current branch",
            ));
        }
        if branch == "HEAD" {
            return Err(failure(
                "refusing to push from a detached HEAD; create a work branch first (git checkout -b <branch>)",
            ));
        }
        let refusal = |kind| {
            failure(format!(
                "refusing to push {kind} branch {branch:?}; create a work branch first (git checkout -b <branch>) and open the pull request from it"
            ))
        };
        if branch == "main" || branch == "master" {
            return Err(refusal("protected"));
        }
        let default = self
            .git(
                ctx,
                dir,
                &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
            )
            .await?;
        if default.error.is_none()
            && branch
                == default
                    .output
                    .trim()
                    .strip_prefix("origin/")
                    .unwrap_or(default.output.trim())
        {
            return Err(refusal("default"));
        }
        if !base.trim().is_empty() && base.trim() == branch {
            return Err(failure(format!(
                "current branch {branch:?} is the same as base_branch; create a work branch first (git checkout -b <branch>)"
            )));
        }
        Ok(())
    }
    async fn view_pr(&self, ctx: &ToolContext, dir: &Path) -> Result<Option<String>, Error> {
        let out = self
            .gh(ctx, dir, &["pr", "view", "--json", "url", "-q", ".url"])
            .await?;
        Ok(
            (out.error.is_none() && out.output.trim().starts_with("https://"))
                .then(|| out.output.trim().to_owned()),
        )
    }
    async fn pull_request(&self, ctx: &ToolContext, input: Input) -> Result<Output, Error> {
        let dir = self.repo_dir(ctx, &input.repo_path).await?;
        self.guard_branch(ctx, &dir, &input.base_branch).await?;
        let status = self.git(ctx, &dir, &["status", "--porcelain"]).await?;
        if status.error.is_none() && !status.output.trim().is_empty() {
            let add = self.git(ctx, &dir, &["add", "-A"]).await?;
            if let Some(e) = add.error {
                return Err(failure(format!("git add failed: {e}")));
            }
            let message = if input.title.is_empty() {
                "changes from agent run"
            } else {
                &input.title
            };
            let message = self.runner.commit_message(ctx, message).await?;
            let commit = self
                .git(ctx, &dir, &["commit", "--no-verify", "-m", &message])
                .await?;
            if let Some(e) = commit.error {
                return Err(failure(format!("git commit failed: {e}")));
            }
        }
        let push = self
            .git(ctx, &dir, &["push", "--no-verify", "-u", "origin", "HEAD"])
            .await?;
        if let Some(e) = push.error {
            return Err(failure(format!("git push failed: {e}\n{}", push.output)));
        }
        let mut args = strings(&["pr", "create"]);
        let branch = self
            .git(ctx, &dir, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await?;
        if branch.error.is_none()
            && !branch.output.trim().is_empty()
            && branch.output.trim() != "HEAD"
        {
            args.extend(strings(&["--head", branch.output.trim()]));
        }
        if !input.title.is_empty() {
            args.extend(strings(&["--title", &input.title]));
        }
        if !input.body.is_empty() {
            args.extend(strings(&["--body", &input.body]));
        }
        if input.title.is_empty() && input.body.is_empty() {
            args.push("--fill".into());
        } else if !input.title.is_empty() && input.body.is_empty() {
            args.extend(strings(&["--body", ""]));
        }
        if !input.base_branch.is_empty() {
            args.extend(strings(&["--base", &input.base_branch]));
        }
        if input.draft {
            args.push("--draft".into());
        }
        let create = self.run(ctx, &dir, Program::Gh, args).await?;
        let viewed = self.view_pr(ctx, &dir).await?;
        let (url, status) = if let Some(e) = create.error {
            match viewed {
                Some(url) => (url, "PR already exists"),
                None => {
                    return Err(failure(format!(
                        "gh pr create failed: {e}\n{}",
                        create.output
                    )));
                }
            }
        } else {
            let url = viewed.unwrap_or_else(|| create.output.trim().to_owned());
            if url.is_empty() {
                return Err(failure("gh pr create returned empty output"));
            }
            (url, "PR created successfully")
        };
        self.record(ctx, ArtifactKind::PullRequest, &url).await;
        Ok(Output {
            pr_url: Some(url),
            status: status.into(),
            ..Default::default()
        })
    }
    async fn issue(&self, ctx: &ToolContext, input: Input) -> Result<Output, Error> {
        if input.title.trim().is_empty() {
            return Err(failure("title is required"));
        }
        let dir = self.repo_dir(ctx, &input.repo_path).await?;
        let mut seen = BTreeSet::new();
        let labels = input
            .labels
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && seen.insert(go_lower(s)))
            .collect::<Vec<_>>();
        if !labels.is_empty() {
            let list = self
                .gh(
                    ctx,
                    &dir,
                    &["label", "list", "--limit", "1000", "--json", "name"],
                )
                .await?;
            if let Some(e) = list.error {
                return Err(failure(format!(
                    "gh label list failed: {e}\n{}",
                    list.output
                )));
            }
            let mut existing = parse_existing_labels(&list.output)
                .map_err(|e| failure(format!("gh label list returned invalid output: {e}")))?;
            for label in &labels {
                if existing.contains(&go_lower(label)) {
                    continue;
                }
                let create = self
                    .gh(
                        ctx,
                        &dir,
                        &["label", "create", "--color", "BFD4F2", "--", label],
                    )
                    .await?;
                if let Some(e) = create.error
                    && !create.output.to_lowercase().contains("already exists")
                {
                    return Err(failure(format!(
                        "gh label create failed: {e}\n{}",
                        create.output
                    )));
                }
                existing.insert(go_lower(label));
            }
        }
        let mut args = strings(&["issue", "create", "--title", &input.title]);
        if !input.body.is_empty() {
            args.extend(strings(&["--body", &input.body]));
        }
        for label in labels {
            args.extend(strings(&["--label", label]));
        }
        for assignee in &input.assignees {
            args.extend(strings(&["--assignee", assignee]));
        }
        let out = self.run(ctx, &dir, Program::Gh, args).await?;
        if let Some(e) = out.error {
            return Err(failure(format!(
                "gh issue create failed: {e}\n{}",
                out.output
            )));
        }
        let url = out.output.trim();
        if url.is_empty() {
            return Err(failure("gh issue create returned empty output"));
        }
        if !url.starts_with("https://") {
            return Err(failure(format!(
                "gh issue create returned unexpected output: {url}"
            )));
        }
        self.record(ctx, ArtifactKind::Issue, url).await;
        Ok(Output {
            issue_url: Some(url.into()),
            status: "Issue created successfully".into(),
            ..Default::default()
        })
    }
}
fn prefix(error: Error, prefix: &str) -> Error {
    if error.info.category == ErrorCategory::Tool {
        failure(format!("{prefix}{error}"))
    } else {
        error
    }
}
fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Array(_) => "array",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::Object(_) => "object",
        _ => "string",
    }
}
fn json_syntax_error(text: &str, error: serde_json::Error) -> String {
    if error.is_eof() {
        return "unexpected end of JSON input".into();
    }
    let index = text
        .lines()
        .take(error.line().saturating_sub(1))
        .map(|s| s.len() + 1)
        .sum::<usize>()
        + error.column().saturating_sub(1);
    let ch = text
        .get(index..)
        .and_then(|s| s.chars().next())
        .unwrap_or(' ');
    let prefix = &text[..index.min(text.len())];
    for literal in ["null", "true", "false"] {
        for n in 1..literal.len() {
            if prefix.ends_with(&literal[..n]) {
                return format!(
                    "invalid character {ch:?} in literal {literal} (expecting {:?})",
                    literal.as_bytes()[n] as char
                )
                .replace('"', "'");
            }
        }
    }
    let diagnostic = error.to_string();
    let suffix = if diagnostic.starts_with("key must be a string")
        || diagnostic.starts_with("trailing comma") && prefix.trim_end().ends_with(',') && ch == '}'
    {
        "looking for beginning of object key string"
    } else if diagnostic.starts_with("expected `,` or `]`") {
        "after array element"
    } else if diagnostic.starts_with("expected `,` or `}`") {
        "after object key:value pair"
    } else if diagnostic.starts_with("expected `:`") {
        "after object key"
    } else if diagnostic.starts_with("trailing characters") {
        "after top-level value"
    } else {
        "looking for beginning of value"
    };
    format!("invalid character {ch:?} {suffix}").replace('"', "'")
}
fn parse_existing_labels(text: &str) -> Result<BTreeSet<String>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| json_syntax_error(text, e))?;
    if value.is_null() {
        return Ok(BTreeSet::new());
    }
    let structure = r#"struct { Name string "json:\"name\"" }"#;
    let items = value.as_array().ok_or_else(|| {
        format!(
            "json: cannot unmarshal {} into Go value of type []{structure}",
            json_type(&value)
        )
    })?;
    let mut labels = BTreeSet::new();
    for item in items {
        if item.is_null() {
            labels.insert(String::new());
            continue;
        }
        let fields = item.as_object().ok_or_else(|| {
            format!(
                "json: cannot unmarshal {} into Go value of type {structure}",
                json_type(item)
            )
        })?;
        let name = fields
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("name"))
            .map(|(_, v)| v);
        let name = match name {
            None | Some(Value::Null) => "",
            Some(v) => v.as_str().ok_or_else(|| {
                format!(
                    "json: cannot unmarshal {} into Go struct field .name of type string",
                    json_type(v)
                )
            })?,
        };
        labels.insert(go_lower(name));
    }
    Ok(labels)
}

impl GitTool {
    async fn clone_repo(
        &self,
        ctx: &ToolContext,
        root: &Path,
        dest: &Path,
        url: &str,
        base: &str,
    ) -> Result<CommandOutput, Error> {
        let mut args = strings(&[
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "clone",
            "--depth",
            "1",
            "--single-branch",
            "--no-tags",
        ]);
        if !base.trim().is_empty() {
            args.extend(strings(&["--branch", base.trim()]));
        }
        args.extend(strings(&["--", url, &dest.to_string_lossy()]));
        self.runner
            .run_clone(
                ctx,
                Command {
                    program: Program::Git,
                    timeout: git_command_timeout(&args),
                    argv: args,
                    cwd: root.to_owned(),
                },
                self.repositories.clone(),
                dest.to_owned(),
            )
            .await
    }
    async fn attach(&self, ctx: &ToolContext, input: Input) -> Result<Output, Error> {
        let (url, name) =
            normalize_repository(first(&[&input.repository, &input.repo])).map_err(failure)?;
        let mut base = first(&[&input.base_branch, &self.options.default_base_branch]).to_owned();
        let branch = first(&[&input.branch_name, &self.options.default_branch_name]);
        let alias = sanitize_alias(first(&[&input.alias, &name]));
        if alias.is_empty() {
            return Err(failure("alias could not be derived from repository"));
        }
        if ctx.work_dir.to_string_lossy().trim().is_empty() {
            return Err(failure("workspace root is required"));
        }
        let fs = &self.repositories;
        fs.create_dir_all(ctx, &ctx.work_dir)
            .await
            .map_err(|e| prefix(e, "creating workspace: "))?;
        let root = fs
            .resolve(ctx, ".")
            .await
            .map_err(|e| prefix(e, "workspace path rejected: "))?;
        let store = fs
            .resolve(ctx, first(&[&self.options.repository_store_dir, "repos"]))
            .await
            .map_err(|e| prefix(e, "repository store path rejected: "))?;
        let dest = store.join(&alias);
        let mut existing = fs
            .exists(ctx, &dest)
            .await
            .map_err(|e| prefix(e, "checking repository destination: "))?;
        if existing && fs.is_git_repository(ctx, &dest).await? {
            let head = self
                .git(
                    ctx,
                    &dest,
                    &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
                )
                .await?;
            if head.error.is_some() && fs.only_git_entry(ctx, &dest).await? {
                fs.remove_all(ctx, &dest).await.map_err(|e| {
                    prefix(
                        e,
                        &format!("removing incomplete clone at {}: ", dest.display()),
                    )
                })?;
                existing = false;
            }
        }
        let mut note = String::new();
        let status = if existing {
            if !fs.is_git_repository(ctx, &dest).await? {
                return Err(failure(format!(
                    "repository alias {alias:?} already exists at {} but is not a git repository",
                    dest.display()
                )));
            }
            let origin = self
                .git(ctx, &dest, &["remote", "get-url", "origin"])
                .await?;
            if let Some(e) = origin.error {
                return Err(failure(format!(
                    "repository alias {alias:?} already exists at {} but its origin could not be read: {e}\n{}",
                    dest.display(),
                    origin.output
                )));
            }
            let origin = origin.output.trim();
            if normalize_repository(origin).map(|r| r.0).ok().as_deref() != Some(&url) {
                return Err(failure(format!(
                    "repository alias {alias:?} already exists at {} with origin {origin:?}, not {url:?}",
                    dest.display()
                )));
            }
            "already_attached"
        } else {
            fs.create_dir_all(ctx, &store)
                .await
                .map_err(|e| prefix(e, "creating repository store: "))?;
            let clone = self.clone_repo(ctx, &root, &dest, &url, &base).await?;
            if let Some(e) = clone.error {
                let lower = clone.output.to_lowercase();
                if !base.trim().is_empty()
                    && lower.contains("remote branch")
                    && lower.contains("not found in upstream")
                {
                    let retry = self.clone_repo(ctx, &root, &dest, &url, "").await?;
                    if let Some(e) = retry.error {
                        return Err(clone_failure(&dest, &e, &retry.output));
                    }
                    note = format!(
                        "requested base branch {base:?} not found on the remote; cloned the repository default branch instead"
                    );
                    base.clear();
                } else {
                    return Err(clone_failure(&dest, &e, &clone.output));
                }
            }
            if !branch.is_empty() {
                let checkout = self.git(ctx, &dest, &["checkout", "-B", branch]).await?;
                if let Some(e) = checkout.error {
                    return Err(failure(format!(
                        "git checkout failed: {e}\n{}",
                        checkout.output
                    )));
                }
            }
            if fs.is_git_repository(ctx, &ctx.work_dir).await?
                && let Ok(rel) = store.strip_prefix(&root)
                && !rel.as_os_str().is_empty()
            {
                fs.ensure_exclude(ctx, &ctx.work_dir, &format!("{}/", rel.display()))
                    .await
                    .map_err(|e| prefix(e, "updating git exclude: "))?;
            }
            "attached"
        };
        let path = dest
            .strip_prefix(&root)
            .map_err(|e| failure(format!("computing repository path: {e}")))?;
        Ok(Output {
            status: status.into(),
            repository: url,
            path: path.to_string_lossy().into_owned(),
            absolute_path: dest.to_string_lossy().into_owned(),
            base_branch: base,
            branch_name: branch.into(),
            note,
            ..Default::default()
        })
    }
}
fn clone_failure(dest: &Path, error: &str, output: &str) -> Error {
    let mut msg = format!("git clone failed: {error}\n{output}");
    let lower = error.to_lowercase();
    if lower.contains("signal: killed") || lower.contains("timeout") {
        msg.push_str(&format!("\nThe clone was interrupted before it completed; the partial checkout at {} was removed. Large repositories may need a retry.", dest.display()));
    }
    failure(msg)
}
fn valid_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.starts_with('-')
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

pub fn normalize_repository(raw: &str) -> Result<(String, String), String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("repository is required".into());
    }
    if raw.contains([' ', '\t', '\r', '\n']) || raw.starts_with('-') {
        return Err("repository must be a GitHub owner/repository name or credential-free HTTPS github.com URL".into());
    }
    let mut repo = raw.to_owned();
    let parts: Vec<_> = raw.split('/').collect();
    if raw.to_lowercase().starts_with("github.com/") {
        repo = format!("https://{raw}");
    } else if parts.len() == 2
        && valid_part(parts[0])
        && valid_part(parts[1].strip_suffix(".git").unwrap_or(parts[1]))
    {
        repo = format!("https://github.com/{raw}");
    }
    // Do not use WHATWG URL normalization: it erases ports, dot segments and credentials.
    let (_, authority_path) = repo
        .split_once("://")
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
        .ok_or("repository must be a credential-free HTTPS github.com owner/repository URL")?;
    let authority_path = authority_path.strip_suffix('#').unwrap_or(authority_path);
    let authority_path = authority_path.strip_suffix('?').unwrap_or(authority_path);
    let (authority, path) = authority_path
        .split_once('/')
        .unwrap_or((authority_path, ""));
    let url_error = "repository must be a credential-free HTTPS github.com owner/repository URL";
    if !authority.eq_ignore_ascii_case("github.com")
        || path.contains(['?', '#'])
        || path.bytes().any(|b| b < 0x20 || b == 0x7f)
    {
        return Err(url_error.into());
    }
    let mut decoded = Vec::new();
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hi = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or(url_error)?;
            let lo = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or(url_error)?;
            decoded.push((hi * 16 + lo) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let mut escaped = String::new();
    for byte in &decoded {
        if byte.is_ascii_alphanumeric() || b"-_.~$&+,/:;=@".contains(byte) {
            escaped.push(*byte as char);
        } else {
            escaped.push_str(&format!("%{byte:02X}"));
        }
    }
    if escaped != path {
        return Err(url_error.into());
    }
    let path = std::str::from_utf8(&decoded)
        .map_err(|_| "repository must identify exactly one GitHub owner/repository")?;
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 2 || !valid_part(parts[0]) || !valid_part(parts[1]) {
        return Err("repository must identify exactly one GitHub owner/repository".into());
    }
    Ok((
        format!("https://github.com/{}/{}.git", parts[0], parts[1]),
        parts[1].into(),
    ))
}
pub fn sanitize_alias(raw: &str) -> String {
    let raw = raw.strip_suffix(".git").unwrap_or(raw).trim();
    let mut out = String::new();
    let mut dash = false;
    for ch in go_lower(raw).chars() {
        if ch.is_ascii_alphanumeric() || "-_.".contains(ch) {
            out.push(ch);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches(['.', '-', '_']);
    out[..out.len().min(80)]
        .trim_matches(['.', '-', '_'])
        .to_owned()
}

fn go_lower(value: &str) -> String {
    value
        .chars()
        .map(|c| c.to_lowercase().next().expect("lowercase character"))
        .collect()
}
