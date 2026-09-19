#![cfg(any(target_os = "linux", target_os = "macos"))]

use adk_core::*;
use adk_tools::{Config, Features, Registry};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
    sync::Arc,
};

async fn call(root: &Path, name: &str, arguments: Value) -> ToolOutput {
    let registry = Registry::build(
        &Config {
            access: AccessMode::WorkspaceWrite,
            features: Features::Strict([name.into()].into()),
            ..Default::default()
        },
        [],
    )
    .unwrap();
    registry
        .get(name)
        .unwrap()
        .execute(
            &ToolContext {
                operation: Context {
                    run_id: "macos-filesystem".into(),
                    cancellation: Arc::new(adk_runtime::CancellationToken::new()),
                    deadline: None,
                },
                work_dir: root.into(),
                policy: Default::default(),
                idempotency_key: None,
            },
            ToolCall {
                id: "call".into(),
                name: name.into(),
                arguments,
            },
        )
        .await
        .unwrap()
}

fn no_temporaries(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        assert!(
            !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".agentsdk-")
        );
        if entry.file_type().unwrap().is_dir() {
            no_temporaries(&entry.path());
        }
    }
}

#[tokio::test]
async fn atomic_writes_and_edits_preserve_modes_and_isolate_hardlinks() {
    let root = tempfile::tempdir().unwrap();
    let result = call(
        root.path(),
        "Write",
        json!({"file_path":"nested/file","content":"old"}),
    )
    .await;
    assert!(!result.is_error, "{result:?}");
    let file = root.path().join("nested/file");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o751)).unwrap();
    let inode = fs::metadata(&file).unwrap().ino();
    let result = call(
        root.path(),
        "Edit",
        json!({"file_path":"nested/file","old_string":"old","new_string":"new"}),
    )
    .await;
    assert!(!result.is_error, "{result:?}");
    assert_ne!(fs::metadata(&file).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o751
    );
    assert_eq!(fs::read(&file).unwrap(), b"new");
    let outside = tempfile::tempdir().unwrap();
    fs::hard_link(&file, outside.path().join("alias")).unwrap();
    assert!(
        call(
            root.path(),
            "Edit",
            json!({"file_path":"nested/file","old_string":"new","new_string":"bad"})
        )
        .await
        .is_error
    );
    let result = call(
        root.path(),
        "Write",
        json!({"file_path":"nested/file","content":"isolated"}),
    )
    .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(fs::read(outside.path().join("alias")).unwrap(), b"new");
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o751
    );
    no_temporaries(root.path());
}

#[tokio::test]
async fn move_never_overwrites_and_failures_restore_quarantined_sources() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source"), "source").unwrap();
    fs::write(root.path().join("destination"), "destination").unwrap();
    symlink("missing", root.path().join("link")).unwrap();
    for destination in ["destination", "link", "absent/file"] {
        assert!(
            call(
                root.path(),
                "Move",
                json!({"source_path":"source","destination_path":destination})
            )
            .await
            .is_error
        );
        assert_eq!(fs::read(root.path().join("source")).unwrap(), b"source");
        assert_eq!(
            fs::read(root.path().join("destination")).unwrap(),
            b"destination"
        );
        no_temporaries(root.path());
    }
    fs::create_dir(root.path().join("dir")).unwrap();
    let result = call(
        root.path(),
        "Move",
        json!({"source_path":"source","destination_path":"dir/file"}),
    )
    .await;
    assert!(!result.is_error, "{result:?}");
    assert!(
        call(root.path(), "Delete", json!({"path":"dir"}))
            .await
            .is_error
    );
    assert_eq!(fs::read(root.path().join("dir/file")).unwrap(), b"source");
    assert!(
        call(
            root.path(),
            "Move",
            json!({"source_path":"dir","destination_path":"dir/child"})
        )
        .await
        .is_error
    );
    let result = call(
        root.path(),
        "Move",
        json!({"source_path":"dir","destination_path":"renamed"}),
    )
    .await;
    assert!(!result.is_error, "{result:?}");
    assert!(
        !call(root.path(), "Delete", json!({"path":"renamed/file"}))
            .await
            .is_error
    );
    assert!(
        !call(root.path(), "Delete", json!({"path":"renamed"}))
            .await
            .is_error
    );
    no_temporaries(root.path());
}

#[tokio::test]
async fn links_special_files_and_edit_limits_fail_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    symlink(outside.path(), root.path().join("parent")).unwrap();
    symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
    fs::hard_link(outside.path().join("secret"), root.path().join("hard")).unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(root.path().join("socket")).unwrap();
    for path in ["parent/secret", "link", "hard", "socket", "../escape"] {
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
        assert!(
            call(root.path(), "Delete", json!({"path":path}))
                .await
                .is_error,
            "{path}"
        );
        assert!(
            call(
                root.path(),
                "Edit",
                json!({"file_path":path,"old_string":"secret","new_string":"bad"})
            )
            .await
            .is_error,
            "{path}"
        );
    }
    for path in ["parent/secret", "link", "socket", "../escape"] {
        assert!(
            call(
                root.path(),
                "Write",
                json!({"file_path":path,"content":"bad"})
            )
            .await
            .is_error,
            "{path}"
        );
    }
    fs::File::create(root.path().join("large"))
        .unwrap()
        .set_len(5 * 1024 * 1024 + 1)
        .unwrap();
    assert!(
        call(
            root.path(),
            "Edit",
            json!({"file_path":"large","old_string":"x","new_string":"y"})
        )
        .await
        .is_error
    );
    assert_eq!(fs::read(outside.path().join("secret")).unwrap(), b"secret");
    assert!(
        fs::symlink_metadata(root.path().join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    no_temporaries(root.path());
}
