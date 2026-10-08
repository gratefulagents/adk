use super::*;
use adk::project_state::{FilesystemOptions, FilesystemResolutionHost, ProjectStore, StoreOptions};

fn selection(bits: u8) -> ProjectStateFeatures {
    ProjectStateFeatures {
        prime_context: bits & 1 != 0,
        task_tools: bits & 2 != 0,
        memory_tools: bits & 4 != 0,
        prime_tool: bits & 8 != 0,
    }
}
fn config(dir: &std::path::Path, bits: u8) -> Config {
    Config {
        work_dir: dir.into(),
        project_state: ProjectStateConfig {
            state_dir: dir.join("state"),
            project_id: "offline".into(),
            ..Default::default()
        },
        features: Some(Features {
            project_state: selection(bits),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn host(dir: &std::path::Path) -> FilesystemResolutionHost {
    FilesystemResolutionHost {
        cwd: dir.into(),
        home: None,
    }
}
#[tokio::test]
async fn all_project_state_feature_combinations_open_only_when_needed() {
    let model = Arc::new(RecordingModel::default());
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/project-state/runtime-observations.json"
    ))
    .unwrap();
    for bits in 0..16 {
        let dir = tempfile::tempdir().unwrap();
        let mut bundle = builder(config(dir.path(), bits as u8), &model)
            .project_state_host(host(dir.path()))
            .build(&context())
            .await
            .unwrap();
        assert_eq!(
            dir.path().join("state").exists(),
            oracle["cases"][bits]["created"].as_bool().unwrap()
        );
        let names: Vec<_> = bundle
            .agent()
            .tools
            .iter()
            .map(|t| t.definition().name.as_str())
            .collect();
        assert_eq!(
            names.iter().filter(|n| n.starts_with("task_")).count(),
            if bits & 2 != 0 { 8 } else { 0 }
        );
        assert_eq!(
            names.iter().filter(|n| n.starts_with("memory_")).count(),
            if bits & 4 != 0 { 6 } else { 0 }
        );
        assert_eq!(names.contains(&"prime_context"), bits & 8 != 0);
        let mut actual = names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        actual.sort();
        assert_eq!(serde_json::json!(actual), oracle["cases"][bits]["tools"]);
        assert!(bundle.warnings().is_empty());
        bundle.close().await.unwrap();
        bundle.close().await.unwrap();
    }
}
#[tokio::test]
async fn injected_store_wins_and_retained_tools_are_revoked_on_close_and_drop() {
    for close in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            ProjectStore::filesystem(FilesystemOptions {
                state_dir: dir.path().join("injected"),
                store: StoreOptions {
                    project_id: "injected".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap(),
        );
        let mut bundle = builder(config(dir.path(), 2), &Arc::new(RecordingModel::default()))
            .project_state_store(store.clone())
            .build(&context())
            .await
            .unwrap();
        assert!(!dir.path().join("state").exists());
        let tool = bundle
            .agent()
            .tools
            .iter()
            .find(|t| t.definition().name == "task_create")
            .unwrap()
            .clone();
        let operation = ToolContext {
            operation: context(),
            work_dir: dir.path().into(),
            policy: bundle.policy().tools.clone(),
            idempotency_key: None,
        };
        let call = || ToolCall {
            raw_arguments: None,
            id: "offline".into(),
            name: "task_create".into(),
            arguments: json!({"title":"persistent task"}),
        };
        assert!(!tool.execute(&operation, call()).await.unwrap().is_error);
        assert_eq!(store.list_tasks().unwrap().len(), 1);
        if close {
            bundle.close().await.unwrap();
        }
        drop(bundle);
        assert_eq!(
            category(tool.execute(&operation, call()).await),
            ErrorCategory::Cancelled
        );
        assert_eq!(store.list_tasks().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn priming_does_not_leak_into_initial_prompt_or_claim_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        ProjectStore::filesystem(FilesystemOptions {
            state_dir: dir.path().join("injected"),
            store: StoreOptions {
                project_id: "injected".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap(),
    );
    let task = store
        .create_task(adk::project_state::CreateTaskInput {
            title: "prime marker".into(),
            ..Default::default()
        })
        .unwrap();
    let model = Arc::new(RecordingModel::default());
    let mut cfg = config(dir.path(), 1);
    cfg.project_state.active_task_id = task.id.clone();
    let mut bundle = builder(cfg, &model)
        .project_state_store(store.clone())
        .runner_config(RunnerConfig {
            working_state_context: " existing marker ".into(),
            ..Default::default()
        })
        .build(&context())
        .await
        .unwrap();
    bundle
        .run(context(), vec![], Arc::new(TestHost))
        .await
        .unwrap();
    let serialized = format!("{:?}", model.requests.lock().unwrap()[0]);
    assert!(!serialized.contains("existing marker"), "{serialized}");
    assert!(!serialized.contains("prime marker"), "{serialized}");
    assert_eq!(store.get_task(&task.id).unwrap().status, "open");
    bundle.close().await.unwrap();
}
#[tokio::test]
async fn automatic_store_errors_fail_build_without_exposing_paths() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("state"), "not a directory").unwrap();
    let error = builder(config(dir.path(), 1), &Arc::new(RecordingModel::default()))
        .project_state_host(host(dir.path()))
        .build(&context())
        .await
        .err()
        .unwrap();
    assert_eq!(error.info.message, "project-state initialization failed");
}

#[tokio::test]
async fn prime_failure_is_nonfatal_and_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        ProjectStore::filesystem(FilesystemOptions {
            state_dir: dir.path().join("injected"),
            store: StoreOptions {
                project_id: "injected".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap(),
    );
    std::fs::write(
        dir.path().join("injected/events.jsonl"),
        "private invalid JSON\nsecond invalid record\n",
    )
    .unwrap();
    let mut bundle = builder(config(dir.path(), 1), &Arc::new(RecordingModel::default()))
        .project_state_store(store)
        .build(&context())
        .await
        .unwrap();
    assert_eq!(bundle.warnings(), &["project-state priming failed"]);
    bundle.close().await.unwrap();
}
#[tokio::test]
async fn legacy_project_state_and_read_only_policy_use_composed_registration() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path(), 0);
    cfg.features = None;
    cfg.legacy_tools.enable_project_state = true;
    cfg.policy.tools.access = AccessMode::ReadOnly;
    let mut bundle = builder(cfg, &Arc::new(RecordingModel::default()))
        .project_state_host(host(dir.path()))
        .build(&context())
        .await
        .unwrap();
    let names: Vec<_> = bundle
        .agent()
        .tools
        .iter()
        .map(|t| t.definition().name.as_str())
        .collect();
    assert!(names.contains(&"task_ready"));
    assert!(names.contains(&"prime_context"));
    assert!(!names.contains(&"task_create"));
    assert!(!names.contains(&"memory_remember"));
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn tool_only_project_state_matches_pinned_selection_without_a_provider() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/project-state/runtime-observations.json"
    ))
    .unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        let dir = tempfile::tempdir().unwrap();
        let mut bundle = Builder::new(config(dir.path(), case["bits"].as_u64().unwrap() as u8))
            .project_state_host(host(dir.path()))
            .build_tools(&context())
            .await
            .unwrap();
        assert_eq!(
            dir.path().join("state").exists(),
            case["tool_only_created"].as_bool().unwrap()
        );
        let mut names: Vec<_> = bundle
            .prepared()
            .tools
            .iter()
            .map(|t| t.definition().name.clone())
            .collect();
        names.sort();
        assert_eq!(serde_json::json!(names), case["tool_only_tools"]);
        bundle.close().await.unwrap();
    }
}
#[tokio::test]
async fn tool_only_builder_initializes_but_does_not_prime() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        ProjectStore::filesystem(FilesystemOptions {
            state_dir: dir.path().join("injected"),
            store: StoreOptions {
                project_id: "injected".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap(),
    );
    let invalid = "private invalid JSON\nsecond invalid record\n";
    let events = dir.path().join("injected/events.jsonl");
    std::fs::write(&events, invalid).unwrap();
    let mut bundle = Builder::new(config(dir.path(), 1))
        .project_state_store(store)
        .build_tools(&context())
        .await
        .unwrap();
    assert!(bundle.prepared().tools.is_empty());
    assert_eq!(std::fs::read_to_string(events).unwrap(), invalid);
    bundle.close().await.unwrap();
    let invalid_path = dir.path().join("state");
    std::fs::write(invalid_path, "not a directory").unwrap();
    assert!(
        Builder::new(config(dir.path(), 1))
            .project_state_host(host(dir.path()))
            .build_tools(&context())
            .await
            .is_err()
    );
}
