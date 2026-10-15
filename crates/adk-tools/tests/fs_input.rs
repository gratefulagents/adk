#![cfg(any(target_os = "linux", target_os = "macos"))]

use adk_core::*;
use adk_tools::{Config, Features, Registry};
use serde_json::json;
use std::{fs, sync::Arc};

#[tokio::test]
async fn omitted_fields_follow_go_zero_values_not_schema_required_flags() {
    let root = tempfile::tempdir().unwrap();
    let context = ToolContext {
        operation: Context {
            run_id: "input-defaults".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.path().into(),
        policy: Default::default(),
        idempotency_key: None,
    };
    for access in [AccessMode::WorkspaceWrite, AccessMode::FullAccess] {
        let registry = Registry::build(
            &Config {
                access,
                features: Features::Strict(
                    [
                        "Write".into(),
                        "Edit".into(),
                        "Move".into(),
                        "Delete".into(),
                    ]
                    .into(),
                ),
                ..Default::default()
            },
            [],
        )
        .unwrap();
        for (name, arguments, initial, error, expected) in [
            ("Write", json!({"file_path":"a"}), "alpha", false, ""),
            (
                "Write",
                json!({"file_path":"a","content":null}),
                "alpha",
                false,
                "",
            ),
            (
                "Edit",
                json!({"file_path":"a","old_string":"alpha"}),
                "alpha",
                false,
                "",
            ),
            (
                "Edit",
                json!({"file_path":"a","old_string":"alpha","new_string":null}),
                "alpha",
                false,
                "",
            ),
            ("Write", json!(null), "alpha", true, "alpha"),
            ("Edit", json!(null), "alpha", true, "alpha"),
            ("Move", json!({"source_path":"a"}), "alpha", true, "alpha"),
            (
                "Move",
                json!({"destination_path":"b"}),
                "alpha",
                true,
                "alpha",
            ),
            ("Move", json!(null), "alpha", true, "alpha"),
            ("Delete", json!({}), "alpha", true, "alpha"),
            (
                "Edit",
                json!({"file_path":"a","old_string":"alpha","new_string":"x"}),
                "alpha alpha",
                true,
                "alpha alpha",
            ),
            (
                "Edit",
                json!({"file_path":"a","old_string":"alpha","new_string":"x","replace_all":null}),
                "alpha alpha",
                true,
                "alpha alpha",
            ),
        ] {
            fs::write(root.path().join("a"), initial).unwrap();
            let output = registry
                .get(name)
                .unwrap()
                .execute(
                    &context,
                    ToolCall {
                        id: "defaults".into(),
                        name: name.into(),
                        arguments,
                        raw_arguments: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(output.is_error, error, "{name}: {output:?}");
            assert_eq!(fs::read_to_string(root.path().join("a")).unwrap(), expected);
            assert!(!root.path().join("b").exists());
        }
    }
}

#[tokio::test]
async fn folded_raw_paths_cannot_bypass_workspace_confinement() {
    for (name, field, extra) in [
        ("Write", "file_path", r#", "content":"changed""#),
        (
            "Edit",
            "file_path",
            r#", "old_string":"alpha", "new_string":"changed""#,
        ),
        ("Move", "source_path", r#", "destination_path":"b""#),
        ("Delete", "path", ""),
    ] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a"), "alpha").unwrap();
        let external = outside.path().join("secret");
        fs::write(&external, "alpha").unwrap();
        let path = serde_json::to_string(&external).unwrap();
        let upper = field.to_ascii_uppercase();
        let raw = format!(r#"{{"{field}":"a","{upper}":{path}{extra}}}"#);
        let registry = Registry::build(
            &Config {
                access: AccessMode::WorkspaceWrite,
                features: Features::Strict([name.into()].into()),
                ..Default::default()
            },
            [],
        )
        .unwrap();
        let context = ToolContext {
            operation: Context {
                run_id: "folded-path".into(),
                cancellation: Arc::new(adk_runtime::CancellationToken::new()),
                deadline: None,
            },
            work_dir: root.path().into(),
            policy: Default::default(),
            idempotency_key: None,
        };
        let output = registry
            .get(name)
            .unwrap()
            .execute(
                &context,
                ToolCall {
                    id: "folded".into(),
                    name: name.into(),
                    arguments: serde_json::from_str(&raw).unwrap(),
                    raw_arguments: Some(raw),
                },
            )
            .await
            .unwrap();
        assert!(output.is_error, "{name}: {output:?}");
        assert_eq!(fs::read_to_string(root.path().join("a")).unwrap(), "alpha");
        assert_eq!(fs::read_to_string(external).unwrap(), "alpha");
        assert!(!root.path().join("b").exists());
    }
}
