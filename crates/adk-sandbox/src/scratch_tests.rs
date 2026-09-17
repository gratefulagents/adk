use super::*;
use crate::{Completion, Executor, ScratchDirectory};
use adk_core::{BoxFuture, Cancellation, Context};
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
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
        run_id: "scratch-test".into(),
        cancellation: Arc::new(Cancel(token)),
        deadline: None,
    }
}
fn workspace() -> PrivateDir {
    let private = PrivateDir::new().unwrap();
    let work = private.0.join("home");
    for name in PROTECTED {
        let path = work.join(name);
        if matches!(*name, ".git/config" | ".mcp.json") {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"protected").unwrap();
        } else {
            fs::create_dir_all(path).unwrap();
        }
    }
    private
}

#[test]
fn scratch_is_private_owned_and_request_clones_retain_it() {
    let scratch = Arc::new(ScratchDirectory::new().unwrap());
    let other = ScratchDirectory::new().unwrap();
    let path = scratch.path().to_owned();
    assert_ne!(path, other.path());
    assert_eq!(
        path.parent().unwrap(),
        Path::new("/tmp").canonicalize().unwrap()
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    fs::write(path.join("capture"), b"pixels").unwrap();
    let mut request = Request::new("/bin/sh");
    assert!(request.scratch.is_empty());
    request.scratch.push(scratch.clone());
    let cloned = request.clone();
    assert!(Arc::ptr_eq(&request.scratch[0], &cloned.scratch[0]));
    drop(scratch);
    drop(request);
    assert!(path.join("capture").exists());
    drop(cloned);
    assert!(!path.exists());
}

#[test]
fn policy_rejects_non_workspace_write_and_overlapping_grants() {
    let work = workspace();
    let config = Config::new(work.0.join("home"));
    let scratch = Arc::new(ScratchDirectory::new().unwrap());
    let mut request = Request::new("/bin/sh");
    request.scratch.push(scratch.clone());
    for access in [AccessMode::ReadOnly, AccessMode::FullAccess] {
        request.access = access;
        assert!(matches!(
            policy::validate(&config, request.clone()),
            Err(Error::Invalid(_))
        ));
    }
    request.access = AccessMode::WorkspaceWrite;
    assert!(policy::validate(&config, request.clone()).is_ok());
    for path in [
        scratch.path().to_owned(),
        scratch.path().join("home"),
        scratch.path().parent().unwrap().to_owned(),
    ] {
        assert!(matches!(
            policy::validate(&Config::new(path), request.clone()),
            Err(Error::Invalid(_))
        ));
    }
    let mut local = config;
    local.backend = Backend::Local;
    request.access = AccessMode::FullAccess;
    request.network = Network::Allow;
    assert!(matches!(
        policy::validate(&local, request),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn native_plans_add_only_exact_scratch_grants() {
    let work = workspace();
    let config = Config::new(work.0.join("home"));
    let mut request = Request::new("/bin/sh");
    request.access = AccessMode::WorkspaceWrite;
    let baseline_args = bwrap_args(&config, &request, &work).unwrap();
    let baseline_profile = seatbelt_profile(&config, &request).unwrap();
    request.scratch = vec![
        Arc::new(ScratchDirectory::new().unwrap()),
        Arc::new(ScratchDirectory::new().unwrap()),
    ];
    let args = bwrap_args(&config, &request, &work).unwrap();
    let profile = seatbelt_profile(&config, &request).unwrap();
    let mut without_scratch = args.clone();
    for (i, scratch) in request.scratch.iter().enumerate() {
        let expected = [
            OsString::from("--bind"),
            scratch.path().into(),
            scratch.path().into(),
        ];
        assert_eq!(args.windows(3).filter(|w| *w == expected).count(), 1);
        let start = without_scratch
            .windows(3)
            .position(|w| w == expected)
            .unwrap();
        without_scratch.drain(start..start + 3);
        assert!(!profile.contains(scratch.path().to_str().unwrap()));
        assert!(profile.contains(&format!(
            "(allow file-read* (subpath (param \"SCRATCH{i}\")))"
        )));
        assert!(profile.contains(&format!("(allow file-write* (require-all (subpath (param \"SCRATCH{i}\")) (require-not (literal (param \"SCRATCH{i}\")))))")));
    }
    assert_eq!(without_scratch, baseline_args);
    let stripped: String = profile
        .lines()
        .filter(|line| !line.contains("SCRATCH"))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(stripped, baseline_profile);
    assert!(!args.windows(3).any(|w| w == ["--bind", "/tmp", "/tmp"]));

    #[cfg(target_os = "macos")]
    {
        let built = build(&config, &request).unwrap();
        let args: Vec<_> = built.command.as_std().get_args().collect();
        for (i, scratch) in request.scratch.iter().enumerate() {
            let parameter = format!("-DSCRATCH{i}={}", scratch.path().display());
            assert!(args.contains(&std::ffi::OsStr::new(&parameter)));
        }
    }

    request.access = AccessMode::ReadOnly;
    let readonly_args = bwrap_args(&config, &request, &work).unwrap();
    assert!(
        !seatbelt_profile(&config, &request)
            .unwrap()
            .contains("SCRATCH")
    );
    for scratch in &request.scratch {
        assert!(!readonly_args.contains(&scratch.path().into()));
    }
}

async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn assert_reaped(pid: i32) {
    assert_eq!(
        unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}
fn lifecycle_request(scratch: Arc<ScratchDirectory>) -> Request {
    let mut request = Request::new("/bin/sh");
    request.args = vec!["-c".into(), "trap 'printf term > \"$1/term\"' TERM; echo $$ > leader; printf ready > \"$1/ready\"; while :; do sleep 0.01; done".into(), "scratch-test".into(), scratch.path().to_string_lossy().into_owned()];
    request.timeout = Some(Duration::from_secs(5));
    request.scratch.push(scratch);
    request
}

#[tokio::test]
async fn session_retains_scratch_through_cancel_and_reap() {
    let work = workspace();
    let scratch = Arc::new(ScratchDirectory::new().unwrap());
    let path = scratch.path().to_owned();
    let weak = Arc::downgrade(&scratch);
    let mut config = Config::new(work.0.join("home"));
    config.backend = Backend::Local;
    config.term_grace = Duration::from_millis(200);
    let mut request = lifecycle_request(scratch);
    request.cwd = config.workspace.clone();
    // Exercise ownership even on hosts without native enforcement. The public
    // policy rejects Local scratch grants; only this supervisor unit test bypasses it.
    let session = crate::process::start_session(config, context(CancellationToken::new()), request);
    wait_file(&path.join("ready")).await;
    let pid = fs::read_to_string(work.0.join("home/leader"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(weak.strong_count(), 1);
    session.cancel();
    wait_file(&path.join("term")).await;
    assert!(weak.upgrade().is_some());
    assert!(path.exists());
    let result = session.wait().await.unwrap();
    assert_eq!(result.completion, Completion::Cancelled);
    assert_reaped(pid);
    assert!(weak.upgrade().is_none());
    assert!(!path.exists());
}

#[tokio::test]
async fn dropped_running_handle_retains_scratch_until_cleanup() {
    let work = workspace();
    let scratch = Arc::new(ScratchDirectory::new().unwrap());
    let path = scratch.path().to_owned();
    let weak = Arc::downgrade(&scratch);
    let mut config = Config::new(work.0.join("home"));
    config.backend = Backend::Local;
    config.term_grace = Duration::from_millis(200);
    let mut request = lifecycle_request(scratch);
    request.cwd = config.workspace.clone();
    let running = crate::process::start(config, context(CancellationToken::new()), request);
    wait_file(&path.join("ready")).await;
    let pid = fs::read_to_string(work.0.join("home/leader"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    drop(running);
    wait_file(&path.join("term")).await;
    assert!(weak.upgrade().is_some());
    assert!(path.exists());
    tokio::time::timeout(Duration::from_secs(5), async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_reaped(pid);
    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn native_scratch_enforcement_or_fail_closed() {
    let work = workspace();
    let scratch = Arc::new(ScratchDirectory::new().unwrap());
    let outside = ScratchDirectory::new().unwrap();
    fs::write(outside.path().join("secret"), b"host-private").unwrap();
    let executor = Executor::new(Config::new(work.0.join("home"))).unwrap();
    let mut request = Request::new("/bin/sh");
    request.access = AccessMode::WorkspaceWrite;
    request.timeout = Some(Duration::from_secs(5));
    request.scratch.push(scratch.clone());
    request.args = vec![
        "-c".into(),
        "printf enforced > \"$1/probe\"".into(),
        "probe".into(),
        scratch.path().to_string_lossy().into_owned(),
    ];
    let ctx = context(CancellationToken::new());
    let result = executor.run(&ctx, request.clone()).await;
    if !result.as_ref().is_ok_and(|r| r.status.success()) {
        assert!(
            !scratch.path().join("probe").exists(),
            "unenforced fallback ran payload"
        );
        assert!(
            std::env::var_os("ADK_REQUIRE_SANDBOX").is_none(),
            "native enforcement required: {result:?}"
        );
        eprintln!("SKIP native scratch containment (fail-closed verified): {result:?}");
        return;
    }
    assert_eq!(fs::read(scratch.path().join("probe")).unwrap(), b"enforced");
    std::os::unix::fs::symlink(outside.path(), scratch.path().join("escape")).unwrap();
    request.args = vec!["-c".into(), "set -e; printf pixels > \"$1/capture\"; test \"$(cat \"$1/capture\")\" = pixels; if cat \"$2/secret\" 2>/dev/null; then exit 11; fi; (printf bad > \"$2/secret\") 2>/dev/null || :; (printf bad > \"$1/escape/secret\") 2>/dev/null || :; if (printf bad > .git/config) 2>/dev/null; then exit 12; fi; if mv \"$1\" moved-scratch 2>/dev/null; then exit 13; fi; printf contained".into(), "scratch-test".into(), scratch.path().to_string_lossy().into_owned(), outside.path().to_string_lossy().into_owned()];
    let result = executor.run(&ctx, request.clone()).await.unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"contained");
    assert_eq!(
        fs::read(outside.path().join("secret")).unwrap(),
        b"host-private"
    );
    assert_eq!(
        fs::read(work.0.join("home/.git/config")).unwrap(),
        b"protected"
    );
    assert!(scratch.path().is_dir());
    request.access = AccessMode::ReadOnly;
    assert!(matches!(
        executor.start(&ctx, request),
        Err(Error::Invalid(_))
    ));
}
