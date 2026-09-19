//! Linux repository filesystem operations and opt-in, host-authorized Git execution.
use crate::{
    git::{Command, CommandOutput, CommandRunner, Program, RepositoryHost},
    workspace::Workspace,
    write::{atomic_write, make_parents, resolve_existing},
};
use adk_core::{AccessMode, BoxFuture, Cancellation, Error, ErrorCategory, ToolContext};
use adk_sandbox::{Completion, Executor, Network, Request};
use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, RenameFlags, openat, renameat_with, statat, unlinkat,
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

fn denied(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::PermissionDenied, message)
}
fn filesystem(error: io::Error) -> Error {
    let category = if error.kind() == io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(18 | 40))
    {
        ErrorCategory::PermissionDenied
    } else {
        ErrorCategory::Tool
    };
    Error::new(category, error.to_string()).with_source(error)
}

#[derive(Clone)]
struct CleanupGrant {
    run: String,
    invocation: Option<String>,
    identity: Option<(u64, u64)>,
}

/// A pinned workspace with cleanup authority limited to immediate store children.
/// `exists(false)` authorizes a new clone; `only_git_entry(true)` authorizes an
/// incomplete checkout. Grants are scoped to the invocation that observed it.
/// Construct one host per workspace and use the same store as `git::Options`.
pub struct WorkspaceRepositoryHost {
    workspace: Workspace,
    store: PathBuf,
    cleanup: Mutex<BTreeMap<PathBuf, CleanupGrant>>,
}

impl WorkspaceRepositoryHost {
    pub fn new(root: &Path, repository_store: &Path) -> Result<Self, Error> {
        let workspace = Workspace::new(root).map_err(filesystem)?;
        let store = Self::resolve_path(&workspace, repository_store)?;
        if store == workspace.root {
            return Err(denied("repository store must be below the workspace root"));
        }
        Ok(Self {
            workspace,
            store,
            cleanup: Mutex::new(BTreeMap::new()),
        })
    }

    fn resolve_path(workspace: &Workspace, path: &Path) -> Result<PathBuf, Error> {
        let input = path.to_str().ok_or_else(|| denied("path must be UTF-8"))?;
        let relative = workspace.relative(input).map_err(filesystem)?;
        let resolved = resolve_existing(&workspace.root.join(relative)).map_err(filesystem)?;
        let relative = resolved
            .strip_prefix(&workspace.root)
            .map_err(|_| denied("path is outside the workspace root"))?;
        let mut existing = relative;
        loop {
            match workspace.open(if existing.as_os_str().is_empty() {
                Path::new(".")
            } else {
                existing
            }) {
                Ok(_) => return Ok(resolved),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    existing = existing.parent().ok_or_else(|| filesystem(error))?;
                }
                Err(error) => return Err(filesystem(error)),
            }
        }
    }

    fn check(&self, context: &ToolContext, write: bool, cleanup: bool) -> Result<(), Error> {
        if !cleanup {
            context.operation.check_active()?;
        }
        if context.work_dir.canonicalize().map_err(filesystem)? != self.workspace.root {
            return Err(denied("repository host belongs to another workspace"));
        }
        if write && context.policy.access == AccessMode::ReadOnly {
            return Err(denied(
                "repository mutation requires writable caller access",
            ));
        }
        Ok(())
    }

    fn relative(&self, path: &Path) -> Result<PathBuf, Error> {
        self.workspace
            .relative(path.to_str().ok_or_else(|| denied("path must be UTF-8"))?)
            .map_err(filesystem)
    }

    fn grant(&self, context: &ToolContext, relative: &Path, identity: Option<(u64, u64)>) {
        let absolute = self.workspace.root.join(relative);
        if absolute.parent() == Some(self.store.as_path())
            && context.policy.access != AccessMode::ReadOnly
        {
            self.cleanup.lock().expect("cleanup lock").insert(
                relative.to_owned(),
                CleanupGrant {
                    run: context.operation.run_id.clone(),
                    invocation: context.idempotency_key.clone(),
                    identity,
                },
            );
        }
    }
}

fn only_git(directory: &File) -> io::Result<bool> {
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." && name != b".git" {
            return Ok(false);
        }
    }
    Ok(true)
}

fn remove_contents(directory: &File) -> io::Result<()> {
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        let metadata = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if FileType::from_raw_mode(metadata.st_mode) == FileType::Directory {
            let child = File::from(openat(
                directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?);
            remove_contents(&child)?;
            unlinkat(directory, name, AtFlags::REMOVEDIR)?;
        } else {
            unlinkat(directory, name, AtFlags::empty())?;
        }
    }
    Ok(())
}

impl RepositoryHost for WorkspaceRepositoryHost {
    fn resolve<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a str,
    ) -> BoxFuture<'a, Result<PathBuf, Error>> {
        Box::pin(async move {
            self.check(context, false, false)?;
            Self::resolve_path(&self.workspace, Path::new(path))
        })
    }

    fn create_dir_all<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.check(context, true, false)?;
            make_parents(&self.workspace, &self.relative(path)?).map_err(filesystem)
        })
    }

    fn exists<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            self.check(context, false, false)?;
            let relative = self.relative(path)?;
            match self.workspace.open(&relative) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    self.grant(context, &relative, None);
                    Ok(false)
                }
                Err(error) => Err(filesystem(error)),
            }
        })
    }

    fn is_git_repository<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            self.check(context, false, false)?;
            let relative = self.relative(path)?.join(".git");
            match self.workspace.open(&relative) {
                Ok(file) => Ok(file.metadata().map_err(filesystem)?.is_dir()),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
                {
                    Ok(false)
                }
                Err(error) => Err(filesystem(error)),
            }
        })
    }

    fn only_git_entry<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            self.check(context, false, false)?;
            let relative = self.relative(path)?;
            let Ok(directory) = self.workspace.open(&relative) else {
                return Ok(false);
            };
            if !only_git(&directory).unwrap_or(false) {
                return Ok(false);
            }
            let metadata = directory.metadata().map_err(filesystem)?;
            if !metadata.is_dir() {
                return Ok(false);
            }
            self.grant(context, &relative, Some((metadata.dev(), metadata.ino())));
            Ok(true)
        })
    }

    fn remove_all<'a>(
        &'a self,
        context: &'a ToolContext,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            // Cleanup remains available after cancellation, but only with a prior invocation grant.
            self.check(context, true, true)?;
            let relative = self.relative(path)?;
            let grant = self
                .cleanup
                .lock()
                .expect("cleanup lock")
                .get(&relative)
                .cloned()
                .filter(|g| {
                    g.run == context.operation.run_id && g.invocation == context.idempotency_key
                })
                .ok_or_else(|| denied("path is not an authorized partial clone root"))?;
            let parent = self
                .workspace
                .open(relative.parent().unwrap_or(Path::new(".")))
                .map_err(filesystem)?;
            let name = relative
                .file_name()
                .ok_or_else(|| denied("cannot remove workspace root"))?;
            let temporary = format!(".agentsdk-git-cleanup-{}", uuid::Uuid::new_v4().simple());
            match renameat_with(&parent, name, &parent, &temporary, RenameFlags::NOREPLACE) {
                Ok(()) => {}
                Err(rustix::io::Errno::NOENT) => return Ok(()),
                Err(error) => return Err(filesystem(error.into())),
            }
            let result = (|| -> io::Result<()> {
                let directory = File::from(openat(
                    &parent,
                    &temporary,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )?);
                if let Some(identity) = grant.identity {
                    let metadata = directory.metadata()?;
                    if identity != (metadata.dev(), metadata.ino()) || !only_git(&directory)? {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "partial clone changed after authorization",
                        ));
                    }
                }
                remove_contents(&directory)?;
                unlinkat(&parent, &temporary, AtFlags::REMOVEDIR)?;
                Ok(())
            })();
            if let Err(error) = result {
                if let Err(restore) =
                    renameat_with(&parent, &temporary, &parent, name, RenameFlags::NOREPLACE)
                {
                    return Err(Error::new(
                        ErrorCategory::Tool,
                        format!(
                            "{error}; cleanup root remains quarantined as {temporary}: {restore}"
                        ),
                    ));
                }
                return Err(filesystem(error));
            }
            self.grant(context, &relative, None);
            Ok(())
        })
    }

    fn ensure_exclude<'a>(
        &'a self,
        context: &'a ToolContext,
        repo: &'a Path,
        pattern: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.check(context, true, false)?;
            if pattern.contains(['\n', '\r', '\0']) {
                return Err(Error::new(
                    ErrorCategory::InvalidInput,
                    "exclude pattern must be one line",
                ));
            }
            let git = self.relative(repo)?.join(".git");
            let directory = self.workspace.open(&git).map_err(filesystem)?;
            if !directory.metadata().map_err(filesystem)?.is_dir() {
                return Err(Error::new(ErrorCategory::Tool, ".git must be a directory"));
            }
            let path = git.join("info/exclude");
            let mut bytes = Vec::new();
            let mode = match self.workspace.read_file(&path) {
                Ok(mut file) => {
                    let mode = file.metadata().map_err(filesystem)?.mode() & 0o777;
                    file.read_to_end(&mut bytes).map_err(filesystem)?;
                    Some(mode)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(filesystem(error)),
            };
            if bytes
                .split(|b| *b == b'\n')
                .any(|line| line.strip_suffix(b"\r").unwrap_or(line) == pattern.as_bytes())
            {
                return Ok(());
            }
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(pattern.as_bytes());
            bytes.push(b'\n');
            make_parents(&self.workspace, &git.join("info")).map_err(filesystem)?;
            atomic_write(&self.workspace, &path, &bytes, mode).map_err(filesystem)
        })
    }
}

/// The host must authorize the actual argv (including approval and remote writes).
/// Credentialed workflows can instead inject their own `CommandRunner`; this
/// adapter never reads credentials or extends the sandbox environment allowlist.
pub trait RunnerPolicy: Send + Sync {
    fn authorize<'a>(
        &'a self,
        context: &'a ToolContext,
        command: &'a Command,
        git_remote_writes: bool,
    ) -> BoxFuture<'a, Result<ExecutionPolicy, Error>>;
    fn commit_message<'a>(
        &'a self,
        context: &'a ToolContext,
        message: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>>;
}

#[derive(Clone, Copy, Debug)]
pub struct ExecutionPolicy {
    pub access: AccessMode,
    pub network: Network,
}

/// Retains command completions independently of callers. Hosts should retain this
/// typed adapter and call `close().await` before shutting down Tokio; Drop only
/// signals cancellation.
pub struct ExecutorCommandRunner {
    runner: Arc<ExecutorRunner>,
    state: Mutex<RunnerState>,
}

#[derive(Default)]
struct RunnerState {
    closed: bool,
    calls: Vec<Arc<RunnerCall>>,
}

struct RunnerCall {
    cancel: tokio::sync::watch::Sender<bool>,
    completion: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

struct CancelOnDrop(tokio::sync::watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

struct CallCancellation {
    parent: Arc<dyn Cancellation>,
    cancelled: tokio::sync::watch::Receiver<bool>,
}
impl Cancellation for CallCancellation {
    fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow() || self.parent.is_cancelled()
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut cancelled = self.cancelled.clone();
            tokio::select! {
                _ = self.parent.cancelled() => {},
                _ = cancelled.wait_for(|cancelled| *cancelled) => {},
            }
        })
    }
}

struct ExecutorRunner {
    executor: Executor,
    git: PathBuf,
    gh: PathBuf,
    policy: Arc<dyn RunnerPolicy>,
    git_remote_writes: bool,
}

impl ExecutorCommandRunner {
    pub fn new(
        executor: Executor,
        git: PathBuf,
        gh: PathBuf,
        policy: Arc<dyn RunnerPolicy>,
        git_remote_writes: bool,
    ) -> Result<Self, Error> {
        for program in [&git, &gh] {
            if !program.is_absolute() || program.components().any(|p| p == Component::ParentDir) {
                return Err(Error::new(
                    ErrorCategory::InvalidInput,
                    "Git/Gh executables must be explicit absolute paths without '..'",
                ));
            }
        }
        Ok(Self {
            runner: Arc::new(ExecutorRunner {
                executor,
                git,
                gh,
                policy,
                git_remote_writes,
            }),
            state: Mutex::new(RunnerState::default()),
        })
    }

    fn run_retained<'a>(
        &'a self,
        context: &'a ToolContext,
        command: Command,
        clone: Option<(Arc<dyn RepositoryHost>, PathBuf)>,
    ) -> BoxFuture<'a, Result<CommandOutput, Error>> {
        Box::pin(async move {
            let (cancel, cancelled) = tokio::sync::watch::channel(false);
            let _cancel_on_drop = CancelOnDrop(cancel.clone());
            let mut operation = context.operation.clone();
            operation.cancellation = Arc::new(CallCancellation {
                parent: operation.cancellation,
                cancelled,
            });
            let context = ToolContext {
                operation,
                work_dir: context.work_dir.clone(),
                policy: context.policy.clone(),
                idempotency_key: context.idempotency_key.clone(),
            };
            let (sender, receiver) = tokio::sync::oneshot::channel();
            {
                let mut state = self.state.lock().unwrap();
                if state.closed {
                    return Err(Error::new(ErrorCategory::Cancelled, "Git runner is closed"));
                }
                let runner = self.runner.clone();
                // Never select-drop Executor::run: it must finish awaiting its
                // supervisor even after the caller abandons this command.
                let completion = tokio::spawn(async move {
                    let result = match clone {
                        Some((repositories, dest)) => {
                            runner
                                .run_clone(&context, command, repositories, dest)
                                .await
                        }
                        None => runner.run(&context, command).await,
                    };
                    let _ = sender.send(result);
                });
                state.calls.push(Arc::new(RunnerCall {
                    cancel,
                    completion: tokio::sync::Mutex::new(Some(completion)),
                }));
            }
            receiver
                .await
                .map_err(|error| Error::new(ErrorCategory::Host, error.to_string()))?
        })
    }

    /// Reject new work and signal every retained command, including abandoned ones.
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        for call in &state.calls {
            call.cancel.send_replace(true);
        }
    }

    /// Cancel and join retained commands through process reaping and clone cleanup. Safe to call
    /// concurrently or retry after dropping a pending close future.
    pub async fn close(&self) -> Result<(), Error> {
        self.cancel();
        let calls = self.state.lock().unwrap().calls.clone();
        let mut error = None;
        for call in calls {
            let mut completion = call.completion.lock().await;
            if let Some(task) = completion.as_mut() {
                if let Err(failure) = task.await {
                    error = Some(Error::new(ErrorCategory::Host, failure.to_string()));
                }
                completion.take();
            }
        }
        self.state.lock().unwrap().calls.clear();
        error.map_or(Ok(()), Err)
    }
}

impl Drop for ExecutorCommandRunner {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl CommandRunner for ExecutorCommandRunner {
    fn run<'a>(
        &'a self,
        context: &'a ToolContext,
        command: Command,
    ) -> BoxFuture<'a, Result<CommandOutput, Error>> {
        self.run_retained(context, command, None)
    }

    fn run_clone<'a>(
        &'a self,
        context: &'a ToolContext,
        command: Command,
        repositories: Arc<dyn RepositoryHost>,
        dest: PathBuf,
    ) -> BoxFuture<'a, Result<CommandOutput, Error>> {
        self.run_retained(context, command, Some((repositories, dest)))
    }

    fn commit_message<'a>(
        &'a self,
        context: &'a ToolContext,
        message: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            if self.state.lock().unwrap().closed {
                return Err(Error::new(ErrorCategory::Cancelled, "Git runner is closed"));
            }
            self.runner.commit_message(context, message).await
        })
    }
}

fn sandbox_error(error: adk_sandbox::Error) -> Error {
    let category = match &error {
        adk_sandbox::Error::Cancelled => ErrorCategory::Cancelled,
        adk_sandbox::Error::TimedOut => ErrorCategory::DeadlineExceeded,
        adk_sandbox::Error::Invalid(_) => ErrorCategory::PermissionDenied,
        adk_sandbox::Error::Unavailable(_) => ErrorCategory::Unsupported,
        _ => ErrorCategory::Host,
    };
    Error::new(category, error.to_string()).with_source(error)
}

async fn await_policy<T>(
    context: &ToolContext,
    future: BoxFuture<'_, Result<T, Error>>,
) -> Result<T, Error> {
    let deadline = async {
        match context.operation.deadline {
            Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        biased;
        () = context.operation.cancellation.cancelled() => Err(Error::new(ErrorCategory::Cancelled, "Git authorization cancelled")),
        () = deadline => Err(Error::new(ErrorCategory::DeadlineExceeded, "Git authorization deadline exceeded")),
        result = future => result,
    }
}

impl CommandRunner for ExecutorRunner {
    fn run<'a>(
        &'a self,
        context: &'a ToolContext,
        command: Command,
    ) -> BoxFuture<'a, Result<CommandOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let policy = await_policy(
                context,
                self.policy
                    .authorize(context, &command, self.git_remote_writes),
            )
            .await?;
            context.operation.check_active()?;
            let access = match (context.policy.access, policy.access) {
                (AccessMode::ReadOnly, _) | (_, AccessMode::ReadOnly) => AccessMode::ReadOnly,
                (AccessMode::WorkspaceWrite, _) | (_, AccessMode::WorkspaceWrite) => {
                    AccessMode::WorkspaceWrite
                }
                _ => AccessMode::FullAccess,
            };
            let mut request = Request::new(match command.program {
                Program::Git => &self.git,
                Program::Gh => &self.gh,
            });
            request.args = command.argv;
            request.cwd = command.cwd;
            request.access = access;
            request.network = policy.network;
            request.env.insert("LC_ALL".into(), "C".into());
            request.timeout = Some(
                context
                    .policy
                    .timeout
                    .map_or(command.timeout, |timeout| timeout.min(command.timeout)),
            );
            let result = self
                .executor
                .run(&context.operation, request)
                .await
                .map_err(sandbox_error)?;
            context.operation.check_active()?;
            match result.completion {
                Completion::Cancelled => {
                    return Err(Error::new(
                        ErrorCategory::Cancelled,
                        "Git command cancelled",
                    ));
                }
                Completion::TimedOut => {
                    return Err(Error::new(
                        ErrorCategory::DeadlineExceeded,
                        "Git command timed out",
                    ));
                }
                Completion::Exited => {}
            }
            if result.truncated {
                return Err(Error::new(
                    ErrorCategory::Tool,
                    "Git command output exceeded the sandbox limit",
                ));
            }
            adk_security::check_secrets(&String::from_utf8_lossy(&result.stdout))?;
            adk_security::check_secrets(&String::from_utf8_lossy(&result.stderr))?;
            let mut output = String::from_utf8_lossy(&result.stdout).into_owned();
            if command.program == Program::Git || !result.status.success() {
                output.push_str(&String::from_utf8_lossy(&result.stderr));
            }
            Ok(CommandOutput {
                output,
                error: (!result.status.success()).then(|| result.status.to_string()),
            })
        })
    }
    fn commit_message<'a>(
        &'a self,
        context: &'a ToolContext,
        message: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let message =
                await_policy(context, self.policy.commit_message(context, message)).await?;
            context.operation.check_active()?;
            Ok(message)
        })
    }
}
