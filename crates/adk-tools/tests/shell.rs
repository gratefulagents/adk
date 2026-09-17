#![cfg(unix)]
use adk_core::*;
use adk_tools::shell;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

fn setup() -> (
    tempfile::TempDir,
    shell::ShellBundle,
    ToolContext,
    Arc<adk_runtime::CancellationToken>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut sandbox = adk_sandbox::Config::new(dir.path());
    sandbox.backend = adk_sandbox::Backend::Local;
    sandbox.term_grace = Duration::from_millis(20);
    let bundle = shell::ShellBundle::new(shell::Config {
        sandbox,
        access: AccessMode::FullAccess,
        git_remote_writes: true,
        environment: BTreeMap::new(),
    })
    .unwrap();
    let cancel = Arc::new(adk_runtime::CancellationToken::new());
    let ctx = ToolContext {
        operation: Context {
            run_id: "shell-test".into(),
            cancellation: cancel.clone(),
            deadline: None,
        },
        work_dir: dir.path().into(),
        policy: ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        },
        idempotency_key: None,
    };
    (dir, bundle, ctx, cancel)
}
fn tool(bundle: &shell::ShellBundle, name: &str) -> Arc<dyn Tool> {
    bundle
        .tools()
        .into_iter()
        .find(|t| t.definition().name == name)
        .unwrap()
}
async fn invoke(tool: &dyn Tool, ctx: &ToolContext, args: Value) -> ToolOutput {
    tool.execute(
        ctx,
        ToolCall {
            id: "test".into(),
            name: tool.definition().name.clone(),
            arguments: args,
        },
    )
    .await
    .unwrap()
}
fn text(out: &ToolOutput) -> &str {
    match &out.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("text"),
    }
}
async fn call(
    bundle: &shell::ShellBundle,
    name: &str,
    ctx: &ToolContext,
    args: Value,
) -> ToolOutput {
    invoke(tool(bundle, name).as_ref(), ctx, args).await
}
async fn start(bundle: &shell::ShellBundle, ctx: &ToolContext, command: &str) -> String {
    let out = call(bundle, "BashStart", ctx, json!({"command":command})).await;
    assert!(!out.is_error, "{}", text(&out));
    text(&out)
        .strip_prefix("started background bash job ")
        .unwrap()
        .into()
}
async fn poll(bundle: &shell::ShellBundle, ctx: &ToolContext, args: Value) -> Value {
    let out = call(bundle, "BashPoll", ctx, args).await;
    assert!(!out.is_error, "{}", text(&out));
    serde_json::from_str(text(&out)).unwrap()
}
#[tokio::test]
async fn bash_combines_streams_empty_exit_timeout_and_stdin_eof() {
    let (_dir, bundle, ctx, _) = setup();
    for (command, expected) in [
        ("printf one; printf two >&2; printf three", "onetwothree"),
        ("true", "(no output)"),
        ("printf failed; exit 7", "failed\nExit code: 7"),
        ("cat; printf eof", "eof"),
    ] {
        let out = call(&bundle, "Bash", &ctx, json!({"command":command})).await;
        assert!(!out.is_error, "{}", text(&out));
        assert_eq!(text(&out), expected);
    }
    let out = call(
        &bundle,
        "Bash",
        &ctx,
        json!({"command":"printf before; sleep 5","timeout":20}),
    )
    .await;
    assert!(!out.is_error);
    assert!(text(&out).contains("before\n[command timed out]"));
    bundle.close().await;
}
#[tokio::test]
async fn async_incremental_nonincremental_and_kill_contract() {
    let (_dir, bundle, ctx, _) = setup();
    let id = start(&bundle, &ctx, "printf first; sleep .1; printf last; exit 3").await;
    let snap = poll(&bundle, &ctx, json!({"id":id,"wait_ms":2000})).await;
    assert_eq!(snap["status"], "exited");
    assert_eq!(snap["exit_code"], 3);
    assert_eq!(snap["output"], "firstlast");
    assert!(snap["started_at"].as_str().unwrap().ends_with('Z'));
    assert!(snap.get("ended_at").is_some());
    assert_eq!(snap["timeout_ms"], 600_000);
    assert_eq!(
        poll(&bundle, &ctx, json!({"id":id})).await["output"],
        "(no new output since last poll)"
    );
    assert_eq!(
        poll(&bundle, &ctx, json!({"id":id,"incremental":false})).await["output"],
        "firstlast"
    );
    let id = start(&bundle, &ctx, "sleep 30").await;
    let killed = call(&bundle, "BashKill", &ctx, json!({"id":id})).await;
    let snap: Value = serde_json::from_str(text(&killed)).unwrap();
    assert_eq!(snap["status"], "exited");
    assert_eq!(snap["running"], false);
    assert_eq!(snap["timed_out"], false);
    bundle.close().await;
}
#[tokio::test]
async fn output_cap_keeps_head_tail_without_killing_command() {
    let (dir, bundle, ctx, _) = setup();
    let out = call(&bundle,"Bash",&ctx,json!({"command":"printf HEAD; head -c 400000 /dev/zero | tr '\\0' x; printf TAIL; touch finished"})).await;
    assert!(!out.is_error, "{}", text(&out));
    assert!(text(&out).starts_with("HEAD"));
    assert!(text(&out).contains("TAIL"));
    assert!(text(&out).contains("process was NOT terminated"));
    assert!(text(&out).len() < 104_000);
    assert!(dir.path().join("finished").exists());
    let id = start(
        &bundle,
        &ctx,
        "head -c 400000 /dev/zero | tr '\\0' x; printf TAIL",
    )
    .await;
    let snap = poll(&bundle, &ctx, json!({"id":id,"wait_ms":2000})).await;
    assert!(snap["output"].as_str().unwrap().ends_with("TAIL"));
    assert!(snap.get("note").is_some());
    assert!(
        poll(&bundle, &ctx, json!({"id":id,"incremental":false})).await["output"]
            .as_str()
            .unwrap()
            .contains("output truncated")
    );
    bundle.close().await;
}
#[tokio::test]
async fn bundle_close_and_owner_cancellation_reap_jobs() {
    let (_dir, bundle, ctx, cancel) = setup();
    let id = start(&bundle, &ctx, "sleep 30").await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), bundle.close())
        .await
        .unwrap();
    let mut active = ctx;
    active.operation.cancellation = Arc::new(adk_runtime::CancellationToken::new());
    assert_eq!(
        poll(&bundle, &active, json!({"id":id})).await["running"],
        false
    );
    let out = call(&bundle, "BashStart", &active, json!({"command":"true"})).await;
    assert!(out.is_error && text(&out).contains("closed"));
}
#[tokio::test]
async fn bundle_drop_cancels_even_if_tool_handles_survive() {
    let (_dir, bundle, ctx, _) = setup();
    let id = start(&bundle, &ctx, "sleep 30").await;
    let poller = tool(&bundle, "BashPoll");
    drop(bundle);
    let out = invoke(poller.as_ref(), &ctx, json!({"id":id,"wait_ms":2000})).await;
    let snap: Value = serde_json::from_str(text(&out)).unwrap();
    assert_eq!(snap["running"], false);
}
#[tokio::test]
async fn restrictive_access_never_falls_back_to_local() {
    let (_dir, bundle, mut ctx, _) = setup();
    // Workspace confinement is not waived by read-only mutation exceptions.
    ctx.policy.access = AccessMode::WorkspaceWrite;
    ctx.policy.allowed_mutating_tools.insert("Bash".into());
    let out = call(&bundle, "Bash", &ctx, json!({"command":"printf safe"})).await;
    assert!(out.is_error && text(&out).contains("local requires explicit FullAccess"));
    let out = call(
        &bundle,
        "Bash",
        &ctx,
        json!({"command":"echo $(touch forbidden)"}),
    )
    .await;
    assert!(out.is_error && text(&out).contains("authorized statically"));
    bundle.close().await;
}
#[tokio::test]
async fn invalid_input_and_unknown_jobs() {
    let (_dir, bundle, ctx, _) = setup();
    for (name, args, expected) in [
        ("Bash", json!({}), "command is required"),
        ("Bash", json!({"command":1}), "Invalid input"),
        ("BashPoll", json!({"id":"x"}), "unknown job id: x"),
        ("BashKill", json!({"id":"x"}), "unknown job id: x"),
    ] {
        let out = call(&bundle, name, &ctx, args).await;
        assert!(out.is_error && text(&out).contains(expected));
    }
    bundle.close().await;
}
#[test]
fn dynamic_schema_uses_only_trusted_environment() {
    let limits = shell::Limits::from_environment(&BTreeMap::from([
        ("GRATEFUL_BASH_DEFAULT_TIMEOUT_MS".into(), "999".into()),
        ("GRATEFUL_BASH_MAX_TIMEOUT_MS".into(), "500".into()),
        ("GRATEFUL_BASH_MAX_OUTPUT_BYTES".into(), "999999999".into()),
    ]));
    assert_eq!(limits.default_timeout_ms, 1000);
    assert_eq!(limits.max_timeout_ms, 1000);
    assert_eq!(limits.output_bytes, 10 * 1024 * 1024);
    for mode in [
        AccessMode::ReadOnly,
        AccessMode::WorkspaceWrite,
        AccessMode::FullAccess,
    ] {
        let def = shell::definition("Bash", mode, &limits).unwrap();
        assert_eq!(def.read_only, mode == AccessMode::ReadOnly);
        assert!(!def.requires_approval);
        let schema = serde_json::to_value(def.input_schema).unwrap();
        assert_eq!(
            schema["properties"]["timeout"]["description"],
            "Timeout in milliseconds (max 1000, default 1000)"
        );
    }
}

#[test]
fn sdk_policy_and_schema_fixture() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tools/shell.json")).unwrap();
    for case in fixture["policy"].as_array().unwrap() {
        let mode = match case["mode"].as_str().unwrap() {
            "read-only" => AccessMode::ReadOnly,
            "workspace-write" => AccessMode::WorkspaceWrite,
            "danger-full-access" => AccessMode::FullAccess,
            other => panic!("{other}"),
        };
        let command = case["command"].as_str().unwrap();
        let result = shell::command_blocked(mode, true, command, false);
        assert_eq!(
            result.is_some(),
            case["blocked"].as_bool().unwrap(),
            "{mode:?}: {command}: {result:?}"
        );
        if let Some(reason) = result {
            assert_eq!(
                reason,
                case["reason"].as_str().unwrap(),
                "{mode:?}: {command}"
            );
        }
    }
    for case in fixture["schemas"].as_array().unwrap() {
        let env: BTreeMap<String, String> =
            serde_json::from_value(case["environment"].clone()).unwrap();
        let limits = shell::Limits::from_environment(&env);
        for (name, description) in [
            ("Bash", "bash_description"),
            ("BashStart", "start_description"),
        ] {
            let def = shell::definition(name, AccessMode::FullAccess, &limits).unwrap();
            assert_eq!(serde_json::to_value(def.input_schema).unwrap(), case[name]);
            assert_eq!(def.description, case[description]);
        }
    }
}

#[tokio::test]
async fn aborted_synchronous_call_reaps_owned_process() {
    let (dir, bundle, ctx, _) = setup();
    let bash = tool(&bundle, "Bash");
    let active = Arc::new(ctx);
    let runner = active.clone();
    let task = tokio::spawn(async move {
        invoke(
            bash.as_ref(),
            &runner,
            json!({"command":"echo $$ > pid; exec sleep 30"}),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !dir.path().join("pid").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let pid = std::fs::read_to_string(dir.path().join("pid"))
        .unwrap()
        .trim()
        .to_owned();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_secs(3), bundle.close())
        .await
        .unwrap();
    let out = call(&bundle, "BashPoll", &active, json!({"id":"sync-1"})).await;
    assert!(out.is_error && text(&out).contains("unknown job id"));
    // An independently configured Local executor checks the reaped PID without
    // allowing any restricted tool variant to spawn on the host.
    let mut config = adk_sandbox::Config::new(dir.path());
    config.backend = adk_sandbox::Backend::Local;
    let executor = adk_sandbox::Executor::new(config).unwrap();
    let mut request = adk_sandbox::Request::new("/bin/bash");
    request.args = vec![
        "--noprofile".into(),
        "--norc".into(),
        "-c".into(),
        format!("kill -0 {pid} 2>/dev/null"),
    ];
    request.access = AccessMode::FullAccess;
    request.network = adk_sandbox::Network::Allow;
    assert!(
        !executor
            .start_session(&active.operation, request)
            .unwrap()
            .wait()
            .await
            .unwrap()
            .status
            .success()
    );
}

#[tokio::test]
async fn async_timeout_and_backend_unavailability_are_reported() {
    let (dir, bundle, ctx, _) = setup();
    let out = call(
        &bundle,
        "BashStart",
        &ctx,
        json!({"command":"printf begun; sleep 30","timeout":25}),
    )
    .await;
    let id = text(&out)
        .strip_prefix("started background bash job ")
        .unwrap();
    let snap = poll(&bundle, &ctx, json!({"id":id,"wait_ms":2000})).await;
    assert_eq!(snap["status"], "timed_out");
    assert_eq!(snap["exit_code"], -1);
    assert_eq!(snap["timed_out"], true);
    bundle.close().await;
    let mut config = adk_sandbox::Config::new(dir.path());
    config.backend = adk_sandbox::Backend::Local;
    let restricted = shell::ShellBundle::new(shell::Config {
        sandbox: config,
        access: AccessMode::WorkspaceWrite,
        git_remote_writes: true,
        environment: BTreeMap::new(),
    })
    .unwrap();
    let out = call(
        &restricted,
        "BashStart",
        &ctx,
        json!({"command":"touch must-not-exist"}),
    )
    .await;
    assert!(out.is_error && text(&out).contains("local requires explicit FullAccess"));
    assert!(!dir.path().join("must-not-exist").exists());
    restricted.close().await;
}

#[tokio::test]
async fn caller_cancellation_stops_async_job_without_bundle_close() {
    let (_dir, bundle, mut ctx, cancel) = setup();
    let id = start(&bundle, &ctx, "printf ready; sleep 30").await;
    cancel.cancel();
    ctx.operation.cancellation = Arc::new(adk_runtime::CancellationToken::new());
    let snap = poll(&bundle, &ctx, json!({"id":id,"wait_ms":2000})).await;
    assert_eq!(snap["running"], false);
    assert_eq!(snap["status"], "exited");
    bundle.close().await;
}

#[tokio::test]
async fn bash_start_reports_spawn_not_immediate_exit_status() {
    let (_dir, bundle, ctx, _) = setup();
    for code in [0, 7] {
        let id = start(&bundle, &ctx, &format!("exit {code}")).await;
        let snap = poll(&bundle, &ctx, json!({"id":id,"wait_ms":2000})).await;
        assert_eq!(snap["exit_code"], code);
        assert_eq!(snap["status"], "exited");
        assert_eq!(snap["output"], "(no new output since last poll)");
    }
    bundle.close().await;
}

#[tokio::test]
async fn bash_start_does_not_wait_for_quiet_job_output_or_exit() {
    let (_dir, bundle, ctx, _) = setup();
    let id = tokio::time::timeout(Duration::from_secs(1), start(&bundle, &ctx, "sleep 30"))
        .await
        .unwrap();
    let snap = poll(&bundle, &ctx, json!({"id":id})).await;
    assert_eq!(snap["status"], "running");
    assert_eq!(snap["output"], "(no new output since last poll)");
    bundle.close().await;
}

#[tokio::test]
async fn bash_start_reports_backend_setup_failure_without_job_id() {
    let (dir, _bundle, ctx, _) = setup();
    let mut sandbox = adk_sandbox::Config::new(dir.path());
    sandbox.backend = if cfg!(target_os = "linux") {
        adk_sandbox::Backend::Seatbelt
    } else {
        adk_sandbox::Backend::Bubblewrap
    };
    let bundle = shell::ShellBundle::new(shell::Config {
        sandbox,
        access: AccessMode::FullAccess,
        git_remote_writes: true,
        environment: BTreeMap::new(),
    })
    .unwrap();
    let out = call(
        &bundle,
        "BashStart",
        &ctx,
        json!({"command":"touch forbidden"}),
    )
    .await;
    assert!(out.is_error, "{}", text(&out));
    assert!(!text(&out).contains("started background bash job"));
    assert!(!dir.path().join("forbidden").exists());
    bundle.close().await;
}

#[tokio::test]
async fn synchronous_secret_output_is_blocked() {
    let (_dir, bundle, ctx, _) = setup();
    let out = call(
        &bundle,
        "Bash",
        &ctx,
        json!({"command":"printf 'AKIA%s' ABCDEFGHIJKLMNOP"}),
    )
    .await;
    assert!(
        out.is_error && text(&out).contains("payload blocked"),
        "{}",
        text(&out)
    );
    assert!(!text(&out).contains("ABCDEFGHIJKLMNOP"));
    bundle.close().await;
}

#[tokio::test]
async fn asynchronous_secret_split_across_polls_is_sticky_and_cancels() {
    for prefix in ["", "printf '%150000s\\n' x; "] {
        let (dir, bundle, ctx, _) = setup();
        let id = start(&bundle, &ctx, &format!("{prefix}printf AKIA; while [ ! -e release ]; do sleep .01; done; printf ABCDEFGHIJKLMNOP; sleep 30")).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snap = poll(&bundle, &ctx, json!({"id":id})).await;
                if snap["output"].as_str().unwrap().contains("AKIA") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        std::fs::write(dir.path().join("release"), b"go").unwrap();
        for incremental in [true, false, true] {
            let out = call(
                &bundle,
                "BashPoll",
                &ctx,
                json!({"id":id,"wait_ms":2000,"incremental":incremental}),
            )
            .await;
            assert!(
                out.is_error && text(&out).contains("payload blocked"),
                "{}",
                text(&out)
            );
            assert!(!text(&out).contains("ABCDEFGHIJKLMNOP"));
        }
        tokio::time::timeout(Duration::from_secs(2), bundle.close())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn terminal_secret_output_is_blocked_and_stays_blocked() {
    let (_dir, bundle, ctx, _) = setup();
    let out = call(
        &bundle,
        "Terminal",
        &ctx,
        json!({"op":"start","wait_ms":10}),
    )
    .await;
    assert!(!out.is_error, "{}", text(&out));
    let snap: Value = serde_json::from_str(text(&out)).unwrap();
    let id = snap["session_id"].as_str().unwrap();
    let out = call(&bundle, "Terminal", &ctx, json!({"op":"send","session_id":id,"keystrokes":"printf AKIA; sleep .1; printf ABCDEFGHIJKLMNOP; sleep 30\n","wait_ms":2000})).await;
    assert!(
        out.is_error && text(&out).contains("payload blocked"),
        "{}",
        text(&out)
    );
    assert!(!text(&out).contains("ABCDEFGHIJKLMNOP"));
    let out = call(
        &bundle,
        "Terminal",
        &ctx,
        json!({"op":"read","session_id":id}),
    )
    .await;
    assert!(out.is_error && text(&out).contains("payload blocked"));
    bundle.close().await;
}

#[tokio::test]
async fn readonly_adapter_preserves_schema_and_uses_enforcing_backend() {
    let (dir, bundle, mut ctx, _) = setup();
    let bash = tool(&bundle, "Bash");
    let readonly = bash.for_access(AccessMode::ReadOnly).unwrap();
    assert!(readonly.definition().read_only);
    assert_eq!(
        readonly.definition().input_schema,
        bash.definition().input_schema
    );
    assert_eq!(
        readonly.definition().description,
        bash.definition().description
    );
    assert!(
        tool(&bundle, "BashStart")
            .for_access(AccessMode::ReadOnly)
            .is_none()
    );
    ctx.policy.access = AccessMode::ReadOnly;
    let out = invoke(
        readonly.as_ref(),
        &ctx,
        json!({"command":"touch forbidden"}),
    )
    .await;
    assert!(
        out.is_error || text(&out).contains("Exit code:"),
        "{}",
        text(&out)
    );
    assert!(!dir.path().join("forbidden").exists());
    let out = invoke(readonly.as_ref(), &ctx, json!({"command":"printf safe"})).await;
    assert!(
        !text(&out).contains("local requires explicit FullAccess"),
        "{}",
        text(&out)
    );
    if out.is_error {
        assert!(
            text(&out).contains("sandbox") || text(&out).contains("subprocess"),
            "{}",
            text(&out)
        );
    }
    bundle.close().await;
}

#[tokio::test]
async fn combined_output_does_not_shift_command_line_numbers() {
    let (_dir, bundle, ctx, _) = setup();
    let output = call(
        &bundle,
        "Bash",
        &ctx,
        json!({"command":"printf '%s\\n' \"$LINENO\"\nprintf '%s\\n' \"$LINENO\" >&2"}),
    )
    .await;
    assert!(!output.is_error, "{output:?}");
    assert_eq!(text(&output), "1\n2\n");
    bundle.close().await;
}

#[tokio::test]
async fn prepared_exact_bash_grant_preserves_but_never_raises_host_access() {
    let (dir, initial, mut ctx, _) = setup();
    initial.close().await;
    for access in [
        AccessMode::FullAccess,
        AccessMode::WorkspaceWrite,
        AccessMode::ReadOnly,
    ] {
        let mut sandbox = adk_sandbox::Config::new(dir.path());
        sandbox.backend = adk_sandbox::Backend::Local;
        let owner = shell::ShellBundle::new(shell::Config {
            sandbox,
            access,
            git_remote_writes: true,
            environment: BTreeMap::new(),
        })
        .unwrap();
        let registry = adk_tools::Registry::build(
            &adk_tools::Config {
                access,
                features: adk_tools::Features::Strict(["Bash".into()].into()),
                allowed_names: Some(["Bash".into()].into()),
                ..Default::default()
            },
            [tool(&owner, "Bash")],
        )
        .unwrap();
        let prepared = registry.prepare(ToolPolicy {
            access: AccessMode::ReadOnly,
            allowed_mutating_tools: ["Bash".into()].into(),
            ..Default::default()
        });
        assert_eq!(prepared.tools.len(), 1);
        assert_eq!(
            prepared.tools[0].definition().read_only,
            access == AccessMode::ReadOnly
        );
        ctx.policy = prepared.policy;
        let output = invoke(
            prepared.tools[0].as_ref(),
            &ctx,
            json!({"command":"touch granted"}),
        )
        .await;
        if access == AccessMode::FullAccess {
            assert!(!output.is_error, "{output:?}");
            assert!(dir.path().join("granted").exists());
            std::fs::remove_file(dir.path().join("granted")).unwrap();
        } else {
            assert!(output.is_error, "{output:?}");
            assert!(!dir.path().join("granted").exists());
            if access == AccessMode::WorkspaceWrite {
                // Retain workspace-write, never unrestricted Local execution.
                assert!(text(&output).contains("local requires explicit FullAccess"));
            }
        }
        owner.close().await;
    }
}
