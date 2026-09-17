#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::*;
use adk_tools::{Config, Features, Registry};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};

fn registry() -> Registry {
    Registry::build(
        &Config {
            features: Features::Strict(
                ["ReadFile", "ListFiles", "Glob", "Grep"]
                    .map(String::from)
                    .into(),
            ),
            ..Default::default()
        },
        [],
    )
    .unwrap()
}
fn context(root: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "search-tests".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
async fn execute(root: &Path, name: &str, args: Value) -> Result<ToolOutput, Error> {
    registry()
        .get(name)
        .unwrap()
        .execute(
            &context(root),
            ToolCall {
                id: "call".into(),
                name: name.into(),
                arguments: args,
            },
        )
        .await
}
fn text(output: &ToolOutput) -> &str {
    assert!(!output.should_pause);
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("expected text"),
    }
}
async fn invoke(root: &Path, name: &str, args: Value) -> String {
    let output = execute(root, name, args).await.unwrap();
    assert!(!output.is_error, "{output:?}");
    text(&output).into()
}
async fn page(root: &Path, name: &str, mut args: Value) -> Value {
    args["output_format"] = json!("json");
    serde_json::from_str(&invoke(root, name, args).await).unwrap()
}
fn write(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[tokio::test]
async fn read_ranges_bounds_listing_absolute_paths_and_suggestions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "z.txt", "one\ntwo\nthree\n");
    write(root, "sub/z.txt", "sub");
    write(root, "large", "x".repeat(100100));
    assert_eq!(
        invoke(root, "list_files", json!({"limit":2})).await,
        "large\nsub/"
    );
    assert_eq!(
        invoke(root, "read_file", json!({"path":root.join("z.txt")})).await,
        "one\ntwo\nthree\n"
    );
    assert_eq!(
        invoke(
            root,
            "read_file",
            json!({"path":"z.txt","start_line":2,"end_line":2})
        )
        .await,
        "two"
    );
    assert_eq!(
        invoke(
            root,
            "read_file",
            json!({"path":"z.txt","start_line":3,"end_line":1})
        )
        .await,
        ""
    );
    assert_eq!(
        invoke(root, "read_file", json!({"path":"z.txt","start_line":99})).await,
        ""
    );
    let large = invoke(root, "read_file", json!({"path":"large"})).await;
    assert_eq!(large.len(), 100000 + "\n[output truncated]".len());
    assert!(large.ends_with("\n[output truncated]"));
    let missing = execute(root, "read_file", json!({"path":"missing/z.txt"}))
        .await
        .unwrap();
    assert!(missing.is_error);
    assert!(text(&missing).contains("sub/z.txt, z.txt"));
    let missing = execute(root, "read_file", json!({"path":"unknown"}))
        .await
        .unwrap();
    assert_eq!(
        text(&missing),
        "unknown: no such file — use glob to locate the file"
    );
    assert!(
        execute(root, "read_file", json!({"path":"../outside"}))
            .await
            .is_err()
    );
    assert!(execute(root, "list_files", json!(42)).await.is_err());
}

#[tokio::test]
async fn complete_deterministic_pages_and_query_bound_cursors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for n in 0..9 {
        write(root, &format!("{n:02}.txt"), "hit\nhit\n");
    }
    for (name, mode, total) in [
        ("glob", "matches", 9),
        ("grep", "matches", 18),
        ("grep", "files", 9),
        ("grep", "count", 9),
    ] {
        let mut args =
            json!({"pattern":if name=="glob" {"*.txt"} else {"hit"},"mode":mode,"limit":2});
        let mut records = Vec::new();
        let mut cursors = std::collections::BTreeSet::new();
        loop {
            let result = page(root, name, args.clone()).await;
            records.extend(result["matches"].as_array().unwrap().clone());
            if result["truncated"] == false {
                assert!(result.get("next_cursor").is_none());
                break;
            }
            let cursor = result["next_cursor"].as_str().unwrap();
            assert!(cursors.insert(cursor.to_owned()));
            args["cursor"] = json!(cursor);
            let mut changed = args.clone();
            changed["pattern"] = json!("other");
            let error = execute(root, name, changed).await.unwrap();
            assert!(error.is_error);
            assert_eq!(text(&error), "cursor does not match this search");
            args["limit"] = json!(3);
        }
        assert_eq!(records.len(), total);
        let unique: std::collections::BTreeSet<_> = records.iter().map(Value::to_string).collect();
        assert_eq!(unique.len(), total);
    }
    assert!(
        invoke(root, "glob", json!({"pattern":"*","limit":1}))
            .await
            .contains("[search_metadata ")
    );
    let empty = page(root, "glob", json!({"pattern":"*.none"})).await;
    assert_eq!(
        empty,
        json!({"matches":[],"truncated":false,"incomplete":false})
    );
}

#[tokio::test]
async fn includes_excludes_double_star_and_gitignore_parent_rules() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for name in [
        "root.go",
        "src/a.go",
        "src/b.go",
        "src/c.txt",
        "excluded/keep.go",
        "node_modules/n.go",
        "vendor/v.go",
    ] {
        write(root, name, "hit\n");
    }
    write(root, ".gitignore", "/excluded/\n!/excluded/keep.go\n");
    write(root, "src/.gitignore", "b.go\n");
    let filtered = page(
        root,
        "glob",
        json!({"pattern":"**/*.go","respect_gitignore":true}),
    )
    .await;
    assert_eq!(filtered["matches"], json!(["root.go", "src/a.go"]));
    let classes = page(
        root,
        "glob",
        json!({"pattern":"**","include":["src/[ab].go"],"exclude":["**/b.go"]}),
    )
    .await;
    assert_eq!(classes["matches"], json!(["src/a.go"]));
    let all = page(
        root,
        "glob",
        json!({"pattern":"**/*.go","skip_default_dirs":false}),
    )
    .await;
    assert_eq!(all["matches"].as_array().unwrap().len(), 6);
    let under = page(root, "glob", json!({"path":"src","pattern":"*.go"})).await;
    assert_eq!(under["matches"], json!(["src/a.go", "src/b.go"]));
    write(root, ".gitignore", "ignored\n!ignored/keep.go\n");
    write(root, "ignored/keep.go", "hit");
    let ignored = page(
        root,
        "glob",
        json!({"pattern":"**/keep.go","respect_gitignore":true}),
    )
    .await;
    assert_eq!(ignored["matches"], json!(["excluded/keep.go"]));
}

#[tokio::test]
async fn grep_context_counts_long_lines_and_omission_bounds() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "a.txt", "before\nhit\nafter\nhit\nend\n");
    let result = page(
        root,
        "grep",
        json!({"pattern":"hit","before_context":1,"after_context":1}),
    )
    .await;
    assert_eq!(
        result["matches"][0],
        json!({"path":"a.txt","line":2,"text":"hit","before":["before"],"after":["after"]})
    );
    assert_eq!(
        invoke(root, "grep", json!({"pattern":"hit","mode":"count"})).await,
        "a.txt: 2"
    );
    assert!(
        invoke(root, "grep", json!({"pattern":"hit","before_context":1}))
            .await
            .contains("\n--\n")
    );
    write(
        root,
        "long.txt",
        format!("hit\n{}\nhit\n", "x".repeat(2 << 20)),
    );
    let result = page(root, "grep", json!({"pattern":"hit","path":"long.txt"})).await;
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["omitted_files"], json!(["long.txt: line too long"]));
    assert_eq!(result["incomplete"], true);
    write(root, "bounded.txt", format!("hit{}\n", "€".repeat(200)));
    let result = page(root, "grep", json!({"pattern":"hit","path":"bounded.txt"})).await;
    assert!(
        result["matches"][0]["text"]
            .as_str()
            .unwrap()
            .ends_with("�...")
    );
    let outside = tempfile::NamedTempFile::new().unwrap();
    for n in 0..101 {
        fs::hard_link(outside.path(), root.join(format!("link{n:03}"))).unwrap();
    }
    let result = page(
        root,
        "grep",
        json!({"pattern":"secret","include":["link*"]}),
    )
    .await;
    assert_eq!(result["omitted_files"].as_array().unwrap().len(), 100);
    assert_eq!(result["omitted_files_truncated"], true);
}

#[tokio::test]
async fn confinement_refuses_symlinks_hardlinks_and_symlinked_ignore_files() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "secret", "hidden-secret\n");
    write(root, "local.txt", "public\n");
    write(outside.path(), "ignore", "*.txt\n");
    symlink(outside.path().join("secret"), root.join("escape")).unwrap();
    symlink(outside.path(), root.join("directory")).unwrap();
    symlink(outside.path().join("ignore"), root.join(".gitignore")).unwrap();
    fs::hard_link(outside.path().join("secret"), root.join("alias")).unwrap();
    for path in ["escape", "directory/secret", "alias"] {
        assert!(
            execute(root, "read_file", json!({"path":path}))
                .await
                .is_err(),
            "{path}"
        );
    }
    let result = page(root, "grep", json!({"pattern":"hidden-secret"})).await;
    assert!(result["matches"].as_array().unwrap().is_empty());
    assert_eq!(
        result["omitted_files"],
        json!(["alias: hard-linked file refused"])
    );
    let result = page(
        root,
        "glob",
        json!({"pattern":"*.txt","respect_gitignore":true}),
    )
    .await;
    assert_eq!(result["matches"], json!(["local.txt"]));
}

#[tokio::test]
async fn bounds_errors_and_cancellation_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for (name, input, message) in [
        (
            "glob",
            json!({"pattern":"*","limit":1001}),
            "limit must not exceed",
        ),
        ("glob", json!({"pattern":"["}), "invalid glob pattern"),
        (
            "glob",
            json!({"pattern":"*","cursor":"!"}),
            "invalid cursor",
        ),
        (
            "grep",
            json!({"pattern":"x","glob":"["}),
            "invalid glob filter",
        ),
        (
            "grep",
            json!({"pattern":"x","before_context":11}),
            "context values must not exceed",
        ),
        (
            "grep",
            json!({"pattern":"x","after_context":-1}),
            "context values must be non-negative",
        ),
        (
            "grep",
            json!({"pattern":"x","mode":"wrong"}),
            "mode must be",
        ),
        ("grep", json!({"pattern":"("}), "invalid regex"),
    ] {
        let result = execute(root, name, input).await.unwrap();
        assert!(result.is_error);
        assert!(text(&result).starts_with(message), "{result:?}");
    }
    write(root, ".gitignore", "ignore\n".repeat(10001));
    let result = execute(
        root,
        "glob",
        json!({"pattern":"*","respect_gitignore":true}),
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert!(text(&result).contains("cannot load more than 10000 rules"));
    let token = adk_runtime::CancellationToken::new();
    token.cancel();
    let mut ctx = context(root);
    ctx.operation.cancellation = Arc::new(token);
    let error = registry()
        .get("grep")
        .unwrap()
        .execute(
            &ctx,
            ToolCall {
                id: "cancel".into(),
                name: "grep".into(),
                arguments: json!({"pattern":"x"}),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Cancelled);
}

#[tokio::test]
async fn symlink_swap_during_read_cannot_escape() {
    use std::{
        os::unix::fs::symlink,
        sync::atomic::{AtomicBool, Ordering},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let outside = tempfile::tempdir().unwrap();
    write(root, "dir/value", "public");
    write(outside.path(), "value", "private-secret");
    let stop = Arc::new(AtomicBool::new(false));
    let child_stop = stop.clone();
    let root_path = root.to_path_buf();
    let outside_path = outside.path().to_path_buf();
    let thread = std::thread::spawn(move || {
        while !child_stop.load(Ordering::Acquire) {
            if fs::rename(root_path.join("dir"), root_path.join("saved")).is_ok() {
                let _ = symlink(&outside_path, root_path.join("dir"));
                let _ = fs::remove_file(root_path.join("dir"));
                let _ = fs::rename(root_path.join("saved"), root_path.join("dir"));
            }
        }
    });
    for _ in 0..200 {
        if let Ok(result) = execute(root, "read_file", json!({"path":"dir/value"})).await {
            assert!(!text(&result).contains("private-secret"));
        }
    }
    stop.store(true, Ordering::Release);
    thread.join().unwrap();
}

#[tokio::test]
async fn every_search_name_rejects_escape_and_pre_cancelled_execution() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "private.txt", "outside-secret");
    let registry = registry();
    for name in ["read_file", "list_files", "glob", "grep"] {
        let path = if name == "read_file" {
            outside.path().join("private.txt")
        } else {
            outside.path().to_path_buf()
        };
        let args = json!({"path":path,"pattern":"private"});
        let error = execute(root.path(), name, args.clone()).await.unwrap_err();
        assert!(
            error.to_string().contains("outside the workspace root"),
            "{name}: {error}"
        );
        assert!(!error.to_string().contains("outside-secret"));
        let cancel = Arc::new(adk_runtime::CancellationToken::new());
        cancel.cancel();
        let mut ctx = context(root.path());
        ctx.operation.cancellation = cancel;
        let error = registry
            .get(name)
            .unwrap()
            .execute(
                &ctx,
                ToolCall {
                    id: "cancel".into(),
                    name: name.into(),
                    arguments: args,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.info.category, ErrorCategory::Cancelled, "{name}");
    }
}
