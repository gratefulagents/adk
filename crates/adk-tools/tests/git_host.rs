#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::{AccessMode, BoxFuture, Context, Error, ErrorCategory, ToolContext, ToolPolicy};
use adk_sandbox::{Backend, Config, Executor, Network};
use adk_tools::{git, git_host};
use git::{Command, CommandRunner, Program, RepositoryHost};
use git_host::{ExecutionPolicy, ExecutorCommandRunner, RunnerPolicy, WorkspaceRepositoryHost};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn context(root: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "git-host-test".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: ToolPolicy {
            access: AccessMode::WorkspaceWrite,
            ..Default::default()
        },
        idempotency_key: Some("invocation-1".into()),
    }
}
fn host(root: &Path) -> WorkspaceRepositoryHost {
    WorkspaceRepositoryHost::new(root, Path::new("repos")).unwrap()
}
fn clean(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        assert!(
            !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".agentsdk-")
        );
        if entry.file_type().unwrap().is_dir() {
            clean(&entry.path());
        }
    }
}

#[tokio::test]
async fn resolves_existing_in_root_aliases_and_missing_tails() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("actual")).unwrap();
    symlink("actual", root.path().join("alias")).unwrap();
    let host = host(root.path());
    let ctx = context(root.path());
    assert_eq!(host.resolve(&ctx, ".").await.unwrap(), root.path());
    assert_eq!(
        host.resolve(&ctx, "alias/missing/child").await.unwrap(),
        root.path().join("actual/missing/child")
    );
    let resolved = host.resolve(&ctx, "alias/missing/child").await.unwrap();
    host.create_dir_all(&ctx, &resolved).await.unwrap();
    assert!(resolved.is_dir());
    assert!(host.exists(&ctx, &resolved).await.unwrap());
    assert!(!host.is_git_repository(&ctx, &resolved).await.unwrap());
    assert!(host.resolve(&ctx, "../escape").await.is_err());
    assert!(WorkspaceRepositoryHost::new(root.path(), Path::new(".")).is_err());
}

#[tokio::test]
async fn symlink_escapes_and_replacements_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("outside")).unwrap();
    symlink("absent", root.path().join("dangling")).unwrap();
    let host = host(root.path());
    let ctx = context(root.path());
    assert!(host.resolve(&ctx, "outside/new").await.is_err());
    assert!(host.resolve(&ctx, "dangling/new").await.is_err());
    assert!(
        host.exists(&ctx, &root.path().join("outside"))
            .await
            .is_err()
    );
    let resolved = host.resolve(&ctx, "safe/new").await.unwrap();
    symlink(outside.path(), root.path().join("safe")).unwrap();
    assert!(host.create_dir_all(&ctx, &resolved).await.is_err());
    assert!(!outside.path().join("new").exists());
}

#[tokio::test]
async fn git_directory_and_only_git_entry_semantics() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let ctx = context(root.path());
    let repo = root.path().join("repos/project");
    host.create_dir_all(&ctx, &repo).await.unwrap();
    assert!(host.only_git_entry(&ctx, &repo).await.unwrap());
    fs::write(repo.join(".git"), "gitdir: /outside").unwrap();
    assert!(!host.is_git_repository(&ctx, &repo).await.unwrap());
    assert!(host.ensure_exclude(&ctx, &repo, "repos/").await.is_err());
    fs::remove_file(repo.join(".git")).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    assert!(host.is_git_repository(&ctx, &repo).await.unwrap());
    assert!(host.only_git_entry(&ctx, &repo).await.unwrap());
    fs::write(repo.join("README"), "keep").unwrap();
    assert!(!host.only_git_entry(&ctx, &repo).await.unwrap());
    assert!(
        !host
            .only_git_entry(&ctx, &repo.join("missing"))
            .await
            .unwrap()
    );
    assert!(
        !host
            .only_git_entry(&ctx, &repo.join("README"))
            .await
            .unwrap()
    );
    assert!(host.remove_all(&ctx, &repo).await.is_err());
    assert_eq!(fs::read_to_string(repo.join("README")).unwrap(), "keep");
    clean(root.path());
}

#[tokio::test]
async fn excludes_append_exact_lines_atomically_and_preserve_modes() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let ctx = context(root.path());
    fs::create_dir(root.path().join(".git")).unwrap();
    host.ensure_exclude(&ctx, root.path(), "repos/")
        .await
        .unwrap();
    let path = root.path().join(".git/info/exclude");
    assert_eq!(fs::read(&path).unwrap(), b"repos/\n");
    fs::write(&path, b"# repos/\nrepos/other\nlast").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    host.ensure_exclude(&ctx, root.path(), "repos/")
        .await
        .unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        b"# repos/\nrepos/other\nlast\nrepos/\n"
    );
    let metadata = fs::metadata(&path).unwrap();
    assert_ne!(metadata.ino(), inode);
    assert_eq!(metadata.mode() & 0o777, 0o640);
    host.ensure_exclude(&ctx, root.path(), "repos/")
        .await
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().ino(), metadata.ino());
    fs::write(&path, b"repos/\r\n").unwrap();
    host.ensure_exclude(&ctx, root.path(), "repos/")
        .await
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"repos/\r\n");
    assert!(
        host.ensure_exclude(&ctx, root.path(), "x\ny")
            .await
            .is_err()
    );
    clean(root.path());
}

#[tokio::test]
async fn excludes_reject_git_info_file_symlinks_and_hardlinks() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), "untouched").unwrap();
    for component in [".git", ".git/info", ".git/info/exclude"] {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join(component);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        symlink(
            if component.ends_with("exclude") {
                outside.path().join("sentinel")
            } else {
                outside.path().into()
            },
            &target,
        )
        .unwrap();
        let host = host(root.path());
        assert!(
            host.ensure_exclude(&context(root.path()), root.path(), "repos/")
                .await
                .is_err(),
            "{component}"
        );
        assert_eq!(
            fs::read_to_string(outside.path().join("sentinel")).unwrap(),
            "untouched"
        );
    }
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join(".git/info")).unwrap();
    fs::hard_link(
        outside.path().join("sentinel"),
        root.path().join(".git/info/exclude"),
    )
    .unwrap();
    assert!(
        host(root.path())
            .ensure_exclude(&context(root.path()), root.path(), "repos/")
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("sentinel")).unwrap(),
        "untouched"
    );
}

#[tokio::test]
async fn cleanup_is_scoped_to_partial_roots_and_invocation_and_never_follows_links() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), "keep").unwrap();
    let host = host(root.path());
    let ctx = context(root.path());
    let repo = root.path().join("repos/project");
    assert!(host.remove_all(&ctx, root.path()).await.is_err());
    assert!(host.remove_all(&ctx, &repo).await.is_err());
    assert!(!host.exists(&ctx, &root.path().join("other")).await.unwrap());
    assert!(
        host.remove_all(&ctx, &root.path().join("other"))
            .await
            .is_err()
    );
    assert!(!host.exists(&ctx, &repo).await.unwrap());
    fs::create_dir_all(repo.join(".git/objects/nested")).unwrap();
    fs::write(repo.join(".git/objects/nested/file"), "partial").unwrap();
    symlink(outside.path(), repo.join("outside")).unwrap();
    fs::hard_link(outside.path().join("sentinel"), repo.join("hardlink")).unwrap();
    let mut other = context(root.path());
    other.idempotency_key = Some("invocation-2".into());
    assert!(host.remove_all(&other, &repo).await.is_err());
    host.remove_all(&ctx, &repo).await.unwrap();
    assert!(!repo.exists());
    assert_eq!(
        fs::read_to_string(outside.path().join("sentinel")).unwrap(),
        "keep"
    );
    fs::create_dir_all(repo.join(".git")).unwrap();
    host.remove_all(&ctx, &repo).await.unwrap();
    clean(root.path());
}

#[tokio::test]
async fn cleanup_rejects_replaced_roots_and_does_not_leave_quarantine() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());
    let ctx = context(root.path());
    let repo = root.path().join("repos/project");
    fs::create_dir_all(repo.join(".git")).unwrap();
    assert!(host.only_git_entry(&ctx, &repo).await.unwrap());
    fs::rename(&repo, root.path().join("old")).unwrap();
    fs::create_dir_all(repo.join(".git")).unwrap();
    assert!(host.remove_all(&ctx, &repo).await.is_err());
    assert!(repo.join(".git").is_dir());
    fs::remove_dir_all(&repo).unwrap();
    symlink(root.path().join("old"), &repo).unwrap();
    assert!(host.remove_all(&ctx, &repo).await.is_err());
    assert!(repo.is_symlink());
    assert!(root.path().join("old/.git").is_dir());
    clean(root.path());
}

#[tokio::test]
async fn filesystem_rejects_readonly_and_wrong_workspace_but_authorized_cleanup_survives_cancellation()
 {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let host = host(root.path());
    assert!(host.resolve(&context(other.path()), ".").await.is_err());
    let mut ctx = context(root.path());
    let repo = root.path().join("repos/project");
    assert!(!host.exists(&ctx, &repo).await.unwrap());
    fs::create_dir_all(repo.join(".git")).unwrap();
    ctx.policy.access = AccessMode::ReadOnly;
    assert!(
        host.create_dir_all(&ctx, &root.path().join("new"))
            .await
            .is_err()
    );
    assert!(host.ensure_exclude(&ctx, &repo, "repos/").await.is_err());
    assert!(host.remove_all(&ctx, &repo).await.is_err());
    ctx.policy.access = AccessMode::WorkspaceWrite;
    let cancellation = adk_runtime::CancellationToken::new();
    cancellation.cancel();
    ctx.operation.cancellation = Arc::new(cancellation);
    assert_eq!(
        host.exists(&ctx, &repo).await.unwrap_err().info.category,
        ErrorCategory::Cancelled
    );
    host.remove_all(&ctx, &repo).await.unwrap();
    assert!(!repo.exists());
}

struct Policy {
    access: AccessMode,
    calls: Mutex<Vec<bool>>,
}
impl RunnerPolicy for Policy {
    fn authorize<'a>(
        &'a self,
        context: &'a ToolContext,
        command: &'a Command,
        remote: bool,
    ) -> BoxFuture<'a, Result<ExecutionPolicy, Error>> {
        Box::pin(async move {
            assert_eq!(context.operation.run_id, "git-host-test");
            self.calls.lock().unwrap().push(remote);
            if command.argv.first().map(String::as_str) == Some("push") && !remote {
                return Err(Error::new(
                    ErrorCategory::PermissionDenied,
                    "remote writes denied by host policy",
                ));
            }
            Ok(ExecutionPolicy {
                access: self.access,
                network: Network::Allow,
            })
        })
    }
    fn commit_message<'a>(
        &'a self,
        _: &'a ToolContext,
        message: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move { Ok(format!("{message}\n\nHost-attributed")) })
    }
}
fn runner(
    root: &Path,
    backend: Backend,
    policy: Arc<Policy>,
    program: &Path,
) -> ExecutorCommandRunner {
    let mut config = Config::new(root);
    config.backend = backend;
    ExecutorCommandRunner::new(
        Executor::new(config).unwrap(),
        program.into(),
        program.into(),
        policy,
        false,
    )
    .unwrap()
}
fn command(root: &Path, args: &[&str]) -> Command {
    Command {
        program: Program::Git,
        argv: args.iter().map(|s| s.to_string()).collect(),
        cwd: root.into(),
        timeout: Duration::from_secs(10),
    }
}
fn policy(access: AccessMode) -> Arc<Policy> {
    Arc::new(Policy {
        access,
        calls: Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn trusted_full_access_executes_real_git_and_requires_host_attribution_policy() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = context(root.path());
    ctx.policy.access = AccessMode::FullAccess;
    let policy = policy(AccessMode::FullAccess);
    let runner = runner(
        root.path(),
        Backend::Local,
        policy.clone(),
        Path::new("/usr/bin/git"),
    );
    let output = runner
        .run(&ctx, command(root.path(), &["init", "--quiet"]))
        .await
        .unwrap();
    assert!(output.error.is_none(), "{output:?}");
    assert!(root.path().join(".git").is_dir());
    let output = runner
        .run(&ctx, command(root.path(), &["status", "--porcelain"]))
        .await
        .unwrap();
    assert!(output.output.is_empty());
    let output = runner
        .run(
            &ctx,
            command(root.path(), &["rev-parse", "--verify", "HEAD"]),
        )
        .await
        .unwrap();
    assert!(output.error.is_some());
    assert!(output.output.contains("fatal:"));
    assert_eq!(
        runner.commit_message(&ctx, "message").await.unwrap(),
        "message\n\nHost-attributed"
    );
    assert_eq!(
        runner
            .run(&ctx, command(root.path(), &["push"]))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::PermissionDenied
    );
    assert_eq!(*policy.calls.lock().unwrap(), vec![false; 4]);
}

#[tokio::test]
async fn local_backend_never_upgrades_restricted_caller_or_host_permissions() {
    let root = tempfile::tempdir().unwrap();
    for (caller, host_access) in [
        (AccessMode::ReadOnly, AccessMode::FullAccess),
        (AccessMode::WorkspaceWrite, AccessMode::FullAccess),
        (AccessMode::FullAccess, AccessMode::ReadOnly),
        (AccessMode::FullAccess, AccessMode::WorkspaceWrite),
    ] {
        let mut ctx = context(root.path());
        ctx.policy.access = caller;
        let runner = runner(
            root.path(),
            Backend::Local,
            policy(host_access),
            Path::new("/usr/bin/git"),
        );
        assert_eq!(
            runner
                .run(&ctx, command(root.path(), &["init", "--quiet"]))
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::PermissionDenied
        );
        assert!(!root.path().join(".git").exists());
    }
}

#[tokio::test]
async fn unavailable_enforcing_backend_does_not_fall_back_to_local() {
    let root = tempfile::tempdir().unwrap();
    for access in [AccessMode::ReadOnly, AccessMode::WorkspaceWrite] {
        let mut ctx = context(root.path());
        ctx.policy.access = access;
        let runner = runner(
            root.path(),
            if cfg!(target_os = "macos") {
                Backend::Bubblewrap
            } else {
                Backend::Seatbelt
            },
            policy(access),
            Path::new("/usr/bin/git"),
        );
        assert_eq!(
            runner
                .run(&ctx, command(root.path(), &["init", "--quiet"]))
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::Unsupported
        );
        assert!(!root.path().join(".git").exists());
    }
}

#[tokio::test]
async fn executor_uses_explicit_paths_clean_environment_and_program_specific_output() {
    let root = tempfile::tempdir().unwrap();
    let probe = root.path().join("probe");
    fs::write(&probe, "#!/bin/sh\nprintf 'locale=%s\\n' \"$LC_ALL\"\nprintf 'credential=%s\\n' \"${GITHUB_TOKEN-unset}\"\nprintf 'err' >&2\nexit \"${1:-0}\"\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
    let mut ctx = context(root.path());
    ctx.policy.access = AccessMode::FullAccess;
    let runner = runner(
        root.path(),
        Backend::Local,
        policy(AccessMode::FullAccess),
        &probe,
    );
    let mut cmd = command(root.path(), &[]);
    let output = runner.run(&ctx, cmd.clone()).await.unwrap();
    assert_eq!(output.output, "locale=C\ncredential=unset\nerr");
    cmd.program = Program::Gh;
    assert_eq!(
        runner.run(&ctx, cmd.clone()).await.unwrap().output,
        "locale=C\ncredential=unset\n"
    );
    cmd.argv = vec!["3".into()];
    let output = runner.run(&ctx, cmd).await.unwrap();
    assert!(output.error.is_some());
    assert!(output.output.ends_with("err"));
    let executor = Executor::new(Config::new(root.path())).unwrap();
    assert!(
        ExecutorCommandRunner::new(
            executor,
            PathBuf::from("git"),
            probe,
            policy(AccessMode::FullAccess),
            false
        )
        .is_err()
    );
}

#[tokio::test]
async fn cancellation_and_shorter_deadlines_are_propagated() {
    let root = tempfile::tempdir().unwrap();
    let probe = root.path().join("wait");
    fs::write(&probe, "#!/bin/sh\nexec /bin/sleep 10\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
    let runner = runner(
        root.path(),
        Backend::Local,
        policy(AccessMode::FullAccess),
        &probe,
    );
    let mut ctx = context(root.path());
    ctx.policy.access = AccessMode::FullAccess;
    ctx.operation.deadline = Some(Instant::now() + Duration::from_millis(50));
    assert_eq!(
        runner
            .run(&ctx, command(root.path(), &[]))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::DeadlineExceeded
    );
    ctx.operation.deadline = None;
    ctx.policy.timeout = Some(Duration::from_millis(50));
    assert_eq!(
        runner
            .run(&ctx, command(root.path(), &[]))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::DeadlineExceeded
    );
    let cancellation = adk_runtime::CancellationToken::new();
    ctx.operation.cancellation = Arc::new(cancellation.clone());
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancellation.cancel();
    };
    ctx.policy.timeout = None;
    let (output, ()) = tokio::join!(runner.run(&ctx, command(root.path(), &[])), cancel);
    assert_eq!(output.unwrap_err().info.category, ErrorCategory::Cancelled);
}

struct PendingPolicy;
impl RunnerPolicy for PendingPolicy {
    fn authorize<'a>(
        &'a self,
        _: &'a ToolContext,
        _: &'a Command,
        _: bool,
    ) -> BoxFuture<'a, Result<ExecutionPolicy, Error>> {
        Box::pin(std::future::pending())
    }
    fn commit_message<'a>(
        &'a self,
        _: &'a ToolContext,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn pending_host_authorization_and_attribution_obey_cancellation_and_deadline() {
    let root = tempfile::tempdir().unwrap();
    let runner = ExecutorCommandRunner::new(
        Executor::new(Config::new(root.path())).unwrap(),
        "/usr/bin/git".into(),
        "/usr/bin/git".into(),
        Arc::new(PendingPolicy),
        false,
    )
    .unwrap();
    let mut ctx = context(root.path());
    ctx.operation.deadline = Some(Instant::now() + Duration::from_millis(20));
    assert_eq!(
        runner
            .run(&ctx, command(root.path(), &["init"]))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::DeadlineExceeded
    );
    assert!(!root.path().join(".git").exists());
    ctx.operation.deadline = None;
    let cancellation = adk_runtime::CancellationToken::new();
    ctx.operation.cancellation = Arc::new(cancellation.clone());
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancellation.cancel();
    };
    let (message, ()) = tokio::join!(runner.commit_message(&ctx, "message"), cancel);
    assert_eq!(message.unwrap_err().info.category, ErrorCategory::Cancelled);
}

#[tokio::test]
async fn trusted_git_commit_keeps_sandbox_hook_disabling_and_host_attribution() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = context(root.path());
    ctx.policy.access = AccessMode::FullAccess;
    let runner = runner(
        root.path(),
        Backend::Local,
        policy(AccessMode::FullAccess),
        Path::new("/usr/bin/git"),
    );
    assert!(
        runner
            .run(&ctx, command(root.path(), &["init", "--quiet"]))
            .await
            .unwrap()
            .error
            .is_none()
    );
    let hook = root.path().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\ntouch hook-was-run\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let message = runner.commit_message(&ctx, "initial").await.unwrap();
    let result = runner
        .run(
            &ctx,
            command(
                root.path(),
                &[
                    "-c",
                    "user.name=Host Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "-m",
                    &message,
                ],
            ),
        )
        .await
        .unwrap();
    assert!(result.error.is_none(), "{result:?}");
    assert!(!root.path().join("hook-was-run").exists());
    let result = runner
        .run(&ctx, command(root.path(), &["log", "-1", "--format=%B"]))
        .await
        .unwrap();
    assert!(result.output.contains("Host-attributed"));
}

#[tokio::test]
async fn directory_symlink_swap_race_never_writes_outside_workspace() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("repo/.git/info")).unwrap();
    fs::write(outside.path().join("exclude"), "outside\n").unwrap();
    let host = host(root.path());
    let ctx = context(root.path());
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    let info = root.path().join("repo/.git/info");
    let alternate = root.path().join("repo/.git/real-info");
    symlink(outside.path(), &alternate).unwrap();
    let thread = std::thread::spawn(move || {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        while flag.load(Ordering::Relaxed) {
            renameat_with(CWD, &info, CWD, &alternate, RenameFlags::EXCHANGE).unwrap();
            std::thread::yield_now();
            renameat_with(CWD, &info, CWD, &alternate, RenameFlags::EXCHANGE).unwrap();
        }
    });
    for _ in 0..100 {
        let _ = host
            .ensure_exclude(&ctx, &root.path().join("repo"), "repos/")
            .await;
    }
    running.store(false, Ordering::Relaxed);
    thread.join().unwrap();
    assert_eq!(
        fs::read_to_string(outside.path().join("exclude")).unwrap(),
        "outside\n"
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
    clean(root.path());
}

#[tokio::test]
async fn executor_blocks_secret_output_even_on_successful_gh_stderr() {
    let root = tempfile::tempdir().unwrap();
    let probe = root.path().join("probe");
    let fake = format!("ghp_{}", "A".repeat(36));
    fs::write(&probe, format!("#!/bin/sh\nprintf '%s' '{fake}' >&2\n")).unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut ctx = context(root.path());
    ctx.policy.access = AccessMode::FullAccess;
    let runner = runner(
        root.path(),
        Backend::Local,
        policy(AccessMode::FullAccess),
        &probe,
    );
    let mut cmd = command(root.path(), &[]);
    cmd.program = Program::Gh;
    let error = runner.run(&ctx, cmd).await.unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Guardrail);
    assert!(!error.to_string().contains(&fake));
}

fn native_runner(root: &Path, policy: Arc<Policy>) -> Arc<ExecutorCommandRunner> {
    let mut config = Config::new(root);
    config.backend = Backend::Local;
    config.term_grace = Duration::from_millis(150);
    Arc::new(
        ExecutorCommandRunner::new(
            Executor::new(config).unwrap(),
            "/bin/sh".into(),
            "/bin/sh".into(),
            policy,
            false,
        )
        .unwrap(),
    )
}

async fn native_pid(root: &Path) -> i32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(pid) = fs::read_to_string(root.join("leader"))
                && let Ok(pid) = pid.trim().parse()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

fn assert_native_reaped(pid: i32) {
    unsafe extern "C" {
        fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
        fn kill(pid: i32, signal: i32) -> i32;
    }
    assert_eq!(unsafe { waitpid(pid, std::ptr::null_mut(), 1) }, -1);
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(10));
    assert_eq!(
        unsafe { kill(pid, 0) },
        -1,
        "child still exists after close"
    );
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(3));
}

#[tokio::test]
async fn executor_close_reaps_active_cancelled_and_abandoned_commands() {
    for mode in ["active", "cancelled", "abandoned"] {
        let root = tempfile::tempdir().unwrap();
        let policy = policy(AccessMode::FullAccess);
        let runner = native_runner(root.path(), policy.clone());
        let mut ctx = context(root.path());
        ctx.policy.access = AccessMode::FullAccess;
        let token = adk_runtime::CancellationToken::new();
        ctx.operation.cancellation = Arc::new(token.clone());
        let cmd = command(root.path(), &["-c", "echo $$ > leader; exec sleep 30"]);
        let running = runner.clone();
        let mut operation = tokio::spawn(async move { running.run(&ctx, cmd).await });
        let pid = native_pid(root.path()).await;
        match mode {
            "cancelled" => token.cancel(),
            "abandoned" => {
                operation.abort();
                assert!((&mut operation).await.unwrap_err().is_cancelled());
            }
            _ => {}
        }
        tokio::time::timeout(Duration::from_secs(5), runner.close())
            .await
            .unwrap()
            .unwrap();
        assert_native_reaped(pid);
        runner.close().await.unwrap();
        if mode != "abandoned" {
            assert_eq!(
                operation.await.unwrap().unwrap_err().info.category,
                ErrorCategory::Cancelled
            );
        }
        let ctx = context(root.path());
        let error = runner
            .run(&ctx, command(root.path(), &["-c", "touch new-work"]))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Git runner is closed"));
        assert_eq!(policy.calls.lock().unwrap().len(), 1);
        assert!(!root.path().join("new-work").exists());
        assert!(
            runner
                .commit_message(&ctx, "message")
                .await
                .unwrap_err()
                .to_string()
                .contains("Git runner is closed")
        );
    }
}

#[tokio::test]
async fn executor_close_is_retryable_after_interruption_and_concurrent() {
    let root = tempfile::tempdir().unwrap();
    let runner = native_runner(root.path(), policy(AccessMode::FullAccess));
    let mut ctx = context(root.path());
    ctx.policy.access = AccessMode::FullAccess;
    let cmd = command(root.path(), &["-c", "echo $$ > leader; exec sleep 30"]);
    let running = runner.clone();
    let operation = tokio::spawn(async move { running.run(&ctx, cmd).await });
    let pid = native_pid(root.path()).await;
    {
        let mut close = Box::pin(runner.close());
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(close.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    let ctx = context(root.path());
    assert!(
        runner
            .run(&ctx, command(root.path(), &[]))
            .await
            .unwrap_err()
            .to_string()
            .contains("Git runner is closed")
    );
    let (first, second) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(runner.close(), runner.close())
    })
    .await
    .unwrap();
    first.unwrap();
    second.unwrap();
    assert_native_reaped(pid);
    assert_eq!(
        operation.await.unwrap().unwrap_err().info.category,
        ErrorCategory::Cancelled
    );
}

#[tokio::test]
async fn executor_abandon_signals_native_command_before_close() {
    let root = tempfile::tempdir().unwrap();
    let runner = native_runner(root.path(), policy(AccessMode::FullAccess));
    let mut ctx = context(root.path());
    ctx.policy.access = AccessMode::FullAccess;
    let cmd = command(
        root.path(),
        &[
            "-c",
            "trap 'echo cancelled > signalled' TERM; echo $$ > leader; while :; do :; done",
        ],
    );
    let running = runner.clone();
    let operation = tokio::spawn(async move { running.run(&ctx, cmd).await });
    let pid = native_pid(root.path()).await;
    operation.abort();
    assert!(operation.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !root.path().join("signalled").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), runner.close())
        .await
        .unwrap()
        .unwrap();
    assert_native_reaped(pid);
}

#[tokio::test]
async fn executor_close_reaps_attach_clone_before_partial_checkout_cleanup() {
    for mode in ["active", "cancelled", "abandoned"] {
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("clone-fixture");
        fs::write(
            &program,
            r#"#!/bin/sh
for dest do :; done
mkdir -p "$dest/.git"
echo partial > "$dest/.git/temp"
trap 'echo signalled > clone-signalled' TERM
echo $$ > leader
while :; do
    if [ ! -f "$dest/.git/temp" ]; then
        echo premature > premature-cleanup
    fi
done
"#,
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = Config::new(root.path());
        config.backend = Backend::Local;
        config.term_grace = Duration::from_millis(150);
        let runner = Arc::new(
            ExecutorCommandRunner::new(
                Executor::new(config).unwrap(),
                program.clone(),
                program,
                policy(AccessMode::FullAccess),
                false,
            )
            .unwrap(),
        );
        let tool = git::tools(
            runner.clone(),
            Arc::new(host(root.path())),
            None,
            git::Options::default(),
        )
        .into_iter()
        .find(|tool| tool.definition().name == "attach_repository")
        .unwrap();
        let mut ctx = context(root.path());
        ctx.policy.access = AccessMode::FullAccess;
        let token = adk_runtime::CancellationToken::new();
        ctx.operation.cancellation = Arc::new(token.clone());
        let mut operation = tokio::spawn(async move {
            tool.execute(
                &ctx,
                adk_core::ToolCall {
                    id: "attach".into(),
                    name: "attach_repository".into(),
                    arguments: serde_json::json!({"repository": "acme/repo"}),
                },
            )
            .await
        });
        let pid = native_pid(root.path()).await;
        let dest = root.path().join("repos/repo");
        assert!(dest.join(".git/temp").is_file());
        match mode {
            "cancelled" => token.cancel(),
            "abandoned" => {
                operation.abort();
                assert!((&mut operation).await.unwrap_err().is_cancelled());
            }
            _ => {}
        }
        tokio::time::timeout(Duration::from_secs(5), runner.close())
            .await
            .unwrap()
            .unwrap();
        assert_native_reaped(pid);
        assert!(!dest.exists(), "partial clone survived {mode}");
        assert!(root.path().join("clone-signalled").exists());
        assert!(!root.path().join("premature-cleanup").exists());
        clean(root.path());
        if mode != "abandoned" {
            assert_eq!(
                operation.await.unwrap().unwrap_err().info.category,
                ErrorCategory::Cancelled
            );
        }
    }
}
