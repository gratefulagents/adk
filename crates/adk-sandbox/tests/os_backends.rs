#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::{AccessMode, BoxFuture, Cancellation, Context};
use adk_sandbox::{Completion, Config, Executor, OutputMode, Request};
use std::{fs, net::TcpListener, path::PathBuf, sync::Arc, time::Duration};

struct Never;
impl Cancellation for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
fn context() -> Context {
    Context {
        run_id: "backend-test".into(),
        cancellation: Arc::new(Never),
        deadline: None,
    }
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("adk-os-test-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn shell(script: &str, access: AccessMode) -> Request {
    let mut req = Request::new("/bin/sh");
    req.args = vec!["-c".into(), script.into()];
    req.access = access;
    req.timeout = Some(Duration::from_secs(5));
    req
}
fn metadata(workspace: &std::path::Path) {
    for dir in [
        ".git/hooks",
        ".codex",
        ".claude",
        ".gemini",
        ".agents",
        ".aws",
        "data",
    ] {
        fs::create_dir_all(workspace.join(dir)).unwrap();
    }
    for path in [
        ".git/config",
        ".mcp.json",
        ".aws/credential",
        "data/original",
    ] {
        fs::write(workspace.join(path), b"original").unwrap();
    }
}

#[tokio::test]
async fn os_enforcement_required_in_ci() {
    let temp = Temp::new();
    let workspace = temp.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    metadata(&workspace);
    let mut config = Config::new(&workspace);
    config.term_grace = Duration::from_millis(50);
    let executor = Executor::new(config).unwrap();
    let ctx = context();
    let probe = executor
        .run(&ctx, shell("printf probe", AccessMode::ReadOnly))
        .await;
    let available = probe
        .as_ref()
        .is_ok_and(|r| r.status.success() && r.stdout == b"probe");
    if !available {
        let reason = format!("OS backend unavailable: {probe:?}");
        assert!(
            std::env::var_os("ADK_REQUIRE_SANDBOX").is_none(),
            "{reason}"
        );
        eprintln!(
            "SKIP OS containment tests: {reason}; set ADK_REQUIRE_SANDBOX=1 to require enforcement"
        );
        return;
    }
    let outside = temp.0.join("outside");
    fs::write(&outside, b"outside-private").unwrap();
    std::os::unix::fs::symlink(&outside, workspace.join("escape")).unwrap();
    let result = executor.run(&ctx, shell("if (printf bad > data/original) 2>/dev/null; then exit 11; fi; if (printf bad > new) 2>/dev/null; then exit 12; fi; if cat escape >/dev/null 2>&1; then exit 13; fi; if cat .aws/credential >/dev/null 2>&1; then exit 14; fi; test ! -e /run/secrets; printf readonly", AccessMode::ReadOnly)).await.unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"readonly");
    assert_eq!(result.completion, Completion::Exited);
    assert_eq!(fs::read(&outside).unwrap(), b"outside-private");
    assert_eq!(
        fs::read(workspace.join("data/original")).unwrap(),
        b"original"
    );

    // The absolute symlink may resolve to a new file in Linux's private /tmp.
    // The invariant is that the HOST target stays unchanged (checked below),
    // not that every write to a same-spelled private path fails.
    let result = executor.run(&ctx, shell("printf allowed > new; mkdir newdir; printf index > .git/index; if (printf bad > .git/config) 2>/dev/null; then exit 21; fi; if (printf bad > .mcp.json) 2>/dev/null; then exit 22; fi; if (printf bad > .git/hooks/evil) 2>/dev/null; then exit 23; fi; if mv .git movedgit 2>/dev/null; then exit 24; fi; (printf bad > escape) 2>/dev/null || :; if ln .git/config alias 2>/dev/null; then if (printf bad > alias) 2>/dev/null; then exit 26; fi; fi; printf writable", AccessMode::WorkspaceWrite)).await.unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"writable");
    assert_eq!(fs::read(workspace.join("new")).unwrap(), b"allowed");
    assert_eq!(fs::read(workspace.join(".git/index")).unwrap(), b"index");
    assert_eq!(
        fs::read(workspace.join(".git/config")).unwrap(),
        b"original"
    );
    assert_eq!(fs::read(&outside).unwrap(), b"outside-private");

    let mut terminal = shell("test -t 1 && printf terminal", AccessMode::ReadOnly);
    terminal.output = OutputMode::Pty { rows: 24, cols: 80 };
    let result = executor.run(&ctx, terminal).await.unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"terminal");

    for output in [OutputMode::Pipes, OutputMode::Pty { rows: 24, cols: 80 }] {
        let mut request = shell(
            "read line; if (printf bad > data/original) 2>/dev/null; then exit 31; fi; if cat .aws/credential >/dev/null 2>&1; then exit 32; fi; printf 'session:%s' \"$line\"",
            AccessMode::ReadOnly,
        );
        request.output = output;
        let mut session = executor.start_session(&ctx, request).unwrap();
        let mut input = session.take_input().unwrap();
        input.write_all(b"confined\n").await.unwrap();
        let result = session.wait().await.unwrap();
        assert!(result.status.success(), "session confinement: {result:?}");
        assert!(String::from_utf8_lossy(&result.stdout).contains("session:confined"));
        assert_eq!(
            fs::read(workspace.join("data/original")).unwrap(),
            b"original"
        );
        assert_eq!(fs::read(&outside).unwrap(), b"outside-private");
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    fs::write(
        workspace.join("address"),
        listener.local_addr().unwrap().to_string(),
    )
    .unwrap();
    let helper = workspace.join("network-helper");
    fs::copy(std::env::current_exe().unwrap(), &helper).unwrap();
    let mut request = Request::new(helper);
    request.args = vec![
        "--ignored".into(),
        "--exact".into(),
        "network_helper".into(),
        "--nocapture".into(),
    ];
    request.timeout = Some(Duration::from_secs(5));
    let result = executor.run(&ctx, request).await.unwrap();
    assert!(result.status.success(), "network deny test: {result:?}");
    assert!(String::from_utf8_lossy(&result.stdout).contains("network-denied"));

    let mut request = Request::new(workspace.join("network-helper"));
    request.args = vec![
        "--ignored".into(),
        "--exact".into(),
        "network_helper".into(),
        "--nocapture".into(),
    ];
    request.timeout = Some(Duration::from_secs(5));
    let result = executor
        .start_session(&ctx, request)
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "session network denial: {result:?}"
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("network-denied"));

    #[cfg(target_os = "macos")]
    {
        fs::write(workspace.join("host-pid"), std::process::id().to_string()).unwrap();
        let mut request = Request::new(workspace.join("network-helper"));
        request.args = vec![
            "--ignored".into(),
            "--exact".into(),
            "host_environment_helper".into(),
            "--nocapture".into(),
        ];
        request.timeout = Some(Duration::from_secs(5));
        let result = executor.run(&ctx, request).await.unwrap();
        assert!(
            result.status.success(),
            "host environment denial: {result:?}"
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("host-environment-denied"));
        for delay in [0, 1, 10] {
            let session = executor
                .start_session(&ctx, shell("sleep 30", AccessMode::ReadOnly))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(delay)).await;
            match session.cancel_and_wait().await {
                Err(adk_sandbox::Error::Cancelled) => {}
                Ok(result) => assert_eq!(result.completion, Completion::Cancelled),
                Err(error) => panic!("cancelled setup must not be a backend failure: {error}"),
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut req = shell(
            "setsid /bin/sh -c 'trap \"\" TERM; while :; do echo x >> data/heartbeat; sleep 0.02; done' & while [ ! -s data/heartbeat ]; do sleep 0.01; done; trap '' TERM; while :; do sleep 1; done",
            AccessMode::WorkspaceWrite,
        );
        req.timeout = Some(Duration::from_millis(300));
        let result = executor.run(&ctx, req).await.unwrap();
        assert_eq!(result.completion, Completion::TimedOut);
        let size = fs::metadata(workspace.join("data/heartbeat"))
            .unwrap()
            .len();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            size,
            fs::metadata(workspace.join("data/heartbeat"))
                .unwrap()
                .len(),
            "setsid descendant escaped PID namespace cleanup"
        );
    }
}

#[test]
#[ignore = "invoked inside sandbox by os_enforcement_required_in_ci"]
fn network_helper() {
    let address = fs::read_to_string("address").unwrap().parse().unwrap();
    assert!(std::net::TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_err());
    println!("network-denied");
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "invoked inside Seatbelt by os_enforcement_required_in_ci"]
fn host_environment_helper() {
    let pid: i32 = fs::read_to_string("host-pid").unwrap().parse().unwrap();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut bytes = vec![0u8; 64 * 1024];
    let mut size = bytes.len();
    // Query only this test's trusted parent, never print its environment.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    let has_path = bytes[..size.min(bytes.len())]
        .windows(b"\0PATH=".len())
        .any(|window| window == b"\0PATH=");
    assert_eq!(
        result, -1,
        "host process environment must be inaccessible (returned {size} bytes; contains PATH key: {has_path})"
    );
    assert!(matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM | libc::EACCES)
    ));
    println!("host-environment-denied");
}
