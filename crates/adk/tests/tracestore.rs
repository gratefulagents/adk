#![cfg(all(feature = "observability", target_os = "linux"))]

use adk::tracestore::*;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, symlink},
};

#[test]
fn defaults_errors_and_store_trait_are_explicit() {
    let limits = Limits::default();
    assert_eq!(
        (
            limits.event_bytes,
            limits.append_file_bytes,
            limits.write_file_bytes,
            limits.rotations
        ),
        (1 << 20, 64 << 20, 16 << 20, 4)
    );
    assert_eq!(
        StoreError::EventTooLarge.to_string(),
        "trace event exceeds per-event byte limit"
    );
    assert_eq!(
        StoreError::CategoryFull.to_string(),
        "trace category exceeds storage quota"
    );
    assert_eq!(
        StoreError::FileTooLarge.to_string(),
        "trace file exceeds per-file byte limit"
    );
    let root = tempfile::tempdir().unwrap();
    let store: Box<dyn TraceStore> = Box::new(FilesystemTraceStore::new(root.path()).unwrap());
    let run = store
        .create_run_dir("run", &RunMetadata::default())
        .unwrap();
    store.append_trace("run", " calls ", b"{}").unwrap();
    assert_eq!(fs::read(run.join("calls.jsonl")).unwrap(), b"{}\n");
    store.write_file("run", "a/../result", b"safe").unwrap();
    assert_eq!(fs::read(run.join("result")).unwrap(), b"safe");
    assert!(
        store
            .create_run_dir(" spaced ", &RunMetadata::default())
            .is_err()
    );
    assert!(matches!(
        store.write_score(
            "run",
            &Score {
                metrics: ScoreMetrics {
                    accuracy: f64::NAN,
                    ..Default::default()
                },
                ..Default::default()
            }
        ),
        Err(StoreError::InvalidScore)
    ));
    assert!(!run.join("score.json").exists());
}

#[test]
fn pinned_go_store_fixture_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let store = FilesystemTraceStore::new(root.path()).unwrap();
    let metadata = RunMetadata {
        run_id: "run".into(),
        candidate_id: "candidate".into(),
        started_at: "2025-01-02T03:04:05.123Z".parse().unwrap(),
        ..Default::default()
    };
    let run = store.create_run_dir("run", &metadata).unwrap();
    store
        .append_trace(
            "run",
            "tool_calls",
            br#"{"schema_version":2,"run_id":"run","type":"tool_start"}"#,
        )
        .unwrap();
    store
        .write_file("run", "artifacts/result.txt", b"result\n")
        .unwrap();
    store
        .write_score(
            "run",
            &Score {
                task_id: "task".into(),
                candidate_id: "candidate".into(),
                success: true,
                metrics: ScoreMetrics {
                    accuracy: 1.0,
                    tokens_used: 42,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    store.update_metadata_mode("run", "plan").unwrap();
    store
        .update_metadata_finished_at("run", "2025-01-02T03:04:06.123Z".parse().unwrap())
        .unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-store.json")).unwrap();
    for name in ["metadata.json", "score.json"] {
        let actual: Value = serde_json::from_slice(&fs::read(run.join(name)).unwrap()).unwrap();
        // JSON numeric spelling is not part of the document contract.
        assert_eq!(
            serde_json::from_value::<serde_json::Map<String, Value>>(actual.clone())
                .unwrap()
                .len(),
            fixture[name].as_object().unwrap().len()
        );
        if name == "score.json" {
            assert_eq!(
                serde_json::from_value::<Score>(actual).unwrap(),
                serde_json::from_value::<Score>(fixture[name].clone()).unwrap()
            );
        } else {
            assert_eq!(
                serde_json::from_value::<RunMetadata>(actual).unwrap(),
                serde_json::from_value::<RunMetadata>(fixture[name].clone()).unwrap()
            );
        }
    }
    for name in ["tool_calls.jsonl", "artifacts/result.txt"] {
        assert_eq!(
            fs::read_to_string(run.join(name)).unwrap(),
            fixture[name].as_str().unwrap()
        );
        assert_eq!(fs::metadata(run.join(name)).unwrap().mode() & 0o777, 0o600);
    }
    assert_eq!(fs::metadata(&run).unwrap().mode() & 0o777, 0o700);
    store.close();
    store.close();
    assert!(matches!(
        store.list_runs(&RunFilter::default()),
        Err(StoreError::Closed)
    ));
    let store = FilesystemTraceStore::new(root.path()).unwrap();
    store.append_trace("run", "tool_calls", b"{}").unwrap();
    assert_eq!(store.run_dir("run").unwrap(), run);
    assert_eq!(
        store
            .list_runs(&RunFilter {
                candidate_id: "candidate".into(),
                since: Some("2025-01-02T04:04:05.123+01:00".parse().unwrap())
            })
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .list_runs(&RunFilter {
                candidate_id: "other".into(),
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_runs(&RunFilter {
                since: Some("2025-01-02T03:04:05.124Z".parse().unwrap()),
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
}

#[test]
fn quotas_rotate_oldest_to_numbered_file_and_preserve_active() {
    let root = tempfile::tempdir().unwrap();
    let store = FilesystemTraceStore::with_limits(
        root.path(),
        Limits {
            event_bytes: 4,
            append_file_bytes: 4,
            write_file_bytes: 4,
            rotations: 1,
        },
    )
    .unwrap();
    let run = store
        .create_run_dir("run", &RunMetadata::default())
        .unwrap();
    store.append_trace("run", "calls", b"one").unwrap();
    store.append_trace("run", "calls", b"two").unwrap();
    assert_eq!(fs::read(run.join("calls.jsonl.001")).unwrap(), b"one\n");
    assert_eq!(fs::read(run.join("calls.jsonl")).unwrap(), b"two\n");
    assert!(matches!(
        store.append_trace("run", "calls", b"end"),
        Err(StoreError::CategoryFull)
    ));
    assert!(matches!(
        store.append_trace("run", "calls", b"long"),
        Err(StoreError::EventTooLarge)
    ));
    assert!(matches!(
        store.write_file("run", "artifact", b"12345"),
        Err(StoreError::FileTooLarge)
    ));
    assert_eq!(fs::read(run.join("calls.jsonl")).unwrap(), b"two\n");
}

#[test]
fn confinement_symlinks_hardlinks_and_root_replacement() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("root");
    let outside = parent.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let store = FilesystemTraceStore::new(&root).unwrap();
    let run = store
        .create_run_dir("run", &RunMetadata::default())
        .unwrap();
    for id in ["", "..", "../outside", "/absolute", "a\\b"] {
        assert!(store.create_run_dir(id, &RunMetadata::default()).is_err());
    }
    for path in ["", "../outside", "/absolute", "a/../../outside", "a\\b"] {
        assert!(store.write_file("run", path, b"bad").is_err());
    }
    symlink(&outside, run.join("escape")).unwrap();
    assert!(store.write_file("run", "escape/secret", b"bad").is_err());
    fs::write(outside.join("secret"), b"unchanged").unwrap();
    symlink(outside.join("secret"), run.join("link.jsonl")).unwrap();
    assert!(store.append_trace("run", "link", b"bad").is_err());
    fs::hard_link(outside.join("secret"), run.join("hard.jsonl")).unwrap();
    store.append_trace("run", "hard", b"safe").unwrap();
    store
        .write_file("run", "link.jsonl", b"replacement")
        .unwrap();
    assert_eq!(fs::read(outside.join("secret")).unwrap(), b"unchanged");
    fs::rename(&root, parent.path().join("moved")).unwrap();
    fs::create_dir_all(root.join("traces/run")).unwrap();
    assert!(store.run_dir("run").is_err());
    store.write_file("run", "pinned", b"yes").unwrap();
    assert!(parent.path().join("moved/traces/run/pinned").exists());
    assert!(!root.join("traces/run/pinned").exists());
}

#[test]
fn list_ignores_corrupt_and_symlink_runs_and_metadata_updates_are_atomic() {
    let root = tempfile::tempdir().unwrap();
    let store = FilesystemTraceStore::new(root.path()).unwrap();
    let run = store
        .create_run_dir(
            "good",
            &RunMetadata {
                run_id: "good".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let bad = store
        .create_run_dir("bad", &RunMetadata::default())
        .unwrap();
    fs::write(bad.join("metadata.json"), b"not json").unwrap();
    symlink(&run, root.path().join("traces/link")).unwrap();
    let old = fs::File::open(run.join("metadata.json")).unwrap();
    store.update_metadata_mode("good", "plan").unwrap();
    assert_ne!(
        old.metadata().unwrap().ino(),
        fs::metadata(run.join("metadata.json")).unwrap().ino()
    );
    let runs = store.list_runs(&RunFilter::default()).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].mode, "plan");
    let value = serde_json::to_value(RunMetadata::default()).unwrap();
    assert_eq!(
        value,
        json!({"run_id":"", "started_at":"0001-01-01T00:00:00Z", "finished_at":"0001-01-01T00:00:00Z"})
    );
}
