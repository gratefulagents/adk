#![cfg(feature = "host")]

use adk::{
    builder::{ConfigSource as BuilderConfigSource, ModeSpec},
    core::{Context, ErrorCategory},
    host::{ConfigSource, PermissionMode, build_mode_directive, fileconfig::FileConfigSource},
    runtime::CancellationToken,
};
use std::{fs, sync::Arc};

fn context() -> Context {
    Context {
        run_id: "fileconfig-test".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}

fn write(root: &std::path::Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[tokio::test]
async fn no_active_mode_is_not_implicitly_chat() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "modes/bad.yaml", "broken: [");
    let source = FileConfigSource::new(dir.path()).with_active_mode("  ");
    let ctx = context();
    assert_eq!(source.root(), dir.path());
    assert_eq!(source.mode_dir(), dir.path().join("modes"));
    assert_eq!(source.agent_dir(), dir.path().join("agents"));
    assert_eq!(source.active_mode(), None);
    assert!(source.mode_snapshot(&ctx).await.unwrap().is_none());
    assert_eq!(source.mode_directive(&ctx).await.unwrap(), "");
    assert_eq!(
        source.permission_mode(&ctx).await.unwrap(),
        PermissionMode::WorkspaceWrite
    );
    assert!(source.guardrail_rules(&ctx).await.unwrap().is_empty());
    assert!(source.handoff_history(&ctx).await.unwrap().items.is_empty());
}

#[tokio::test]
async fn direct_lookup_prioritizes_yaml_and_bypasses_unrelated_errors() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "modes/review.yaml",
        "name: actual\ntoolAccess: analysis\ninstructions: Inspect\n",
    );
    write(dir.path(), "modes/review.yml", "name: other\n");
    write(dir.path(), "modes/bad.yaml", "broken: [");
    write(dir.path(), "agents/bad.md", "---\nunknown: yes\n---\nBody");
    let source = FileConfigSource::new(dir.path()).with_active_mode(" review ");
    let ctx = context();
    assert_eq!(source.active_mode(), Some("review"));
    assert_eq!(
        source.mode_snapshot(&ctx).await.unwrap().unwrap().name,
        "actual"
    );
    assert_eq!(
        source.permission_mode(&ctx).await.unwrap(),
        PermissionMode::ReadOnly
    );
    assert!(
        source
            .mode_directive(&ctx)
            .await
            .unwrap()
            .ends_with("Inspect")
    );
    assert!(source.list_modes(&ctx).is_err());
    assert!(source.load_files(&ctx).is_err());
    assert!(BuilderConfigSource::load(&source, &ctx).await.is_err());
    write(dir.path(), "modes/review.yaml", "toolAccess: typo");
    assert!(source.mode_snapshot(&ctx).await.is_err());
    fs::remove_file(dir.path().join("modes/review.yaml")).unwrap();
    assert_eq!(source.get_mode(&ctx, "review").unwrap().name, "other");
}

#[test]
fn fallback_matches_declared_and_display_names_and_builtin_overrides() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "modes/custom.YAML",
        "name: Chat\ndisplayName: Custom Chat\n",
    );
    write(
        dir.path(),
        "modes/reviewer.yml",
        "name: critic\ndisplayName: Code Review\n",
    );
    let source = FileConfigSource::new(dir.path());
    let ctx = context();
    assert_eq!(source.get_mode(&ctx, " cRiTiC ").unwrap().name, "critic");
    assert_eq!(source.get_mode(&ctx, "code review").unwrap().name, "critic");
    assert_eq!(
        source.get_mode(&ctx, "CHAT").unwrap().display_name,
        "Custom Chat"
    );
    assert_eq!(
        source.get_mode(&ctx, "PLAN").unwrap().tool_access,
        "read-only"
    );
    assert_eq!(source.list_modes(&ctx).unwrap().len(), 3);
    assert!(source.get_mode(&ctx, "missing").is_err());
    for name in [
        "",
        " ",
        ".",
        "..",
        "../plan",
        "nested/plan",
        "nested\\plan",
        "C:plan",
    ] {
        assert_eq!(
            source.get_mode(&ctx, name).unwrap_err().info.category,
            ErrorCategory::InvalidInput
        );
    }
}

#[tokio::test]
async fn mode_and_role_reads_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "agents/bad.md", "---\nname: critic\n");
    let source = FileConfigSource::new(dir.path()).with_active_mode("plan");
    let ctx = context();
    assert_eq!(source.list_modes(&ctx).unwrap().len(), 2);
    assert!(source.mode_snapshot(&ctx).await.unwrap().is_some());
    assert!(source.role_catalog(&ctx).await.is_err());
    assert!(source.load_files(&ctx).is_err());
    write(dir.path(), "agents/bad.md", "Valid instructions");
    write(dir.path(), "modes/bad.yaml", "unknown: value");
    assert_eq!(
        source.role_catalog(&ctx).await.unwrap()[0].instructions,
        "Valid instructions"
    );
    assert!(source.list_modes(&ctx).is_err());
    assert!(source.load_files(&ctx).is_err());
}

#[test]
fn standalone_role_catalog_uses_requested_directory_and_strict_policy() {
    use adk::host::fileconfig::load_role_catalog;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "agents/bad.md", "---\nunknown: yes\n---\nBad");
    write(dir.path(), "custom/reviewer.md", "Review carefully.");
    let ctx = context();
    let roles = load_role_catalog(&ctx, dir.path().join("custom")).unwrap();
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "reviewer");
    assert_eq!(roles[0].instructions, "Review carefully.");
    assert!(
        load_role_catalog(&ctx, dir.path().join("missing"))
            .unwrap()
            .is_empty()
    );
    for text in [
        "---\nunknown: yes\n---\nBad",
        "---\nname: reviewer\n---\nDuplicate",
    ] {
        write(dir.path(), "custom/invalid.md", text);
        assert!(load_role_catalog(&ctx, dir.path().join("custom")).is_err());
    }
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = Context {
        cancellation: Arc::new(token),
        ..context()
    };
    assert_eq!(
        load_role_catalog(&cancelled, dir.path().join("missing"))
            .unwrap_err()
            .info
            .category,
        ErrorCategory::Cancelled,
    );
}

#[test]
fn normalized_mode_fields_routing_and_constraints_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "modes/sample.yaml",
        r#"
metadata:
  name: ' sample '
spec:
  version: ' v2 '
  displayName: ' Review '
  description: ' Description '
  category: ' direct '
  instructions: ' Inspect '
  toolAccess: ' READ_ONLY '
  modelRouting:
    defaultModel: ' openai/primary '
    fallbackModels: [' openai/fallback ', '', '  ']
    reasoningLevel: ' high '
    textVerbosity: ' low '
    roleOverrides:
      critic:
        model: ' openai/critic '
        fallbackModels: [' ', ' openai/backup ']
        reasoningLevel: ' medium '
        textVerbosity: ' high '
  constraints:
    maxTurns: 7
    subAgentMaxTurns: 3
    maxConcurrentSubAgents: 2
    maxRuntimeMinutes: 10
    maxRetries: 0
"#,
    );
    let mode = FileConfigSource::new(dir.path())
        .get_mode(&context(), "sample")
        .unwrap();
    assert_eq!(mode.name, "sample");
    assert_eq!(mode.version, "v2");
    assert_eq!(mode.display_name, "Review");
    assert_eq!(mode.description, "Description");
    assert_eq!(mode.category, "direct");
    assert_eq!(mode.instructions, "Inspect");
    assert_eq!(mode.tool_access, "read-only");
    let routing = mode.model_routing.unwrap();
    assert_eq!(routing.default_model, "openai/primary");
    assert_eq!(routing.fallback_models.unwrap(), ["openai/fallback"]);
    assert_eq!(routing.reasoning_level, "high");
    assert_eq!(routing.text_verbosity, "low");
    let role = &routing.role_overrides["critic"];
    assert_eq!(role.model, "openai/critic");
    assert_eq!(role.fallback_models.as_ref().unwrap(), &["openai/backup"]);
    assert_eq!(role.reasoning_level, "medium");
    assert_eq!(role.text_verbosity, "high");
    let limits = mode.constraints.unwrap();
    assert_eq!(limits.max_concurrent_subagents.unwrap().get(), 2);
    assert_eq!(limits.max_runtime_minutes.unwrap().get(), 10);
    assert_eq!(limits.max_turns.unwrap().get(), 7);
    assert_eq!(limits.subagent_max_turns.unwrap().get(), 3);
    assert_eq!(limits.max_retries, Some(0));
}

#[test]
fn role_frontmatter_uses_first_nonblank_alias_instead_of_duplicate_alias_errors() {
    let dir = tempfile::tempdir().unwrap();
    for (path, access, model) in [("first", " read_only ", " primary "), ("blank", "  ", " ")] {
        write(
            dir.path(),
            &format!("agents/{path}.md"),
            &format!(
                "---\r\nname: ' {path} '\r\ndescription: ' Review code '\r\ntool_access: '{access}'\r\ntoolAccess: execution\r\nmodel_override: '{model}'\r\nmodel: ' alternate '\r\n---\r\n Instructions \r\n"
            ),
        );
    }
    write(dir.path(), "agents/plain.md", "Plain instructions");
    let roles = FileConfigSource::new(dir.path())
        .load_roles(&context())
        .unwrap();
    assert_eq!(roles[0].tool_access, "full");
    assert_eq!(roles[0].model_override, "alternate");
    assert_eq!(roles[1].tool_access, "read-only");
    assert_eq!(roles[1].model_override, "primary");
    assert_eq!(roles[1].description, " Review code ");
    assert_eq!(roles[1].instructions, "Instructions");
    assert_eq!(roles[2].tool_access, "full");
}

#[test]
fn strict_aggregate_rejects_duplicate_names_unknown_fields_and_invalid_limits() {
    let dir = tempfile::tempdir().unwrap();
    let source = FileConfigSource::new(dir.path());
    for text in [
        "name: ../escape",
        "unknown: value",
        "toolAccess: typo",
        "constraints:\n  maxTurns: 0",
        "constraints:\n  subAgentMaxTurns: 0",
        "constraints:\n  maxConcurrentSubAgents: 0",
        "constraints:\n  maxRuntimeMinutes: 0",
        "name: one\nname: two",
        "modelRouting:\n  unknown: value",
    ] {
        write(dir.path(), "modes/test.yaml", text);
        assert!(source.load_files(&context()).is_err(), "{text}");
    }
    write(dir.path(), "modes/test.yaml", "name: custom");
    write(dir.path(), "modes/duplicate.yaml", "name: CUSTOM");
    assert!(source.load_files(&context()).is_err());
    fs::remove_dir_all(dir.path().join("modes")).unwrap();
    for text in [
        "---\nunknown: yes\n---\nBody",
        "---\ntool_access: typo\n---\nBody",
        "---\nname: role",
        " ",
        "---\nname: one\nname: two\n---\nBody",
    ] {
        write(dir.path(), "agents/test.md", text);
        assert!(source.load_files(&context()).is_err(), "{text}");
    }
    write(dir.path(), "agents/test.md", "---\nname: role\n---\nBody");
    write(
        dir.path(),
        "agents/duplicate.md",
        "---\nname: role\n---\nBody",
    );
    assert!(source.load_files(&context()).is_err());
}

#[tokio::test]
async fn cancellation_is_checked_by_independent_operations_and_active_facade() {
    let dir = tempfile::tempdir().unwrap();
    let source = FileConfigSource::new(dir.path());
    let token = CancellationToken::new();
    token.cancel();
    let ctx = Context {
        cancellation: Arc::new(token),
        ..context()
    };
    assert!(source.list_modes(&ctx).is_err());
    assert!(source.get_mode(&ctx, "chat").is_err());
    assert!(source.load_roles(&ctx).is_err());
    assert!(source.mode_snapshot(&ctx).await.unwrap().is_none());
    let source = source.with_active_mode("chat");
    assert!(source.mode_snapshot(&ctx).await.is_err());
    assert!(source.mode_directive(&ctx).await.is_err());
    assert!(source.permission_mode(&ctx).await.is_err());
}

#[test]
fn pure_directive_matches_go_labels_restrictions_and_whitespace() {
    assert_eq!(build_mode_directive(None), "");
    assert_eq!(build_mode_directive(Some(&ModeSpec::default())), "");
    let mut mode = ModeSpec {
        name: " critic ".into(),
        display_name: " Review ".into(),
        description: " Description ".into(),
        instructions: " Inspect\ncarefully ".into(),
        tool_access: " ANALYSIS ".into(),
        ..Default::default()
    };
    assert_eq!(
        build_mode_directive(Some(&mode)),
        "Mode: Review\n\nMode description: Description\n\nTool access: read-only. Do not modify files or run mutating commands. Bash is restricted to statically-authorized commands: no command substitution $(...) or backticks, no heredocs, no VAR=value prefixes, no eval/source, no function definitions, no git-mutating subcommands (commit/add/push/merge/rebase/branch/tag/stash/remote), and no gh CLI — use plain literal read commands and the dedicated read tools (read_file, grep, glob, list_files) instead.\n\nInspect\ncarefully"
    );
    mode.display_name = " ".into();
    mode.tool_access = "full".into();
    assert_eq!(
        build_mode_directive(Some(&mode)),
        "Mode: critic\n\nMode description: Description\n\nInspect\ncarefully"
    );
}

struct NoCalls;
impl adk::core::Model for NoCalls {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: adk::core::ModelRequest,
    ) -> adk::core::BoxFuture<'a, Result<adk::core::ModelResponse, adk::core::Error>> {
        panic!("build must not call a model")
    }
}
impl adk::core::StreamingModel for NoCalls {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: adk::core::ModelRequest,
    ) -> adk::core::BoxFuture<'a, Result<Box<dyn adk::core::ModelStream + 'a>, adk::core::Error>>
    {
        panic!("build must not call a model")
    }
}
impl adk::runtime::subagent::ChildExecutor for NoCalls {
    fn execute<'a>(
        &'a self,
        _: adk::runtime::subagent::ChildInvocation,
        _: adk::runtime::subagent::ChildControl,
    ) -> adk::core::BoxFuture<'a, Result<adk::runtime::subagent::ChildOutcome, adk::core::Error>>
    {
        panic!("build must not execute a child")
    }
}

#[tokio::test]
async fn builder_checks_injected_concurrency_ceiling_without_mutating_owner() {
    use adk::builder::{Builder, Config, Features, SessionState};
    use adk::runtime::subagent::{Scheduler, SchedulerConfig};
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "modes/limited.yaml",
        "constraints:\n  maxConcurrentSubAgents: 2\n  maxRuntimeMinutes: 10\n",
    );
    let ctx = context();
    for ceiling in [1, 2, 3] {
        let scheduler = Scheduler::new(
            ctx.clone(),
            SchedulerConfig {
                max_concurrency: ceiling,
                ..Default::default()
            },
            Arc::new(NoCalls),
            None,
        )
        .unwrap();
        let handle = scheduler.handle();
        let mut owner = SessionState::with_scheduler(scheduler);
        let result = Builder::new(Config {
            active_mode: Some("limited".into()),
            features: Some(Features {
                subagents: true,
                ..Default::default()
            }),
            ..Default::default()
        })
        .model(
            "openai",
            adk::providers::factory::Kind::OpenAi,
            Arc::new(NoCalls),
        )
        .unwrap()
        .source(Arc::new(FileConfigSource::new(dir.path())))
        .session(owner.handle())
        .build(&ctx)
        .await;
        match result {
            Ok(mut bundle) => {
                assert!(ceiling <= 2);
                bundle.close().await.unwrap();
            }
            Err(error) => {
                assert_eq!(ceiling, 3);
                assert_eq!(error.info.category, ErrorCategory::InvalidInput);
                assert!(error.to_string().contains("maxConcurrentSubAgents"));
            }
        }
        assert_eq!(handle.max_concurrency(), ceiling);
        assert!(!owner.handle().is_closed());
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn builder_selection_retains_full_validation_instead_of_direct_lookup() {
    use adk::builder::{Builder, Config};
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "modes/good.yaml", "name: good");
    write(dir.path(), "modes/bad.yaml", "unknown: value");
    let ctx = context();
    assert!(
        FileConfigSource::new(dir.path())
            .get_mode(&ctx, "good")
            .is_ok()
    );
    let result = Builder::new(Config {
        active_mode: Some("good".into()),
        ..Default::default()
    })
    .model(
        "openai",
        adk::providers::factory::Kind::OpenAi,
        Arc::new(NoCalls),
    )
    .unwrap()
    .source(Arc::new(
        FileConfigSource::new(dir.path()).with_active_mode("good"),
    ))
    .build(&ctx)
    .await;
    assert!(matches!(result, Err(error) if error.info.category == ErrorCategory::InvalidInput));
}

#[tokio::test]
async fn pinned_go_directives_permissions_and_full_schema() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/fileconfig/sdk-fileconfig.json"
    ))
    .unwrap();
    let mut checked = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if !matches!(
            name,
            "builtin-chat" | "builtin-plan" | "plain-full-schema" | "direct-formatter"
        ) && !name.starts_with("access-")
        {
            continue;
        }
        if !case["source_only"].as_array().unwrap().is_empty() {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in case["input"]["files"].as_object().unwrap() {
            write(dir.path(), path, text.as_str().unwrap());
        }
        let source = FileConfigSource::new(dir.path())
            .with_active_mode(case["input"]["active_mode"].as_str().unwrap());
        for (query, expected) in case["input"]["queries"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["output"].as_array().unwrap())
        {
            let actual = match query["operation"].as_str().unwrap() {
                "BuildModeDirective" => {
                    let template = &query["template"];
                    let mode = (!template.is_null()).then(|| ModeSpec {
                        name: template["Name"].as_str().unwrap().into(),
                        display_name: template["DisplayName"].as_str().unwrap().into(),
                        description: template["Description"].as_str().unwrap().into(),
                        tool_access: template["ToolAccess"].as_str().unwrap().into(),
                        instructions: template["Instructions"].as_str().unwrap().into(),
                        ..Default::default()
                    });
                    build_mode_directive(mode.as_ref())
                }
                "ModeDirective" => source.mode_directive(&context()).await.unwrap(),
                "PermissionMode" => match source.permission_mode(&context()).await.unwrap() {
                    PermissionMode::ReadOnly => "read-only".into(),
                    PermissionMode::WorkspaceWrite => "workspace-write".into(),
                    PermissionMode::DangerFullAccess => {
                        panic!("fileconfig must not elevate access")
                    }
                },
                _ => continue,
            };
            assert_eq!(
                actual,
                expected["result"].as_str().unwrap(),
                "{name}: {query}"
            );
            checked += 1;
        }
        if name == "plain-full-schema" {
            let actual = source.get_mode(&context(), "custom").unwrap();
            let expected = &case["output"][1]["result"];
            assert_eq!(actual.name, expected["Name"]);
            assert_eq!(actual.version, expected["Version"]);
            assert_eq!(actual.autonomous, expected["Autonomous"]);
            let routing = actual.model_routing.unwrap();
            assert_eq!(
                routing.default_model,
                expected["ModelRouting"]["DefaultModel"]
            );
            assert_eq!(
                serde_json::json!(routing.fallback_models),
                expected["ModelRouting"]["FallbackModels"]
            );
            assert_eq!(
                serde_json::json!(routing.role_overrides[" planner "].fallback_models),
                expected["ModelRouting"]["RoleOverrides"][" planner "]["FallbackModels"]
            );
            let constraints = actual.constraints.unwrap();
            assert_eq!(
                constraints.max_turns.unwrap().get(),
                expected["Constraints"]["MaxTurns"].as_u64().unwrap() as u32
            );
            assert_eq!(
                constraints.subagent_max_turns.unwrap().get(),
                expected["Constraints"]["SubAgentMaxTurns"]
                    .as_u64()
                    .unwrap() as u32
            );
            assert_eq!(
                constraints.max_concurrent_subagents.unwrap().get(),
                expected["Constraints"]["MaxConcurrentSubAgents"]
                    .as_u64()
                    .unwrap() as u32
            );
            assert_eq!(
                constraints.max_runtime_minutes.unwrap().get(),
                expected["Constraints"]["MaxRuntimeMinutes"]
                    .as_u64()
                    .unwrap() as u32
            );
            assert_eq!(
                constraints.max_retries.unwrap(),
                expected["Constraints"]["MaxRetries"].as_u64().unwrap() as u32
            );
        }
    }
    assert_eq!(checked, 34);
}

#[tokio::test]
async fn unterminated_role_frontmatter_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "agents/reviewer.md",
        "---\nname: reviewer\ntool_access: read-only\n",
    );
    let source = FileConfigSource::new(root.path());
    assert!(source.load_roles(&context()).is_err());
}
