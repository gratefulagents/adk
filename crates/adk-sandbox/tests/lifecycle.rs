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

async fn session_bytes(session: &mut adk_sandbox::ProcessSession, expected: usize) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        while bytes.len() < expected {
            let output = session.next_output().await;
            assert!(!output.truncated, "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            bytes.extend(output.stdout);
            assert!(!output.finished || bytes.len() >= expected);
        }
        bytes
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_pipes_are_binary_bidirectional_and_incremental() {
    let temp = Temp::new();
    let mut session = local(&temp)
        .start_session(&context(CancellationToken::new()), shell("cat"))
        .unwrap();
    let mut input = session.take_input().unwrap();
    assert!(session.take_input().is_none());
    let bytes: Vec<u8> = (0..32768).map(|n| n as u8).collect();
    input.write_all(&bytes).await.unwrap();
    assert_eq!(session_bytes(&mut session, bytes.len()).await, bytes);
    assert!(session.poll().stdout.is_empty());
    input.write_all(b"second\0round\n").await.unwrap();
    assert_eq!(session_bytes(&mut session, 13).await, b"second\0round\n");
    input.close();
    let result = session.wait().await.unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stdout.is_empty());
    assert!(!result.truncated);
}

#[tokio::test]
async fn session_wait_closes_untaken_pipe_input_and_preserves_stderr() {
    let temp = Temp::new();
    let session = local(&temp)
        .start_session(
            &context(CancellationToken::new()),
            shell("cat; printf out; printf err >&2"),
        )
        .unwrap();
    let result = session.wait().await.unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"out");
    assert_eq!(result.stderr, b"err");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_bounded_output_survives_exit_and_poll_replenishes_budget() {
    let temp = Temp::new();
    let mut config = Config::new(&temp.0);
    config.backend = Backend::Local;
    config.output_limit = 17;
    config.term_grace = Duration::ZERO;
    let executor = Executor::new(config).unwrap();
    let mut session = executor
        .start_session(
            &context(CancellationToken::new()),
            shell("i=0; while [ $i -lt 1000 ]; do printf abc; printf def >&2; i=$((i+1)); done"),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !session.is_finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let output = session.poll();
    assert!(output.finished && output.truncated);
    assert_eq!(output.stdout.len() + output.stderr.len(), 17);
    let empty = session.next_output().await;
    assert!(
        empty.finished && !empty.truncated && empty.stdout.is_empty() && empty.stderr.is_empty()
    );
    assert!(session.wait().await.unwrap().status.success());

    let mut session = executor
        .start_session(
            &context(CancellationToken::new()),
            shell("while read line; do printf 12345678901234567; done"),
        )
        .unwrap();
    let mut input = session.take_input().unwrap();
    for _ in 0..4 {
        input.write_all(b"next\n").await.unwrap();
        assert_eq!(session_bytes(&mut session, 17).await, b"12345678901234567");
    }
    input.close();
    assert!(session.wait().await.unwrap().status.success());
}

#[tokio::test]
async fn session_pty_accepts_terminal_input_and_merges_output() {
    let temp = Temp::new();
    let mut request =
        shell("stty -echo; printf ready; read line; printf 'reply:%s' \"$line\"; printf err >&2");
    request.output = OutputMode::Pty { rows: 24, cols: 80 };
    let mut session = local(&temp)
        .start_session(&context(CancellationToken::new()), request)
        .unwrap();
    let mut input = session.take_input().unwrap();
    assert_eq!(session_bytes(&mut session, 5).await, b"ready");
    input.write_all(b"hello\n").await.unwrap();
    assert_eq!(session_bytes(&mut session, 13).await, b"reply:helloerr");
    let result = session.wait().await.unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stderr.is_empty());
    assert!(input.write_all(b"late").await.is_err());
}

#[tokio::test]
async fn session_cancel_reaps_with_blocked_writer_and_background_children() {
    let temp = Temp::new();
    let mut session = local(&temp).start_session(&context(CancellationToken::new()), shell("echo $$ > leader; trap '' TERM; (trap '' TERM; while :; do echo x >> heartbeat; sleep 0.02; done) & wait")).unwrap();
    let mut input = session.take_input().unwrap();
    wait_file(&temp.0.join("leader")).await;
    wait_file(&temp.0.join("heartbeat")).await;
    let pid = fs::read_to_string(temp.0.join("leader"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let writer = tokio::spawn(async move { input.write_all(&vec![b'x'; 1024 * 1024]).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!writer.is_finished());
    let result = session.cancel_and_wait().await.unwrap();
    assert_eq!(result.completion, Completion::Cancelled);
    assert_reaped(pid);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), writer)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let size = fs::metadata(temp.0.join("heartbeat")).unwrap().len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(size, fs::metadata(temp.0.join("heartbeat")).unwrap().len());
}

#[tokio::test]
async fn session_drop_reaps_and_revokes_detached_input_for_pipes_and_pty() {
    for output in [OutputMode::Pipes, OutputMode::Pty { rows: 24, cols: 80 }] {
        let temp = Temp::new();
        let mut request = shell("echo $$ > leader; trap '' TERM; while :; do sleep 1; done");
        request.output = output;
        let mut session = local(&temp)
            .start_session(&context(CancellationToken::new()), request)
            .unwrap();
        let mut input = session.take_input().unwrap();
        wait_file(&temp.0.join("leader")).await;
        let pid = fs::read_to_string(temp.0.join("leader"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        drop(session);
        tokio::time::timeout(Duration::from_secs(3), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_reaped(pid);
        assert!(input.write_all(b"late").await.is_err());
    }
}

#[tokio::test]
async fn session_context_cancellation_and_deadlines_reap() {
    for mode in 0..3 {
        let temp = Temp::new();
        let token = CancellationToken::new();
        let mut ctx = context(token.clone());
        let mut request = shell("echo $$ > leader; trap '' TERM; while :; do sleep 1; done");
        if mode == 1 {
            ctx.deadline = Some(Instant::now() + Duration::from_millis(200));
        }
        if mode == 2 {
            request.timeout = Some(Duration::from_millis(200));
        }
        let session = local(&temp).start_session(&ctx, request).unwrap();
        wait_file(&temp.0.join("leader")).await;
        let pid = fs::read_to_string(temp.0.join("leader"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        if mode == 0 {
            token.cancel();
        }
        let result = session.wait().await.unwrap();
        assert_eq!(
            result.completion,
            if mode == 0 {
                Completion::Cancelled
            } else {
                Completion::TimedOut
            }
        );
        assert_reaped(pid);
    }
}

#[tokio::test]
async fn session_policy_rejects_escalation_environment_escape_and_pre_cancel() {
    let temp = Temp::new();
    let executor = Executor::new(Config::new(&temp.0)).unwrap();
    let ctx = context(CancellationToken::new());
    assert!(
        executor
            .start_session(&ctx, shell("touch forbidden"))
            .is_err()
    );
    for mode in 0..4 {
        let mut request = shell("touch forbidden");
        match mode {
            0 => request.access = AccessMode::ReadOnly,
            1 => request.network = Network::Deny,
            2 => request.cwd = "..".into(),
            _ => {
                request.env.insert("LD_PRELOAD".into(), "x".into());
            }
        }
        assert!(local(&temp).start_session(&ctx, request).is_err());
    }
    let token = CancellationToken::new();
    token.cancel();
    assert!(
        local(&temp)
            .start_session(&context(token), shell("touch forbidden"))
            .is_err()
    );
    let mut ctx = ctx;
    ctx.deadline = Some(Instant::now() - Duration::from_secs(1));
    assert!(
        local(&temp)
            .start_session(&ctx, shell("touch forbidden"))
            .is_err()
    );
    assert!(!temp.0.join("forbidden").exists());
}

#[tokio::test]
async fn session_backend_failure_notifies_without_unconfined_fallback() {
    let temp = Temp::new();
    let mut config = Config::new(&temp.0);
    config.backend = if cfg!(target_os = "linux") {
        Backend::Seatbelt
    } else {
        Backend::Bubblewrap
    };
    let executor = Executor::new(config).unwrap();
    let mut request = Request::new("/bin/sh");
    request.args = vec!["-c".into(), "touch forbidden".into()];
    let mut session = executor
        .start_session(&context(CancellationToken::new()), request)
        .unwrap();
    let mut input = session.take_input().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), session.ready())
            .await
            .unwrap()
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), session.next_output())
            .await
            .unwrap()
            .finished
    );
    assert!(input.write_all(b"x").await.is_err());
    assert!(matches!(
        session.wait().await,
        Err(adk_sandbox::Error::Unavailable(_))
    ));
    assert!(!temp.0.join("forbidden").exists());
}

#[tokio::test]
async fn session_abandoned_wait_reaps() {
    let temp = Temp::new();
    let session = local(&temp)
        .start_session(
            &context(CancellationToken::new()),
            shell("echo $$ > leader; trap '' TERM; while :; do sleep 1; done"),
        )
        .unwrap();
    let task = tokio::spawn(session.wait());
    wait_file(&temp.0.join("leader")).await;
    let pid = fs::read_to_string(temp.0.join("leader"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while unsafe { libc::kill(pid, 0) } == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_reaped(pid);
}

#[tokio::test]
async fn session_bundle_close_cancels_all_then_reaps_all() {
    let mut sessions = Vec::new();
    let mut workspaces = Vec::new();
    for _ in 0..3 {
        let temp = Temp::new();
        let session = local(&temp)
            .start_session(
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
        sessions.push((session, pid));
        workspaces.push(temp);
    }
    for (session, _) in &sessions {
        session.cancel();
    }
    for (session, pid) in sessions {
        assert_eq!(
            session.cancel_and_wait().await.unwrap().completion,
            Completion::Cancelled
        );
        assert_reaped(pid);
    }
}

#[tokio::test]
async fn session_ready_reports_permission_and_late_exec_failures() {
    use std::os::unix::fs::PermissionsExt;
    for output in [OutputMode::Pipes, OutputMode::Pty { rows: 24, cols: 80 }] {
        for executable in [false, true] {
            let temp = Temp::new();
            let program = temp.0.join("program");
            fs::write(&program, b"#!/adk-missing-interpreter\n").unwrap();
            fs::set_permissions(
                &program,
                fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
            )
            .unwrap();
            let mut request = shell("");
            request.program = program;
            request.output = output;
            let mut session = local(&temp)
                .start_session(&context(CancellationToken::new()), request)
                .unwrap();
            let error = tokio::time::timeout(Duration::from_secs(1), session.ready())
                .await
                .unwrap()
                .unwrap_err();
            assert!(error.to_string().contains("subprocess I/O"), "{error}");
            assert_eq!(
                session.ready().await.unwrap_err().to_string(),
                error.to_string()
            );
            let Err(adk_sandbox::Error::Io(error)) = session.wait().await else {
                panic!("original I/O error must be preserved")
            };
            assert_eq!(
                error.kind(),
                if executable {
                    std::io::ErrorKind::NotFound
                } else {
                    std::io::ErrorKind::PermissionDenied
                }
            );
        }
    }
}

#[tokio::test]
async fn session_ready_distinguishes_spawn_from_immediate_exit() {
    let temp = Temp::new();
    for code in [0, 7] {
        let mut session = local(&temp)
            .start_session(
                &context(CancellationToken::new()),
                shell(&format!("printf captured; exit {code}")),
            )
            .unwrap();
        session.ready().await.unwrap();
        while !session.is_finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        session.ready().await.unwrap();
        let result = session.wait().await.unwrap();
        assert_eq!(result.status.code(), Some(code));
        assert_eq!(result.stdout, b"captured");
    }
}

#[tokio::test]
async fn session_ready_is_retryable_and_does_not_wait_for_quiet_jobs() {
    use std::future::{Future, poll_fn};
    use std::task::Poll;
    let temp = Temp::new();
    let mut session = local(&temp)
        .start_session(&context(CancellationToken::new()), shell("sleep 30"))
        .unwrap();
    let mut ready = Box::pin(session.ready());
    poll_fn(|cx| {
        assert!(ready.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(ready);
    tokio::time::timeout(Duration::from_secs(1), session.ready())
        .await
        .unwrap()
        .unwrap();
    let output = session.poll();
    assert!(!output.finished && output.stdout.is_empty() && output.stderr.is_empty());
    assert_eq!(
        session.cancel_and_wait().await.unwrap().completion,
        Completion::Cancelled
    );
}

#[tokio::test]
async fn session_abandoned_readiness_reaps_spawned_child() {
    use std::future::{Future, poll_fn};
    use std::task::Poll;
    let temp = Temp::new();
    let mut session = local(&temp)
        .start_session(
            &context(CancellationToken::new()),
            shell("echo $$ > leader; trap '' TERM; while :; do sleep 1; done"),
        )
        .unwrap();
    let mut ready = Box::pin(async move { session.ready().await });
    poll_fn(|cx| {
        assert!(ready.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    wait_file(&temp.0.join("leader")).await;
    let pid = fs::read_to_string(temp.0.join("leader"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    drop(ready);
    tokio::time::timeout(Duration::from_secs(3), async {
        while unsafe { libc::kill(pid, 0) } == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_reaped(pid);
}
