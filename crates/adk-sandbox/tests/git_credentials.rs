#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::{AccessMode, BoxFuture, Cancellation, Context};
use adk_sandbox::{Backend, Config, Error, Executor, Network, Request, ScratchDirectory};
use std::{fs, path::Path, process::Command, sync::Arc, time::Duration};

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
        run_id: "git-credentials".into(),
        cancellation: Arc::new(Never),
        deadline: None,
    }
}
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("/usr/bin/git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git fixture: {output:?}");
}
fn fixture() -> ScratchDirectory {
    let temp = ScratchDirectory::new().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    fs::write(root.join("tracked"), "original\n").unwrap();
    git(root, &["add", "tracked"]);
    git(
        root,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "initial",
        ],
    );
    fs::write(root.join("tracked"), "changed\n").unwrap();
    for dir in [".codex", ".claude", ".gemini", ".agents", "nested"] {
        fs::create_dir(root.join(dir)).unwrap();
    }
    fs::write(root.join(".mcp.json"), "{}").unwrap();
    git(&root.join("nested"), &["init", "-q"]);
    git(root, &["worktree", "add", "-q", "--detach", "linked"]);
    git(root, &["init", "-q", "--bare", "bare"]);
    for name in [
        ".git/config",
        "nested/.git/config",
        ".git/worktrees/linked/config.worktree",
        "bare/config",
    ] {
        let path = root.join(name);
        let mut content = fs::read_to_string(&path).unwrap_or_default();
        content.push_str(
            "\n[remote \"origin\"]\nurl = https://boundary-seven-token@example.invalid/repo\n",
        );
        fs::write(path, content).unwrap();
    }
    std::os::unix::fs::symlink("nested/.git/config", root.join("config-alias")).unwrap();
    temp
}
fn request(script: &str, access: AccessMode) -> Request {
    let mut req = Request::new("/bin/sh");
    req.args = vec!["-c".into(), script.into()];
    req.access = access;
    req.network = Network::Allow;
    req.hide_git_credentials = true;
    req.timeout = Some(Duration::from_secs(10));
    req
}

fn sandbox_config(root: &Path) -> Config {
    let mut config = Config::new(root);
    #[cfg(target_os = "macos")]
    config
        .runtime_roots
        .push(adk_sandbox::macos_developer_toolchain_root().unwrap());
    config
}

#[tokio::test]
async fn local_and_unsupported_metadata_fail_closed() {
    let temp = fixture();
    let mut config = sandbox_config(temp.path());
    config.backend = Backend::Local;
    let executor = Executor::new(config).unwrap();
    assert!(matches!(
        executor
            .run(&context(), request("exit 99", AccessMode::FullAccess))
            .await,
        Err(Error::Invalid(_))
    ));
    let executor = Executor::new(sandbox_config(temp.path())).unwrap();
    fs::write(
        temp.path().join("nested/.git/config"),
        "[include]\npath = ../../private-config\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("private-config"),
        "[remote \"origin\"]\nurl=secret\n",
    )
    .unwrap();
    assert!(matches!(
        executor
            .run(&context(), request("exit 99", AccessMode::ReadOnly))
            .await,
        Err(Error::Invalid(_))
    ));
    fs::write(temp.path().join("nested/.git/config"), "").unwrap();
    let outside = ScratchDirectory::new().unwrap();
    fs::create_dir(temp.path().join("external")).unwrap();
    fs::write(
        temp.path().join("external/.git"),
        format!("gitdir: {}\n", outside.path().display()),
    )
    .unwrap();
    assert!(matches!(
        executor
            .run(&context(), request("exit 99", AccessMode::ReadOnly))
            .await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn credential_read_masks_enforced_or_required_in_ci() {
    let temp = fixture();
    let executor = Executor::new(sandbox_config(temp.path())).unwrap();
    // Probe without masking so a broken mask never gets classified as unavailable.
    let mut probe = request("printf probe", AccessMode::ReadOnly);
    probe.hide_git_credentials = false;
    let result = executor.run(&context(), probe).await;
    if !result
        .as_ref()
        .is_ok_and(|r| r.status.success() && r.stdout == b"probe")
    {
        assert!(
            std::env::var_os("ADK_REQUIRE_SANDBOX").is_none(),
            "native enforcement required: {result:?}"
        );
        eprintln!(
            "SKIP Git credential enforcement: {result:?}; set ADK_REQUIRE_SANDBOX=1 to fail on skip"
        );
        return;
    }
    for access in [AccessMode::ReadOnly, AccessMode::WorkspaceWrite] {
        let script = r#"
set -eu
for file in .git/config nested/.git/config .git/worktrees/linked/config.worktree bare/config config-alias; do
    value=$(cat "$file" 2>/dev/null || :)
    test -z "$value"
done
for repo in . nested linked; do
    test -z "$(git -C "$repo" config --get remote.origin.url || :)"
    git -C "$repo" status --porcelain >/dev/null
    git -C "$repo" diff --no-ext-diff >/dev/null
done
git diff --no-ext-diff -- tracked | grep -q changed
printf masked
"#;
        let result = executor
            .run(&context(), request(script, access))
            .await
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert_eq!(result.stdout, b"masked");
        assert!(!String::from_utf8_lossy(&result.stderr).contains("boundary-seven-token"));
    }
    let script = r#"
set -eu
if (printf stolen > nested/.git/config) 2>/dev/null; then exit 21; fi
if mv nested renamed 2>/dev/null; then
    test -z "$(cat renamed/.git/config 2>/dev/null || :)"
fi
if ln .git/config hard-alias 2>/dev/null; then
    test -z "$(cat hard-alias 2>/dev/null || :)"
fi
printf ordinary > ordinary
printf index > .git/test-index
printf pinned
"#;
    let result = executor
        .run(&context(), request(script, AccessMode::WorkspaceWrite))
        .await
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"pinned");
    assert!(
        fs::read_to_string(temp.path().join(".git/config"))
            .unwrap()
            .contains("boundary-seven-token")
    );
    assert_eq!(fs::read(temp.path().join("ordinary")).unwrap(), b"ordinary");
    assert_eq!(
        fs::read(temp.path().join(".git/test-index")).unwrap(),
        b"index"
    );
}
