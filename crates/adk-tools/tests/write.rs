#![cfg(target_os = "linux")]
use adk_core::*;
use adk_tools::{Config, Features, Registry};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
    sync::Arc,
};

fn config(access: AccessMode) -> Config {
    Config {
        access,
        features: Features::Strict(["Write".into()].into()),
        ..Default::default()
    }
}
async fn call(root: &Path, access: AccessMode, arguments: Value) -> ToolOutput {
    let context = ToolContext {
        operation: Context {
            run_id: "write".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: Default::default(),
        idempotency_key: None,
    };
    Registry::build(&config(access), [])
        .unwrap()
        .get("Write")
        .unwrap()
        .execute(
            &context,
            ToolCall {
                id: "test".into(),
                name: "Write".into(),
                arguments,
            },
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn workspace_creates_parents_preserves_modes_and_replaces_inodes() {
    let root = tempfile::tempdir().unwrap();
    let output = call(
        root.path(),
        AccessMode::WorkspaceWrite,
        json!({"file_path":"nested/file", "content":"€"}),
    )
    .await;
    assert!(!output.is_error, "{output:?}");
    assert_eq!(
        output.content,
        vec![Content::Text {
            text: format!(
                "Successfully wrote 3 bytes to {}/nested/file",
                root.path().display()
            )
        }]
    );
    let path = root.path().join("nested/file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    assert!(
        !call(
            root.path(),
            AccessMode::WorkspaceWrite,
            json!({"file_path":path, "content":"new"})
        )
        .await
        .is_error
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o751
    );
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(fs::read_dir(root.path().join("nested")).unwrap().count(), 1);
}
#[tokio::test]
async fn workspace_rejects_escape_but_preserves_internal_alias_semantics() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
    fs::write(root.path().join("target"), "old").unwrap();
    symlink("target", root.path().join("internal")).unwrap();
    for path in [
        root.path().join("escape/new"),
        root.path().join("link"),
        outside.path().join("secret"),
        root.path().join(".."),
    ] {
        assert!(
            call(
                root.path(),
                AccessMode::WorkspaceWrite,
                json!({"file_path":path,"content":"bad"})
            )
            .await
            .is_error
        );
    }
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "secret"
    );
    assert!(!outside.path().join("new").exists());
    assert!(
        !call(
            root.path(),
            AccessMode::WorkspaceWrite,
            json!({"file_path":"internal","content":"new"})
        )
        .await
        .is_error
    );
    assert!(
        fs::symlink_metadata(root.path().join("internal"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(root.path().join("target")).unwrap(),
        "new"
    );
    fs::hard_link(outside.path().join("secret"), root.path().join("hard")).unwrap();
    assert!(
        !call(
            root.path(),
            AccessMode::WorkspaceWrite,
            json!({"file_path":"hard","content":"isolated"})
        )
        .await
        .is_error
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "secret"
    );
}
#[tokio::test]
async fn full_access_retains_direct_write_semantics_and_refuses_final_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("file");
    assert!(
        !call(
            root.path(),
            AccessMode::FullAccess,
            json!({"file_path":path,"content":"old"})
        )
        .await
        .is_error
    );
    let inode = fs::metadata(&path).unwrap().ino();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        !call(
            root.path(),
            AccessMode::FullAccess,
            json!({"file_path":path,"content":"new"})
        )
        .await
        .is_error
    );
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    symlink(&path, root.path().join("link")).unwrap();
    assert!(
        call(
            root.path(),
            AccessMode::FullAccess,
            json!({"file_path":"link","content":"bad"})
        )
        .await
        .is_error
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "new");
}
#[tokio::test]
async fn invalid_inputs_and_directories_do_not_write_and_read_only_excludes_write() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        Registry::build(&config(AccessMode::ReadOnly), [])
            .unwrap()
            .names()
            .count(),
        0
    );
    for access in [AccessMode::FullAccess, AccessMode::WorkspaceWrite] {
        for input in [
            json!({}),
            Value::Null,
            json!({"file_path":3}),
            json!({"file_path":root.path(),"content":"bad"}),
        ] {
            assert!(call(root.path(), access, input).await.is_error);
        }
        assert!(
            !call(
                root.path(),
                access,
                json!({"file_path":"empty","content":null})
            )
            .await
            .is_error
        );
        assert_eq!(fs::read(root.path().join("empty")).unwrap(), b"");
    }
}

#[tokio::test]
async fn cancelled_write_and_special_files_never_change_contents() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a"), "unchanged").unwrap();
    let token = Arc::new(adk_runtime::CancellationToken::new());
    token.cancel();
    let context = ToolContext {
        operation: Context {
            run_id: "cancelled-write".into(),
            cancellation: token,
            deadline: None,
        },
        work_dir: root.path().into(),
        policy: Default::default(),
        idempotency_key: None,
    };
    assert!(
        Registry::build(&config(AccessMode::WorkspaceWrite), [])
            .unwrap()
            .get("Write")
            .unwrap()
            .execute(
                &context,
                ToolCall {
                    id: "call".into(),
                    name: "Write".into(),
                    arguments: json!({"file_path":"a","content":"bad"}),
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(root.path().join("a")).unwrap(),
        "unchanged"
    );
    rustix::fs::mknodat(
        rustix::fs::CWD,
        root.path().join("fifo"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR,
        0,
    )
    .unwrap();
    for access in [AccessMode::FullAccess, AccessMode::WorkspaceWrite] {
        assert!(
            call(
                root.path(),
                access,
                json!({"file_path":"fifo","content":"bad"})
            )
            .await
            .is_error
        );
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
