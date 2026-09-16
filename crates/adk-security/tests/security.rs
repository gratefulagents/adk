use adk_core::*;
use adk_security::*;
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn policy(access: AccessMode) -> SecurityPolicy {
    SecurityPolicy {
        tools: ToolPolicy {
            access,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "shell".into(),
        description: String::new(),
        input_schema: schemars::schema_for!(serde_json::Value),
        read_only: true,
        requires_approval: false,
    }
}
fn call(command: &str) -> ToolCall {
    ToolCall {
        id: "call-1".into(),
        name: "shell".into(),
        arguments: json!({"command": command}),
    }
}
fn request(command: &str) -> CommandRequest {
    CommandRequest::from_call(call(command)).unwrap()
}
fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| (*s).into()).collect()
}

#[test]
fn composition_clamps_all_axes() {
    let mut a = policy(AccessMode::ReadOnly);
    a.tools.allowed_tools = Some(set(&["shell", "review"]));
    a.tools.allowed_mutating_tools = set(&["review"]);
    a.tools.timeout = Some(Duration::from_secs(3));
    a.tools.approval = ApprovalPolicy::All;
    a.approve_mutations = true;
    let mut b = policy(AccessMode::FullAccess);
    b.git_remote_writes = true;
    b.allow_network = true;
    b.tools.allowed_tools = Some(set(&["review", "write"]));
    b.tools.denied_tools = set(&["review"]);
    b.tools.timeout = Some(Duration::from_secs(20));
    let c = a.compose(&b);
    assert_eq!(c, b.compose(&a));
    assert_eq!(c.tools.access, AccessMode::ReadOnly);
    assert_eq!(c.tools.allowed_tools, Some(set(&["review"])));
    assert_eq!(c.tools.denied_tools, set(&["review"]));
    assert_eq!(c.tools.allowed_mutating_tools, set(&["review"]));
    assert_eq!(c.tools.timeout, Some(Duration::from_secs(3)));
    assert_eq!(c.tools.approval, ApprovalPolicy::All);
    assert!(!c.git_remote_writes && !c.allow_network && c.approve_mutations);
    assert!(a.for_child(&b).tools.allowed_mutating_tools.is_empty());
    assert_eq!(a.for_child(&b).tools.access, AccessMode::ReadOnly);
    assert_eq!(a.compose(&a), a);
    assert_eq!(normalize_access("fulll"), AccessMode::ReadOnly);
    assert_eq!(normalize_access(""), AccessMode::ReadOnly);
}

#[test]
fn composed_tool_decisions_never_escalate() {
    let modes = [
        AccessMode::ReadOnly,
        AccessMode::WorkspaceWrite,
        AccessMode::FullAccess,
    ];
    for left in modes {
        for right in modes {
            for mask in 0..256 {
                let mut a = policy(left);
                let mut b = policy(right);
                if mask & 1 != 0 {
                    a.tools.allowed_tools = Some(set(&[]));
                }
                if mask & 2 != 0 {
                    b.tools.denied_tools = set(&["shell"]);
                }
                if mask & 4 != 0 {
                    a.tools.allowed_mutating_tools = set(&["shell"]);
                }
                if mask & 8 != 0 {
                    b.tools.allowed_mutating_tools = set(&["shell"]);
                }
                if mask & 16 != 0 {
                    a.tools.approval = ApprovalPolicy::All;
                }
                if mask & 32 != 0 {
                    b.tools.approval = ApprovalPolicy::All;
                }
                let mut tool = definition();
                tool.read_only = mask & 64 != 0;
                tool.requires_approval = mask & 128 != 0;
                let composed = a.compose(&b).tools.decision(&tool);
                let decisions = [a.tools.decision(&tool), b.tools.decision(&tool)];
                if decisions.contains(&ToolDecision::Deny) {
                    assert_eq!(composed, ToolDecision::Deny);
                } else if decisions.contains(&ToolDecision::RequireApproval) {
                    assert_eq!(composed, ToolDecision::RequireApproval);
                } else {
                    assert_eq!(composed, ToolDecision::Allow);
                }
            }
        }
    }
}

#[test]
fn exact_mutation_exceptions_and_empty_allowlist() {
    let mut p = policy(AccessMode::ReadOnly);
    p.tools.allowed_mutating_tools = set(&["shell"]);
    let mut tool = definition();
    tool.read_only = false;
    assert_eq!(p.tools.decision(&tool), ToolDecision::Allow);
    tool.name = "shell_more".into();
    assert_eq!(p.tools.decision(&tool), ToolDecision::Deny);
    p.tools.allowed_tools = Some(BTreeSet::new());
    assert_eq!(p.tools.decision(&definition()), ToolDecision::Deny);
}

#[test]
fn upstream_command_corpus_and_obfuscations_fail_closed() {
    let corpus = include_str!("fixtures/cmd_obfuscation.txt");
    assert_eq!(corpus.lines().count(), 34);
    for (index, line) in corpus.lines().enumerate() {
        assert!(
            classify_command(line, AccessMode::ReadOnly, false).is_err(),
            "corpus case {index}"
        );
    }
    for line in [
        "r\\m -fr /",
        "r'm' -r -f /",
        "rm -rf /tmp/../etc",
        "rm -rf //",
        "rm -rf /./",
        "echo hi && rm -rf /",
        "sudo -u root rm -rf /",
        "env -S 'git push origin main'",
        "nice -n 2 git push origin main",
        "git -c alias.x='!rm -rf /' x",
        "git --exec-path=/tmp status",
        "git push origin HEAD:main",
        "git push origin +HEAD:feature",
        "git push origin",
        "git push",
        "git push --all",
        "git push origin :feature",
        "git push origin HEAD:refs/heads/master",
        "git push origin refs/tags/main",
        "git push origin HEAD:refs/heads/HEAD",
        "git push origin HEAD:refs/tags/safe",
        "echo $(rm -rf /)",
        "echo `git push`",
        "ls ${IFS}",
        "ls *.txt",
        "ls > >(sh)",
        "cat <<EOF\nhi\nEOF",
        "sh -c 'ls'",
        "ls & git push origin main",
        "A=x ls",
        "export PATH=/tmp; ls",
        "source script",
        "eval 'ls'",
        "find . -exec sh -c x ';'",
        "sed -n '1e rm -rf /' file",
        "sort --compress-program=sh file",
        "rg --pre sh file",
        "git diff --output=file",
        "git log --ext-diff",
        "cat 'unterminated",
        "cat \\",
        "ls \0",
        "ls\r rm -rf /",
        "echo hi > /etc/../etc/passwd",
        "tee /dev/sda",
        "unknown-program",
        "./ls",
        "/tmp/ls",
        "go test ./...",
        "python -c 'print(1)'",
        ":(){ :|:& };:",
    ] {
        assert!(
            classify_command(line, AccessMode::FullAccess, true).is_err(),
            "accepted unsafe shell case: {line:?}"
        );
    }
}

#[test]
fn safe_literal_commands_are_not_substring_matched() {
    for line in [
        "ls -la",
        "git status",
        "git --no-pager status --short",
        "grep -r 'rm -rf /' docs/",
        "echo '$(rm -rf /)'",
        "echo rm -rf /",
        "cat README.md | wc -l",
        "ls && pwd",
        "ls; cat README.md",
        "ls 2>/dev/null",
        "echo \\\"literal\\\"",
        "'ls' -l",
        "\\ls -a",
    ] {
        assert_eq!(
            classify_command(line, AccessMode::ReadOnly, false).unwrap(),
            CommandClass::ReadOnly,
            "{line}"
        );
    }
    for line in [
        "rm -rf ./build",
        "mkdir -p output",
        "echo ok > output",
        "git add file",
    ] {
        assert!(classify_command(line, AccessMode::ReadOnly, true).is_err());
        assert_eq!(
            classify_command(line, AccessMode::WorkspaceWrite, false).unwrap(),
            CommandClass::Mutating
        );
    }
    for access in [AccessMode::WorkspaceWrite, AccessMode::FullAccess] {
        assert!(classify_command("git push origin HEAD:feature", access, false).is_err());
        assert_eq!(
            classify_command("git push origin HEAD:feature", access, true).unwrap(),
            CommandClass::Mutating
        );
        assert!(classify_command("git push origin main", access, true).is_err());
    }
}

#[test]
fn secret_corpus_is_detected_without_leaking_into_errors() {
    let mut count = 0;
    for (index, line) in include_str!("fixtures/secret_obfuscation.txt")
        .lines()
        .enumerate()
    {
        let line = line
            .split_once(". ")
            .filter(|(prefix, _)| prefix.len() <= 4)
            .map_or(line, |(_, text)| text)
            .trim();
        if line.is_empty() {
            continue;
        }
        count += 1;
        let error = check_secrets(line).unwrap_err();
        assert_eq!(
            error.info.category,
            ErrorCategory::Guardrail,
            "secret case {index}"
        );
        assert!(
            !error.info.message.contains(line),
            "secret case {index} leaked"
        );
    }
    assert!(count >= 15);
    for text in [
        "hello world",
        "ghp_short",
        "the bearer of this letter",
        "sk-short",
        "passwordless login is enabled",
    ] {
        assert!(check_secrets(text).is_ok());
    }
    let token = format!("ghp_{}", "a".repeat(36));
    let hidden = token
        .chars()
        .map(|c| format!("{c}\u{200b}"))
        .collect::<String>();
    assert!(check_secrets(&hidden).is_err());
    assert!(check_secrets(&token.replace('_', "_\u{1b}[31m")).is_err());
    let wide = token
        .chars()
        .map(|c| char::from_u32(c as u32 + 0xfee0).unwrap())
        .collect::<String>();
    assert!(check_secrets(&wide).is_err());
}

struct Cancel(AtomicBool);
impl Cancellation for Cancel {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
struct TestHost {
    decision: ApprovalDecision,
    calls: Mutex<Vec<ApprovalRequest>>,
    cancel: Option<Arc<Cancel>>,
}
impl Host for TestHost {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        request: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(request);
            if let Some(cancel) = &self.cancel {
                cancel.0.store(true, Ordering::SeqCst);
            }
            Ok(self.decision)
        })
    }
}
fn context() -> Context {
    Context {
        run_id: "run-1".into(),
        cancellation: Arc::new(Cancel(AtomicBool::new(false))),
        deadline: None,
    }
}
fn host(decision: ApprovalDecision) -> TestHost {
    TestHost {
        decision,
        calls: Mutex::new(Vec::new()),
        cancel: None,
    }
}

#[tokio::test]
async fn deny_before_host_approval_and_no_model_grants() {
    let h = host(ApprovalDecision::Approve);
    let mut p = policy(AccessMode::FullAccess);
    p.tools.approval = ApprovalPolicy::All;
    p.tools.denied_tools = set(&["shell"]);
    assert!(
        p.authorize(&context(), &h, &definition(), request("ls"))
            .await
            .is_err()
    );
    p.tools.denied_tools.clear();
    assert!(
        p.authorize(&context(), &h, &definition(), request("rm -rf /"))
            .await
            .is_err()
    );
    assert!(
        p.authorize(
            &context(),
            &h,
            &definition(),
            request("git push origin feature")
        )
        .await
        .is_err()
    );
    let token = format!("echo ghp_{}", "a".repeat(36));
    assert!(
        p.authorize(&context(), &h, &definition(), request(&token))
            .await
            .is_err()
    );
    assert!(h.calls.lock().unwrap().is_empty());
    for value in [
        json!({"command":"ls", "approved":true}),
        json!({"command":"ls", "access":"full_access"}),
        json!({"command":5}),
        json!({}),
        json!("ls"),
    ] {
        let mut call = call("ls");
        call.arguments = value;
        assert!(CommandRequest::from_call(call).is_err());
    }
    let mut wrong = call("ls");
    wrong.name = "agent_shell".into();
    assert!(
        p.authorize(
            &context(),
            &h,
            &definition(),
            CommandRequest::from_call(wrong).unwrap()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn approval_is_bound_to_exact_immutable_call_and_policy() {
    let h = host(ApprovalDecision::Approve);
    let mut p = policy(AccessMode::WorkspaceWrite);
    p.approve_mutations = true;
    let expected = call("rm -rf ./build");
    let Authorization::Approved(approved) = p
        .authorize(
            &context(),
            &h,
            &definition(),
            CommandRequest::from_call(expected.clone()).unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("deferred");
    };
    assert_eq!(approved.call(), &expected);
    assert_eq!(approved.command(), "rm -rf ./build");
    assert_eq!(approved.run_id(), "run-1");
    assert_eq!(approved.policy(), &p);
    p.tools.access = AccessMode::FullAccess;
    assert_eq!(approved.policy().tools.access, AccessMode::WorkspaceWrite);
    assert_eq!(h.calls.lock().unwrap()[0].call, expected);
    assert_eq!(h.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn tool_owned_approval_denial_defer_and_cancellation() {
    let p = policy(AccessMode::ReadOnly);
    let mut tool = definition();
    tool.requires_approval = true;
    let denied = p
        .authorize(
            &context(),
            &host(ApprovalDecision::Deny),
            &tool,
            request("ls"),
        )
        .await;
    assert!(matches!(
        denied,
        Err(Error {
            info: ErrorInfo {
                category: ErrorCategory::ApprovalDenied,
                ..
            },
            ..
        })
    ));
    let deferred = p
        .authorize(
            &context(),
            &host(ApprovalDecision::Defer),
            &tool,
            request("ls"),
        )
        .await
        .unwrap();
    assert!(
        matches!(deferred, Authorization::Deferred(ApprovalRequest { call, .. }) if call.arguments == json!({"command":"ls"}))
    );
    let cancel = Arc::new(Cancel(AtomicBool::new(false)));
    let mut c = context();
    c.cancellation = cancel.clone();
    let h = TestHost {
        cancel: Some(cancel),
        ..host(ApprovalDecision::Approve)
    };
    assert!(p.authorize(&c, &h, &tool, request("ls")).await.is_err());
    let mut expired = context();
    expired.deadline = Some(Instant::now() - Duration::from_secs(1));
    assert!(
        p.authorize(&expired, &h, &tool, request("ls"))
            .await
            .is_err()
    );
    assert_eq!(h.calls.lock().unwrap().len(), 1);
}

struct HangingHost;
impl Host for HangingHost {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(std::future::pending())
    }
}
struct SoonCancelled;
impl Cancellation for SoonCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {
            tokio::time::sleep(Duration::from_millis(5)).await;
        })
    }
}

#[tokio::test]
async fn hanging_host_approval_is_interrupted() {
    let mut p = policy(AccessMode::ReadOnly);
    p.tools.approval = ApprovalPolicy::All;
    let mut c = context();
    c.deadline = Some(Instant::now() + Duration::from_millis(5));
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        p.authorize(&c, &HangingHost, &definition(), request("ls")),
    )
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(Error {
            info: ErrorInfo {
                category: ErrorCategory::DeadlineExceeded,
                ..
            },
            ..
        })
    ));
    c.deadline = None;
    c.cancellation = Arc::new(SoonCancelled);
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        p.authorize(&c, &HangingHost, &definition(), request("ls")),
    )
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(Error {
            info: ErrorInfo {
                category: ErrorCategory::Cancelled,
                ..
            },
            ..
        })
    ));
}

#[test]
fn parser_limits_and_git_helpers_fail_closed() {
    for line in [
        "ls &&",
        "ls |",
        "ls ||",
        "ls;;pwd",
        "rm -rf ../../",
        "git diff",
        "git show",
        "git log",
        "git diff -- --no-ext-diff --no-textconv",
        "git diff --no-ext-diff",
    ] {
        assert!(
            classify_command(line, AccessMode::FullAccess, true).is_err(),
            "{line}"
        );
    }
    assert!(classify_command(&"x".repeat(65_537), AccessMode::FullAccess, true).is_err());
    assert!(classify_command(&"ls;".repeat(2049), AccessMode::FullAccess, true).is_err());
    for line in [
        "git diff --no-ext-diff --no-textconv --stat",
        "git log --no-ext-diff --no-textconv --oneline",
        "ls;",
        "ls\n",
    ] {
        assert!(
            classify_command(line, AccessMode::ReadOnly, false).is_ok(),
            "{line}"
        );
    }
}
