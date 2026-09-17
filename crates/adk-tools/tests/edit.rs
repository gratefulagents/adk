#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::*;
use adk_tools::{Config, Features, Registry};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    sync::Arc,
};
fn registry(access: AccessMode) -> Registry {
    Registry::build(
        &Config {
            access,
            features: Features::Strict(["Edit".into()].into()),
            ..Default::default()
        },
        [],
    )
    .unwrap()
}
fn context(root: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "edit".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
async fn call(root: &Path, access: AccessMode, args: Value) -> ToolOutput {
    registry(access)
        .get("Edit")
        .unwrap()
        .execute(
            &context(root),
            ToolCall {
                id: "call".into(),
                name: "Edit".into(),
                arguments: args,
            },
        )
        .await
        .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("expected text"),
    }
}
#[tokio::test]
async fn exact_matching_preserves_binary_bytes_modes_and_line_endings() {
    for access in [AccessMode::WorkspaceWrite, AccessMode::FullAccess] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("a");
        fs::write(&path, b"\xff\xfealpha\r\nbeta\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
        let result = call(
            root.path(),
            access,
            json!({"file_path":"a","old_string":"alpha\r\n","new_string":"new"}),
        )
        .await;
        assert!(!result.is_error, "{result:?}");
        assert_eq!(fs::read(&path).unwrap(), b"\xff\xfenewbeta\n");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o751
        );
        assert!(text(&result).contains("@@ -1,2 +1,1 @@\n-��alpha\r\n-beta\n+��newbeta"));
        let before = fs::read(&path).unwrap();
        for args in [
            json!({"file_path":"a","old_string":"absent","new_string":"x"}),
            json!({"file_path":"a","old_string":"new","new_string":"new"}),
            json!({"file_path":"a","old_string":""}),
        ] {
            assert!(call(root.path(), access, args).await.is_error);
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }
}
#[tokio::test]
async fn multiple_matches_require_opt_in_and_diff_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("a");
    fs::write(&path, "foo\n".repeat(1000)).unwrap();
    assert!(
        call(
            root.path(),
            AccessMode::WorkspaceWrite,
            json!({"file_path":"a","old_string":"foo","new_string":"bar"})
        )
        .await
        .is_error
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "foo\n".repeat(1000));
    let result = call(
        root.path(),
        AccessMode::WorkspaceWrite,
        json!({"file_path":"a","old_string":"foo","new_string":"bar","replace_all":true}),
    )
    .await;
    assert!(!result.is_error);
    assert!(text(&result).contains("Successfully replaced 1000 occurrences"));
    assert!(text(&result).ends_with("... [diff truncated]"));
    assert!(text(&result).len() < 8500);
    assert_eq!(fs::read_to_string(&path).unwrap(), "bar\n".repeat(1000));
}
#[tokio::test]
async fn confinement_hardlinks_file_limits_and_cancellation_preserve_content() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
    fs::hard_link(outside.path().join("secret"), root.path().join("hard")).unwrap();
    let file = fs::File::create(root.path().join("large")).unwrap();
    file.set_len(5 * 1024 * 1024 + 1).unwrap();
    for path in [
        root.path().join("link"),
        root.path().join("hard"),
        root.path().join("large"),
        outside.path().join("secret"),
    ] {
        assert!(
            call(
                root.path(),
                AccessMode::WorkspaceWrite,
                json!({"file_path":path,"old_string":"secret","new_string":"bad"})
            )
            .await
            .is_error
        );
    }
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "secret"
    );
    assert_eq!(registry(AccessMode::ReadOnly).names().count(), 0);
    let token = Arc::new(adk_runtime::CancellationToken::new());
    token.cancel();
    let mut ctx = context(root.path());
    ctx.operation.cancellation = token;
    assert!(
        registry(AccessMode::WorkspaceWrite)
            .get("Edit")
            .unwrap()
            .execute(
                &ctx,
                ToolCall {
                    id: "call".into(),
                    name: "Edit".into(),
                    arguments: json!({"file_path":"hard","old_string":"secret","new_string":"bad"})
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "secret"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
}
