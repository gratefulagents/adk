#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::*;
use adk_tools::{Config, Features, Registry};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{FileTypeExt, PermissionsExt, symlink},
    path::Path,
    sync::Arc,
};

fn context(root: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "patch".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
fn registry(access: AccessMode) -> Registry {
    Registry::build(
        &Config {
            access,
            features: Features::Strict(["ApplyPatch".into()].into()),
            ..Default::default()
        },
        [],
    )
    .unwrap()
}
async fn call(root: &Path, arguments: Value) -> ToolOutput {
    registry(AccessMode::WorkspaceWrite)
        .get("ApplyPatch")
        .unwrap()
        .execute(
            &context(root),
            ToolCall {
                id: "test".into(),
                name: "ApplyPatch".into(),
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
#[derive(Deserialize, Default)]
#[serde(default)]
struct File {
    content: String,
    mode: u32,
    kind: String,
    target: String,
}
#[derive(Deserialize)]
struct Expected {
    content: String,
    is_error: bool,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    arguments: Value,
    files: BTreeMap<String, File>,
    error_only: bool,
    expected: Expected,
    tree: Value,
}
fn setup(root: &Path, files: &BTreeMap<String, File>) {
    for (name, file) in files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mode = if file.mode == 0 { 0o644 } else { file.mode };
        match file.kind.as_str() {
            "symlink" | "hardlink" => continue,
            "directory" => {
                fs::create_dir_all(path).unwrap();
                continue;
            }
            "fifo" => {
                rustix::fs::mknodat(
                    rustix::fs::CWD,
                    &path,
                    rustix::fs::FileType::Fifo,
                    rustix::fs::Mode::from_raw_mode(mode),
                    0,
                )
                .unwrap();
            }
            _ => {
                fs::write(
                    &path,
                    if file.kind == "invalid_utf8" {
                        &[255]
                    } else {
                        file.content.as_bytes()
                    },
                )
                .unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            }
        }
    }
    for (name, file) in files {
        if file.kind == "symlink" {
            symlink(&file.target, root.join(name)).unwrap();
        }
        if file.kind == "hardlink" {
            fs::hard_link(root.join(&file.target), root.join(name)).unwrap();
        }
    }
}
fn tree(root: &Path) -> Value {
    fn walk(root: &Path, dir: &Path, out: &mut serde_json::Map<String, Value>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            let kind = meta.file_type();
            let mut value = json!({"mode":meta.permissions().mode() & 0o777});
            if kind.is_file() {
                value["kind"] = json!("file");
                let bytes = fs::read(&path).unwrap();
                if !bytes.is_empty() {
                    value["data"] = json!(STANDARD.encode(bytes));
                }
            } else if kind.is_dir() {
                value["kind"] = json!("directory");
                walk(root, &path, out);
            } else if kind.is_symlink() {
                value["kind"] = json!("symlink");
                value["target"] = json!(fs::read_link(&path).unwrap());
            } else if kind.is_fifo() {
                value["kind"] = json!("fifo");
            } else {
                panic!("unexpected entry");
            }
            out.insert(
                path.strip_prefix(root).unwrap().to_str().unwrap().into(),
                value,
            );
        }
    }
    let mut out = serde_json::Map::new();
    walk(root, root, &mut out);
    Value::Object(out)
}
#[tokio::test]
async fn pinned_sdk_fixtures() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../../../fixtures/tools/patch.json")).unwrap();
    for case in cases {
        let root = tempfile::tempdir().unwrap();
        setup(root.path(), &case.files);
        let output = call(root.path(), case.arguments).await;
        assert_eq!(
            output.is_error,
            case.expected.is_error,
            "{}: {}",
            case.name,
            text(&output)
        );
        if !case.error_only {
            assert_eq!(
                text(&output).replace(root.path().to_str().unwrap(), "<root>"),
                case.expected.content,
                "{}",
                case.name
            );
        }
        assert_eq!(tree(root.path()), case.tree, "{}", case.name);
    }
}
#[tokio::test]
async fn confinement_and_no_partial_validation_changes() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join("good"), "old\n").unwrap();
    fs::write(outside.path().join("secret"), "old\n").unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
    fs::hard_link(outside.path().join("secret"), root.path().join("hard")).unwrap();
    for path in ["../secret", "escape/secret", "link", "hard"] {
        for dry in [true, false] {
            let before = tree(root.path());
            let output = call(root.path(),json!({"patch":format!("*** Begin Patch\n*** Update File: good\n-old\n+new\n*** Update File: {path}\n-old\n+new\n*** End Patch\n"),"dry_run":dry})).await;
            assert!(output.is_error, "{path}: {}", text(&output));
            assert_eq!(tree(root.path()), before);
            assert_eq!(
                fs::read_to_string(outside.path().join("secret")).unwrap(),
                "old\n"
            );
        }
    }
}
#[tokio::test]
async fn limits_and_bounded_output() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a"), "old\n").unwrap();
    let cases = [
        ("x".repeat(1024 * 1024 + 1), "patch is too large"),
        (
            format!(
                "*** Begin Patch\n*** Add File: new\n{}*** End Patch\n",
                "+x\n".repeat(16385)
            ),
            "too many lines",
        ),
        (
            format!(
                "*** Begin Patch\n*** Update File: a\n{}*** End Patch\n",
                "@@\n-old\n+new\n".repeat(257)
            ),
            "too many hunks",
        ),
        (
            format!(
                "*** Begin Patch\n{}*** End Patch\n",
                (0..129)
                    .map(|i| format!("*** Add File: new{i}\n"))
                    .collect::<String>()
            ),
            "too many files",
        ),
    ];
    for (patch, error) in cases {
        let before = tree(root.path());
        let output = call(root.path(), json!({"patch":patch})).await;
        assert!(output.is_error);
        assert!(text(&output).contains(error), "{}", text(&output));
        assert_eq!(tree(root.path()), before);
    }
    let patch = format!(
        "*** Begin Patch\n*** Add File: new\n{}*** End Patch\n",
        "+abc\n".repeat(3000)
    );
    let output = call(root.path(), json!({"patch":patch,"dry_run":true})).await;
    assert!(!output.is_error, "{}", text(&output));
    let result: Value = serde_json::from_str(text(&output)).unwrap();
    assert!(
        result["diff"]
            .as_str()
            .unwrap()
            .ends_with("\n... [diff truncated]")
    );
    assert!(result["diff"].as_str().unwrap().len() < 8250);
    assert!(!root.path().join("new").exists());
    let patch = format!(
        "index {}\ndiff --git a/a b/a\nnew mode 100755\n",
        "界".repeat(3000)
    );
    let output = call(root.path(), json!({"patch":patch,"dry_run":true})).await;
    assert!(!output.is_error);
    assert!(text(&output).contains("\\ufffd\\ufffd\\n... [diff truncated]"));
    let patch = format!(
        "*** Begin Patch\n{}*** End Patch\n",
        (0..128)
            .map(|i| format!(
                "*** Add File: {}/{}/{}{i:03}\n",
                "x".repeat(200),
                "y".repeat(200),
                "z".repeat(100)
            ))
            .collect::<String>()
    );
    let output = call(root.path(), json!({"patch":patch,"dry_run":true})).await;
    assert!(output.is_error);
    assert!(text(&output).starts_with("Patch result is too large"));
    let huge = "x".repeat(5 * 1024 * 1024 + 1);
    fs::write(root.path().join("large"), huge).unwrap();
    let output = call(
        root.path(),
        json!({"patch":"*** Begin Patch\n*** Delete File: large\n*** End Patch"}),
    )
    .await;
    assert!(output.is_error);
    assert!(text(&output).contains("is too large"));
}
#[tokio::test]
async fn aggregate_limit_and_linear_repeated_prefix_matching() {
    let root = tempfile::tempdir().unwrap();
    let data = "a\n".repeat(5 * 1024 * 1024 / 2);
    fs::write(root.path().join("a"), &data).unwrap();
    let patch = format!(
        "*** Begin Patch\n*** Update File: a\n@@\n{}-b\n+c\n*** End Patch\n",
        " a\n".repeat(16382)
    );
    let start = std::time::Instant::now();
    let output = call(root.path(), json!({"patch":patch,"dry_run":true})).await;
    assert!(output.is_error);
    assert!(text(&output).contains("does not match"));
    assert!(start.elapsed().as_secs() < 15);
    let mut patch = "*** Begin Patch\n".to_owned();
    for i in 0..13 {
        fs::write(root.path().join(format!("file{i}")), &data).unwrap();
        patch.push_str(&format!("*** Delete File: file{i}\n"));
    }
    patch.push_str("*** End Patch\n");
    let output = call(root.path(), json!({"patch":patch,"dry_run":true})).await;
    assert!(output.is_error);
    assert!(text(&output).contains("aggregate limit"));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 14);
}
#[tokio::test]
async fn full_access_remains_confined_and_cancellation_precedes_mutation() {
    assert!(registry(AccessMode::ReadOnly).get("ApplyPatch").is_none());
    let root = tempfile::tempdir().unwrap();
    let registry = registry(AccessMode::FullAccess);
    let tool = registry.get("ApplyPatch").unwrap();
    let mut ctx = context(root.path());
    let output = tool.execute(&ctx,ToolCall {id:"test".into(),name:"ApplyPatch".into(),arguments:json!({"patch":"*** Begin Patch\n*** Add File: ../escape\n+x\n*** End Patch"})}).await.unwrap();
    assert!(output.is_error);
    ctx.operation.deadline = Some(std::time::Instant::now());
    let result = tool
        .execute(
            &ctx,
            ToolCall {
                id: "test".into(),
                name: "ApplyPatch".into(),
                arguments: json!({"patch":"*** Begin Patch\n*** Add File: new\n+x\n*** End Patch"}),
            },
        )
        .await;
    assert!(result.is_err());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
