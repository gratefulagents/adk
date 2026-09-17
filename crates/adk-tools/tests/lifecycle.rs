#![cfg(target_os = "linux")]
use adk_core::*;
use adk_tools::{Config, Features, Registry};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::symlink, path::Path, sync::Arc};

fn config(access: AccessMode) -> Config {
    Config {
        access,
        features: Features::Strict(["Move".into(), "Delete".into()].into()),
        ..Default::default()
    }
}
fn context(root: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "lifecycle".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
async fn call(root: &Path, name: &str, arguments: Value) -> ToolOutput {
    Registry::build(&config(AccessMode::WorkspaceWrite), [])
        .unwrap()
        .get(name)
        .unwrap()
        .execute(
            &context(root),
            ToolCall {
                id: "test".into(),
                name: name.into(),
                arguments,
            },
        )
        .await
        .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    assert!(!output.should_pause);
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("expected text"),
    }
}
fn assert_no_quarantine(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        assert!(
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".agentsdk-lifecycle-")
        );
    }
}

#[tokio::test]
async fn moves_files_and_directories_and_deletes_only_empty_directories() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("parent")).unwrap();
    fs::write(root.path().join("a"), "content").unwrap();
    let output = call(
        root.path(),
        "Move",
        json!({"source_path":"a", "destination_path":"parent//b"}),
    )
    .await;
    assert!(!output.is_error, "{output:?}");
    assert_eq!(
        text(&output),
        r#"{"destination_path":"parent/b","operation":"move","source_path":"a"}"#
    );
    assert!(!root.path().join("a").exists());
    let output = call(
        root.path(),
        "Move",
        json!({"source_path":"parent", "destination_path":"renamed"}),
    )
    .await;
    assert!(!output.is_error);
    assert_eq!(
        fs::read_to_string(root.path().join("renamed/b")).unwrap(),
        "content"
    );
    assert!(
        call(root.path(), "Delete", json!({"path":"renamed"}))
            .await
            .is_error
    );
    assert!(root.path().join("renamed/b").exists());
    let output = call(root.path(), "Delete", json!({"path":"renamed/b"})).await;
    assert!(!output.is_error);
    assert_eq!(
        text(&output),
        r#"{"operation":"delete","path":"renamed/b"}"#
    );
    assert!(
        !call(root.path(), "Delete", json!({"path":"renamed"}))
            .await
            .is_error
    );
    assert_no_quarantine(root.path());
}

#[tokio::test]
async fn failures_restore_source_and_never_replace_destination() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a"), "source").unwrap();
    fs::write(root.path().join("b"), "destination").unwrap();
    for destination in ["b", "missing/child", "a"] {
        assert!(
            call(
                root.path(),
                "Move",
                json!({"source_path":"a","destination_path":destination})
            )
            .await
            .is_error
        );
        assert_eq!(fs::read_to_string(root.path().join("a")).unwrap(), "source");
        assert_eq!(
            fs::read_to_string(root.path().join("b")).unwrap(),
            "destination"
        );
        assert_no_quarantine(root.path());
    }
    fs::create_dir(root.path().join("dir")).unwrap();
    assert!(
        call(
            root.path(),
            "Move",
            json!({"source_path":"dir","destination_path":"dir/child"})
        )
        .await
        .is_error
    );
    assert!(root.path().join("dir").is_dir());
    assert_no_quarantine(root.path());
}

#[tokio::test]
async fn rejects_links_special_files_and_path_traversal_without_touching_targets() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
    symlink(outside.path(), root.path().join("parent-link")).unwrap();
    fs::hard_link(outside.path().join("secret"), root.path().join("hard")).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        root.path().join("fifo"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR,
        0,
    )
    .unwrap();
    for path in [
        "link",
        "hard",
        "fifo",
        "parent-link/secret",
        "../secret",
        ".",
        "a/../b",
        "./a",
        "",
        "a\\b",
        "/tmp/x",
        "a\nb",
    ] {
        assert!(
            call(root.path(), "Delete", json!({"path":path}))
                .await
                .is_error,
            "{path}"
        );
        assert!(
            call(
                root.path(),
                "Move",
                json!({"source_path":path,"destination_path":"moved"})
            )
            .await
            .is_error,
            "{path}"
        );
        assert_no_quarantine(root.path());
    }
    assert!(
        fs::symlink_metadata(root.path().join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(root.path().join("hard").exists());
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "secret"
    );
}

#[tokio::test]
async fn read_only_selection_cancellation_and_invalid_inputs_do_not_mutate() {
    assert_eq!(
        Registry::build(&config(AccessMode::ReadOnly), [])
            .unwrap()
            .names()
            .count(),
        0
    );
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a"), "original").unwrap();
    let token = Arc::new(adk_runtime::CancellationToken::new());
    token.cancel();
    let mut context = context(root.path());
    context.operation.cancellation = token;
    let registry = Registry::build(&config(AccessMode::WorkspaceWrite), []).unwrap();
    assert!(
        registry
            .get("Delete")
            .unwrap()
            .execute(
                &context,
                ToolCall {
                    id: "test".into(),
                    name: "Delete".into(),
                    arguments: json!({"path":"a"})
                }
            )
            .await
            .is_err()
    );
    for input in [
        json!({"path":3}),
        json!([]),
        Value::Null,
        json!({"path":null}),
    ] {
        assert!(call(root.path(), "Delete", input).await.is_error);
    }
    assert_eq!(
        fs::read_to_string(root.path().join("a")).unwrap(),
        "original"
    );
    assert_no_quarantine(root.path());
}

#[tokio::test]
async fn parent_symlink_swaps_cannot_mutate_outside_workspace() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("parent")).unwrap();
    for index in 0..100 {
        fs::write(root.path().join(format!("parent/{index}")), "inside").unwrap();
        fs::write(outside.path().join(index.to_string()), "outside").unwrap();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let parent = root.path().join("parent");
    let parked = root.path().join("parked");
    let target = outside.path().to_path_buf();
    let worker = std::thread::spawn(move || {
        while !worker_stop.load(Ordering::Relaxed) {
            fs::rename(&parent, &parked).unwrap();
            symlink(&target, &parent).unwrap();
            std::thread::yield_now();
            fs::remove_file(&parent).unwrap();
            fs::rename(&parked, &parent).unwrap();
        }
    });
    for index in 0..100 {
        if index % 2 == 0 {
            call(
                root.path(),
                "Delete",
                json!({"path":format!("parent/{index}")}),
            )
            .await;
        } else {
            call(root.path(), "Move", json!({"source_path":format!("parent/{index}"),"destination_path":format!("moved-{index}")})).await;
        }
    }
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    for index in 0..100 {
        assert_eq!(
            fs::read_to_string(outside.path().join(index.to_string())).unwrap(),
            "outside"
        );
    }
    assert_no_quarantine(root.path());
    assert_no_quarantine(&root.path().join("parent"));
}
