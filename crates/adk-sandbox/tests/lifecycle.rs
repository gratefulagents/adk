#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::{AccessMode, BoxFuture, Cancellation, Context};
use adk_sandbox::{Backend, Completion, Config, Executor, Network, OutputMode, Request};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct Cancel(CancellationToken);
impl Cancellation for Cancel {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.0.cancelled())
    }
}
fn context(token: CancellationToken) -> Context {
    Context {
        run_id: "test".into(),
        cancellation: Arc::new(Cancel(token)),
        deadline: None,
    }
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "adk-lifecycle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn local(temp: &Temp) -> Executor {
    let mut config = Config::new(&temp.0);
    config.backend = Backend::Local;
    config.term_grace = Duration::from_millis(50);
    Executor::new(config).unwrap()
}
fn shell(script: &str) -> Request {
    let mut req = Request::new("/bin/sh");
    req.args = vec!["-c".into(), script.into()];
    req.access = AccessMode::FullAccess;
    req.network = Network::Allow;
    req.timeout = Some(Duration::from_secs(5));
    req
}
async fn wait_file(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
fn assert_reaped(pid: i32) {
    let result = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
    assert_eq!(result, -1, "direct child was not reaped");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "direct child still exists"
    );
}

#[tokio::test]
async fn captures_bounds_and_sanitizes_without_killing_verbose_process() {
    let temp = Temp::new();
    let mut config = Config::new(&temp.0);
    config.backend = Backend::Local;
    config.output_limit = 73;
    config.term_grace = Duration::ZERO;
    let executor = Executor::new(config).unwrap();
    let result = executor.run(&context(CancellationToken::new()), shell("i=0; while [ $i -lt 2000 ]; do printf abc; printf def >&2; i=$((i+1)); done; echo done > completed")).await.unwrap();
    assert!(result.status.success());
    assert_eq!(result.completion, Completion::Exited);
    assert!(result.truncated);
    assert_eq!(result.stdout.len() + result.stderr.len(), 73);
    assert!(temp.0.join("completed").exists());
    let result = local(&temp)
        .run(&context(CancellationToken::new()), shell("/usr/bin/env"))
        .await
        .unwrap();
    let env = String::from_utf8(result.stdout).unwrap();
    assert!(env.contains("PATH=/usr/bin:/bin"));
    assert!(!env.contains("AWS_"));
    assert!(!env.contains("OPENAI_"));
    assert!(!env.contains("LD_LIBRARY_PATH"));
}

#[tokio::test]
async fn timeout_kills_term_ignoring_tree_and_reaps_leader() {
    let temp = Temp::new();
    let mut request = shell(
        "echo $$ > leader; trap '' TERM; (trap '' TERM; while :; do echo x >> heartbeat; sleep 0.02; done) & wait",
    );
    request.timeout = Some(Duration::from_millis(200));
    let result = local(&temp)
        .run(&context(CancellationToken::new()), request)
        .await
        .unwrap();
    assert_eq!(result.completion, Completion::TimedOut);
    assert!(!result.status.success());
    assert_reaped(
        fs::read_to_string(temp.0.join("leader"))
            .unwrap()
            .trim()
            .parse()
            .unwrap(),
    );
    let size = fs::metadata(temp.0.join("heartbeat")).unwrap().len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(size, fs::metadata(temp.0.join("heartbeat")).unwrap().len());
}

#[tokio::test]
async fn cancellation_and_abandoned_future_trigger_owned_cleanup() {
    for abandon in [false, true] {
        let temp = Temp::new();
        let executor = local(&temp);
        let token = CancellationToken::new();
        let ctx = context(token.clone());
        let task = tokio::spawn(async move {
            executor
                .run(
                    &ctx,
                    shell("echo $$ > leader; trap '' TERM; while :; do sleep 1; done"),
                )
                .await
        });
        wait_file(&temp.0.join("leader")).await;
        let pid: i32 = fs::read_to_string(temp.0.join("leader"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        if abandon {
            task.abort();
            let _ = task.await;
        } else {
            token.cancel();
            assert_eq!(
                task.await.unwrap().unwrap().completion,
                Completion::Cancelled
            );
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_reaped(pid);
    }
}

#[tokio::test]
async fn normal_leader_exit_also_cleans_background_group() {
    let temp = Temp::new();
    let result = local(&temp).run(&context(CancellationToken::new()), shell("(trap '' TERM; while :; do echo x >> heartbeat; sleep 0.02; done) & sleep 0.1; exit 7")).await.unwrap();
    assert_eq!(result.status.code(), Some(7));
    let size = fs::metadata(temp.0.join("heartbeat")).unwrap().len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(size, fs::metadata(temp.0.join("heartbeat")).unwrap().len());
}

#[tokio::test]
async fn pty_is_private_terminal_with_dimensions_and_bounded_capture() {
    let temp = Temp::new();
    let mut req = shell("test -t 0 && test -t 1 && test -t 2 && stty size; echo error >&2");
    req.output = OutputMode::Pty { rows: 31, cols: 93 };
    let result = local(&temp)
        .run(&context(CancellationToken::new()), req)
        .await
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("31 93"), "{output}");
    assert!(output.contains("error"));
    assert!(result.stderr.is_empty());
}

#[tokio::test]
async fn pty_timeout_and_context_deadline_cleanup() {
    let temp = Temp::new();
    let mut req = shell("echo $$ > leader; trap '' TERM; while :; do printf x; done");
    req.output = OutputMode::Pty { rows: 24, cols: 80 };
    let mut ctx = context(CancellationToken::new());
    ctx.deadline = Some(Instant::now() + Duration::from_millis(150));
    let result = local(&temp).run(&ctx, req).await.unwrap();
    assert_eq!(result.completion, Completion::TimedOut);
    assert_reaped(
        fs::read_to_string(temp.0.join("leader"))
            .unwrap()
            .trim()
            .parse()
            .unwrap(),
    );
}

#[tokio::test]
async fn policy_path_and_pre_cancel_are_fail_closed() {
    let temp = Temp::new();
    let executor = local(&temp);
    for access in [AccessMode::ReadOnly, AccessMode::WorkspaceWrite] {
        let mut req = shell("touch forbidden");
        req.access = access;
        assert!(
            executor
                .run(&context(CancellationToken::new()), req)
                .await
                .is_err()
        );
    }
    for cwd in [PathBuf::from(".."), PathBuf::from("/")] {
        let mut req = shell("touch forbidden");
        req.cwd = cwd;
        assert!(
            executor
                .run(&context(CancellationToken::new()), req)
                .await
                .is_err()
        );
    }
    let outside = Temp::new();
    std::os::unix::fs::symlink(&outside.0, temp.0.join("escape")).unwrap();
    let mut req = shell("touch forbidden");
    req.cwd = "escape".into();
    assert!(
        executor
            .run(&context(CancellationToken::new()), req)
            .await
            .is_err()
    );
    let mut req = shell("touch forbidden");
    req.network = Network::Deny;
    assert!(
        executor
            .run(&context(CancellationToken::new()), req)
            .await
            .is_err()
    );
    let token = CancellationToken::new();
    token.cancel();
    assert!(
        executor
            .run(&context(token), shell("touch forbidden"))
            .await
            .is_err()
    );
    assert!(!temp.0.join("forbidden").exists());
    assert!(Executor::new(Config::new("/")).is_err());
}

#[tokio::test]
async fn explicit_handle_cleanup_is_awaitable() {
    let temp = Temp::new();
    let running = local(&temp)
        .start(
            &context(CancellationToken::new()),
            shell("echo $$ > leader; trap '' TERM; while :; do sleep 1; done"),
        )
        .unwrap();
    wait_file(&temp.0.join("leader")).await;
    let pid = fs::read_to_string(temp.0.join("leader"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let result = running.cancel_and_wait().await.unwrap();
    assert_eq!(result.completion, Completion::Cancelled);
    assert_reaped(pid);
}

#[tokio::test]
async fn overflowing_timeout_and_missing_program_do_not_spawn() {
    let temp = Temp::new();
    let mut req = shell("touch forbidden");
    req.timeout = Some(Duration::MAX);
    assert!(
        local(&temp)
            .run(&context(CancellationToken::new()), req)
            .await
            .is_err()
    );
    let mut req = shell("touch forbidden");
    req.program = temp.0.join("nonexistent");
    assert!(
        local(&temp)
            .run(&context(CancellationToken::new()), req)
            .await
            .is_err()
    );
    assert!(!temp.0.join("forbidden").exists());
}
