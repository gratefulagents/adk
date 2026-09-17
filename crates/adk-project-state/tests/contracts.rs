use adk_project_state::*;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use tempfile::TempDir;

fn options() -> StoreOptions {
    StoreOptions {
        project_id: "test-project".into(),
        actor: "agent".into(),
        run_id: "run-1".into(),
        ..Default::default()
    }
}
fn open(path: &Path, sqlite: bool) -> ProjectStore {
    if sqlite {
        ProjectStore::sqlite(SQLiteOptions {
            path: path.join("state.db"),
            store: options(),
            ..Default::default()
        })
        .unwrap()
    } else {
        ProjectStore::filesystem(FilesystemOptions {
            state_dir: path.join("state"),
            store: options(),
            ..Default::default()
        })
        .unwrap()
    }
}
fn task(store: &ProjectStore, title: &str) -> Task {
    store
        .create_task(CreateTaskInput {
            title: title.into(),
            ..Default::default()
        })
        .unwrap()
}
fn memory(store: &ProjectStore, content: &str) -> Memory {
    store
        .upsert_memory(UpsertMemoryInput {
            content: content.into(),
            ..Default::default()
        })
        .unwrap()
}
fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/project-state")
        .join(path)
}
fn events() -> Vec<Event> {
    fs::read_to_string(fixture("baseline/events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn go_event_schemas_roundtrip_without_precision_loss() {
    for line in fs::read_to_string(fixture("baseline/events.jsonl"))
        .unwrap()
        .lines()
    {
        let expected: Value = serde_json::from_str(line).unwrap();
        let event: Event = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(event).unwrap(), expected);
    }
    let state = State::replay(&events()).unwrap();
    assert_eq!(
        state.tasks["task_000000000001"].metadata["large"].as_u64(),
        Some(9007199254740993)
    );
    let task: Task =
        serde_json::from_value(json!({"labels":null,"comments":null,"depends_on":null})).unwrap();
    assert!(task.labels.is_empty());
    let input: CreateTaskInput =
        serde_json::from_value(json!({"title":"hi","type":"bug"})).unwrap();
    assert_eq!(input.task_type, "bug");
    let input: TaskPatch =
        serde_json::from_value(json!({"title":"","priority":0,"replace_labels":true})).unwrap();
    assert_eq!(input.title, Some(String::new()));
    assert_eq!(input.priority, Some(0));
    assert!(input.replace_labels);
}

#[test]
fn go_filesystem_and_sqlite_fixtures_replay_and_accept_rust_writes() {
    let expected: Value =
        serde_json::from_slice(&fs::read(fixture("expected.json")).unwrap()).unwrap();
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let store = if sqlite {
            let path = temp.path().join("baseline.db");
            fs::copy(fixture("baseline.sqlite"), &path).unwrap();
            ProjectStore::sqlite(SQLiteOptions {
                path,
                store: StoreOptions {
                    project_id: "fixture-project".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap()
        } else {
            fs::copy(
                fixture("baseline/events.jsonl"),
                temp.path().join("events.jsonl"),
            )
            .unwrap();
            ProjectStore::filesystem(FilesystemOptions {
                state_dir: temp.path().into(),
                store: StoreOptions {
                    project_id: "fixture-project".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap()
        };
        assert_eq!(
            serde_json::to_value(store.list_tasks().unwrap()).unwrap(),
            expected["tasks"]
        );
        assert_eq!(
            serde_json::to_value(store.list_memories(MemoryFilter::default()).unwrap()).unwrap(),
            expected["memories"]
        );
        assert_eq!(
            serde_json::to_value(store.list_session_summaries(0).unwrap()).unwrap(),
            expected["sessions"]
        );
        assert_eq!(
            serde_json::to_value(store.ready_tasks(TaskFilter::default()).unwrap()).unwrap(),
            expected["ready"]
        );
        assert_eq!(
            serde_json::to_value(
                store
                    .search_memories(MemoryFilter {
                        query: "ownership missing".into(),
                        ..Default::default()
                    })
                    .unwrap()
            )
            .unwrap(),
            expected["recall"]
        );
        assert_eq!(
            store.prime_context(PrimeOptions::default()).unwrap(),
            fs::read_to_string(fixture("prime.txt")).unwrap().trim()
        );
        let before = store.events().unwrap().len();
        task(&store, "Rust appended");
        assert_eq!(store.events().unwrap().len(), before + 1);
        assert_eq!(
            State::replay(&store.events().unwrap()).unwrap().tasks.len(),
            3
        );
    }
}

#[test]
fn task_lifecycle_filters_and_session_contracts() {
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let store = open(temp.path(), sqlite);
        assert!(store.create_task(CreateTaskInput::default()).is_err());
        let a = store
            .create_task(CreateTaskInput {
                title: " A ".into(),
                task_type: "feat".into(),
                priority: 99,
                labels: vec!["Core".into(), "Core".into(), " ".into()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            (a.title.as_str(), a.task_type.as_str(), a.priority),
            ("A", "feature", 4)
        );
        assert_eq!(a.labels, vec!["Core"]);
        let b = task(&store, "B");
        store.add_dependency(&b.id, &a.id).unwrap();
        assert!(store.add_dependency(&a.id, &a.id).is_err());
        assert_eq!(store.get_task(&a.id).unwrap().blocks, vec![b.id.clone()]);
        assert_eq!(store.ready_tasks(TaskFilter::default()).unwrap().len(), 1);
        let c = store
            .create_task(CreateTaskInput {
                title: "C".into(),
                depends_on: vec!["unknown".into()],
                ..Default::default()
            })
            .unwrap();
        assert!(
            !store
                .ready_tasks(TaskFilter::default())
                .unwrap()
                .iter()
                .any(|t| t.id == c.id)
        );
        store.claim_task(&a.id, "").unwrap();
        assert_eq!(store.get_task(&a.id).unwrap().assignee, "agent");
        store.close_task(&a.id, " done ").unwrap();
        assert_eq!(
            store.get_task(&a.id).unwrap().comments[0].body,
            "Closed: done"
        );
        assert_eq!(
            store.ready_tasks(TaskFilter::default()).unwrap()[0].id,
            b.id
        );
        store
            .update_task(
                &b.id,
                TaskPatch {
                    assignee: Some("other".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(store.ready_tasks(TaskFilter::default()).unwrap().is_empty());
        assert_eq!(
            store
                .ready_tasks(TaskFilter {
                    include_assigned: true,
                    ..Default::default()
                })
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .ready_tasks(TaskFilter {
                    actor: "other".into(),
                    ..Default::default()
                })
                .unwrap()
                .len(),
            1
        );
        store
            .update_task(
                &a.id,
                TaskPatch {
                    status: Some("open".into()),
                    labels: vec![],
                    replace_labels: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(store.get_task(&a.id).unwrap().closed_at.is_none());
        assert!(store.get_task(&a.id).unwrap().labels.is_empty());
        store.remove_dependency(&b.id, &a.id).unwrap();
        assert!(store.get_task(&a.id).unwrap().blocks.is_empty());
        let s = store
            .save_session_summary(SessionSummary {
                summary: "ready".into(),
                task_ids: vec![a.id.clone(), a.id.clone()],
                ..Default::default()
            })
            .unwrap();
        let updated = store
            .save_session_summary(SessionSummary {
                id: s.id.clone(),
                summary: "updated".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(s.created_at, updated.created_at);
        assert_eq!(store.list_session_summaries(0).unwrap().len(), 1);
        let reopened = open(temp.path(), sqlite);
        assert_eq!(reopened.list_tasks().unwrap(), store.list_tasks().unwrap());
        assert_eq!(
            reopened.list_session_summaries(0).unwrap(),
            store.list_session_summaries(0).unwrap()
        );
    }
}

#[test]
fn memory_upsert_lexical_and_stats_contracts() {
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let store = open(temp.path(), sqlite);
        assert!(store.upsert_memory(UpsertMemoryInput::default()).is_err());
        assert!(store.search_memories(MemoryFilter::default()).is_err());
        let a = store
            .upsert_memory(UpsertMemoryInput {
                content: "Rust ownership".into(),
                kind: "pinned".into(),
                scope: "task".into(),
                tags: vec!["rust".into(), "core".into()],
                ..Default::default()
            })
            .unwrap();
        let b = memory(&store, "ownership newer");
        let hits = store
            .search_memories(MemoryFilter {
                query: "OWNERSHIP missing".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits[0].id, a.id);
        assert_eq!(hits[1].id, b.id);
        assert_eq!(
            store
                .list_memories(MemoryFilter {
                    tags: vec!["RUST".into(), "core".into()],
                    ..Default::default()
                })
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .list_memories(MemoryFilter {
                    tags: vec!["rust".into(), "absent".into()],
                    ..Default::default()
                })
                .unwrap()
                .is_empty()
        );
        let updated = store
            .upsert_memory(UpsertMemoryInput {
                id: a.id.clone(),
                content: "Replacement".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(a.created_at, updated.created_at);
        assert_eq!(updated.kind, "semantic");
        assert!(updated.tags.is_empty());
        let stats = store.memory_stats(MemoryFilter::default()).unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.by_kind["semantic"], 2);
        store.delete_memory(&a.id).unwrap();
        assert!(store.delete_memory(&a.id).is_err());
        assert_eq!(
            open(temp.path(), sqlite)
                .list_memories(MemoryFilter::default())
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .events()
                .unwrap()
                .iter()
                .any(|e| e.payload["content"] == "Rust ownership")
        );
    }
}

#[test]
fn separate_handles_do_not_lose_updates_or_initialization() {
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let initial = open(temp.path(), sqlite);
        let t = task(&initial, "shared");
        let barrier = Arc::new(Barrier::new(6));
        let handles: Vec<_> = (0..6)
            .map(|i| {
                let path = temp.path().to_owned();
                let id = t.id.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let s = open(&path, sqlite);
                    barrier.wait();
                    for j in 0..8 {
                        s.add_comment(&id, &format!("actor-{i}"), &format!("comment-{j}"))
                            .unwrap();
                        memory(&s, &format!("memory-{i}-{j}"));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let s = open(temp.path(), sqlite);
        assert_eq!(s.get_task(&t.id).unwrap().comments.len(), 48);
        assert_eq!(s.list_memories(MemoryFilter::default()).unwrap().len(), 48);
        let events = s.events().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.event_type == "project.initialized")
                .count(),
            1
        );
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.seq, i as i64 + 1);
        }
    }
}

#[test]
fn filesystem_tail_recovery_corruption_and_lock_timeout() {
    let temp = TempDir::new().unwrap();
    let store = open(temp.path(), false);
    let log = store.state_dir().join("events.jsonl");
    let initial = fs::read(&log).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"{\"seq\":")
        .unwrap();
    task(&store, "after torn tail");
    assert_eq!(store.list_tasks().unwrap().len(), 1);
    assert!(fs::read(&log).unwrap().starts_with(&initial));
    let mut valid = fs::read(&log).unwrap();
    valid.pop();
    fs::write(&log, &valid).unwrap();
    task(&store, "after unterminated record");
    assert_eq!(store.list_tasks().unwrap().len(), 2);
    let good = fs::read(&log).unwrap();
    fs::write(&log, [b"{broken}\n".as_slice(), good.as_slice()].concat()).unwrap();
    assert!(store.list_tasks().is_err());
    fs::write(&log, &good).unwrap();
    let lock = store.state_dir().join("locks/state.lock");
    fs::write(&lock, format!("pid={}\ntoken=other\n", std::process::id())).unwrap();
    assert!(matches!(
        ProjectStore::filesystem(FilesystemOptions {
            state_dir: store.state_dir().into(),
            store: options(),
            lock_timeout: Duration::from_millis(25)
        }),
        Err(Error::LockTimeout)
    ));
    assert!(lock.exists());
    fs::remove_file(lock).unwrap();
    assert!(
        ProjectStore::filesystem(FilesystemOptions {
            state_dir: store.state_dir().into(),
            store: StoreOptions {
                project_id: "wrong".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn owner_only_permissions_and_dead_owner_recovery() {
    use std::os::unix::fs::PermissionsExt;
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let store = open(temp.path(), sqlite);
        memory(&store, "private");
        if sqlite {
            assert_eq!(
                fs::metadata(temp.path().join("state.db"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        } else {
            for dir in ["", "indexes", "snapshots", "locks"] {
                assert_eq!(
                    fs::metadata(store.state_dir().join(dir))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o700
                );
            }
            for file in [
                "events.jsonl",
                "indexes/project.json",
                "indexes/tasks.json",
                "indexes/memories.json",
                "indexes/sessions.json",
            ] {
                assert_eq!(
                    fs::metadata(store.state_dir().join(file))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
            fs::write(
                store.state_dir().join("locks/state.lock"),
                "pid=2147483647\ntoken=dead\n",
            )
            .unwrap();
            task(&store, "recovered");
            assert!(!store.state_dir().join("locks/state.lock").exists());
        }
    }
}

#[test]
fn sqlite_schema_prefix_validation_and_project_isolation() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("shared.db");
    let make = |id: &str| {
        ProjectStore::sqlite(SQLiteOptions {
            path: path.clone(),
            store: StoreOptions {
                project_id: id.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
    };
    let a = make("a");
    let b = make("b");
    task(&a, "only a");
    assert!(b.list_tasks().unwrap().is_empty());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(projectstate_events)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        columns,
        vec![
            "project_id",
            "seq",
            "event_id",
            "run_id",
            "actor",
            "ts",
            "type",
            "payload"
        ]
    );
    assert!(
        ProjectStore::sqlite_connection(
            rusqlite::Connection::open_in_memory().unwrap(),
            "bad;DROP TABLE x;",
            options()
        )
        .is_err()
    );
    let custom = ProjectStore::sqlite_connection(
        rusqlite::Connection::open_in_memory().unwrap(),
        "custom_",
        options(),
    )
    .unwrap();
    assert!(custom.list_tasks().unwrap().is_empty());
}

struct TestEmbedder {
    calls: AtomicUsize,
    fail: bool,
}
impl Embedder for TestEmbedder {
    fn model(&self) -> &str {
        "test-embedding"
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(Error::Invalid("offline".into()));
        }
        Ok(texts
            .iter()
            .map(|s| {
                if s.contains("cat") || s.contains("feline") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }
}
#[test]
fn embedding_recall_is_explicit_cached_and_falls_back() {
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let s = open(temp.path(), sqlite);
        let a = memory(&s, "feline animal");
        memory(&s, "unrelated");
        let e = TestEmbedder {
            calls: AtomicUsize::new(0),
            fail: false,
        };
        let filter = MemoryFilter {
            query: "cat".into(),
            ..Default::default()
        };
        assert!(s.search_memories(filter.clone()).unwrap().is_empty());
        let hits = s
            .search_with_embeddings(filter.clone(), &e, HybridConfig::default())
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, a.id);
        assert_eq!(e.calls.load(Ordering::SeqCst), 2);
        s.search_with_embeddings(filter.clone(), &e, HybridConfig::default())
            .unwrap();
        assert_eq!(e.calls.load(Ordering::SeqCst), 3);
        let fail = TestEmbedder {
            calls: AtomicUsize::new(0),
            fail: true,
        };
        assert!(
            s.search_with_embeddings(filter, &fail, HybridConfig::default())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            s.search_with_embeddings(
                MemoryFilter {
                    query: "feline".into(),
                    ..Default::default()
                },
                &fail,
                HybridConfig::default()
            )
            .unwrap()
            .len(),
            1
        );
        s.delete_memory(&a.id).unwrap();
        assert!(
            s.search_with_embeddings(
                MemoryFilter {
                    query: "cat".into(),
                    ..Default::default()
                },
                &e,
                HybridConfig::default()
            )
            .unwrap()
            .is_empty()
        );
    }
}

#[test]
fn hybrid_boosts_do_not_manufacture_relevance() {
    let pinned = Memory {
        id: "pinned".into(),
        kind: "pinned".into(),
        content: "unrelated".into(),
        updated_at: Utc::now(),
        ..Default::default()
    };
    assert!(
        recall::rank_hybrid(
            "search",
            vec![pinned],
            &[],
            &BTreeMap::new(),
            HybridConfig::default(),
            Utc::now()
        )
        .is_empty()
    );
    assert_eq!(recall::cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    assert_eq!(recall::cosine_similarity(&[0.0], &[1.0]), 0.0);
    assert_eq!(recall::cosine_similarity(&[f32::NAN], &[1.0]), 0.0);
}

struct PrivacyHooks {
    after: AtomicUsize,
}
impl StateHooks for PrivacyHooks {
    fn prepare_memory(&self, input: &mut UpsertMemoryInput) -> Result<()> {
        input.content = input.content.replace("SECRET", "[redacted]");
        Ok(())
    }
    fn before_write(&self, event: &Event) -> Result<()> {
        if event.payload["title"] == "denied" {
            Err(Error::Denied("test policy".into()))
        } else {
            Ok(())
        }
    }
    fn after_write(&self, _: &Event) {
        self.after.fetch_add(1, Ordering::SeqCst);
    }
    fn before_embed(&self, _: &[String]) -> Result<()> {
        Err(Error::Denied("no external disclosure".into()))
    }
}
#[test]
fn hooks_redact_before_log_and_deny_before_provider() {
    let temp = TempDir::new().unwrap();
    let hook = Arc::new(PrivacyHooks {
        after: AtomicUsize::new(0),
    });
    let s = open(temp.path(), false).with_hooks(hook.clone());
    assert!(
        s.create_task(CreateTaskInput {
            title: "denied".into(),
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(s.events().unwrap().len(), 1);
    assert_eq!(memory(&s, "SECRET value").content, "[redacted] value");
    assert!(
        !fs::read_to_string(s.state_dir().join("events.jsonl"))
            .unwrap()
            .contains("SECRET")
    );
    assert_eq!(hook.after.load(Ordering::SeqCst), 1);
    let embedder = TestEmbedder {
        calls: AtomicUsize::new(0),
        fail: false,
    };
    assert!(matches!(
        s.search_with_embeddings(
            MemoryFilter {
                query: "value".into(),
                ..Default::default()
            },
            &embedder,
            HybridConfig::default()
        ),
        Err(Error::Denied(_))
    ));
    assert_eq!(embedder.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn retention_and_purge_remove_history_not_just_live_entries() {
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let s = open(temp.path(), sqlite);
        let pinned = s
            .upsert_memory(UpsertMemoryInput {
                kind: "pinned".into(),
                content: "keep pinned".into(),
                ..Default::default()
            })
            .unwrap();
        let old = memory(&s, "old secret");
        let recent = memory(&s, "new secret");
        let removed = s
            .retain_memories(RetentionPolicy {
                max_count: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(removed, vec![old.id.clone()]);
        assert_eq!(s.list_memories(MemoryFilter::default()).unwrap().len(), 2);
        assert!(
            s.events()
                .unwrap()
                .iter()
                .any(|e| e.payload["content"] == "old secret")
        );
        assert_eq!(s.purge_memories(&[old.id.clone()]).unwrap(), 2);
        assert!(
            !s.events()
                .unwrap()
                .iter()
                .any(|e| e.payload["content"] == "old secret")
        );
        s.retain_memories(RetentionPolicy {
            before: Some(Utc::now() + ChronoDuration::seconds(1)),
            purge_history: true,
            ..Default::default()
        })
        .unwrap();
        let remaining = s.list_memories(MemoryFilter::default()).unwrap();
        assert_eq!(remaining[0].id, pinned.id);
        assert!(
            !s.events()
                .unwrap()
                .iter()
                .any(|e| e.payload["id"] == recent.id)
        );
        let events = s.events().unwrap();
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.seq, i as i64 + 1);
        }
        assert_eq!(
            open(temp.path(), sqlite)
                .list_memories(MemoryFilter::default())
                .unwrap(),
            remaining
        );
    }
}

#[test]
fn namespace_memory_schema_and_any_tag_phrase_first_search() {
    use adk_project_state::memory::{InMemoryStore, Store};
    let fixtures: Vec<adk_project_state::memory::Memory> =
        serde_json::from_slice(&fs::read(fixture("namespace-memory.json")).unwrap()).unwrap();
    for f in &fixtures {
        assert_eq!(
            *f,
            serde_json::from_value(serde_json::to_value(f).unwrap()).unwrap()
        );
    }
    let s = InMemoryStore::new();
    for m in &fixtures {
        s.store(
            &m.namespace,
            &m.content,
            &m.tags,
            &m.source_run,
            m.metadata.clone(),
        )
        .unwrap();
    }
    s.store("other", "alpha beta", &[], "", Value::Null)
        .unwrap();
    let hits = s
        .search("project-a", "alpha beta", &["one".into(), "two".into()], 10)
        .unwrap();
    assert_eq!(
        hits.iter().map(|m| m.similarity).collect::<Vec<_>>(),
        vec![1.0, 0.5]
    );
    assert_eq!(hits[0].content, "alpha beta");
    assert!(s.delete("other", hits[0].id).is_err());
    s.delete("project-a", hits[0].id).unwrap();
    assert_eq!(s.list("project-a", &[], 0).unwrap().len(), 1);
    assert!(s.store("", "value", &[], "", Value::Null).is_err());
    let empty = s
        .store("empty-tags", "value", &[], "", Value::Null)
        .unwrap();
    assert!(serde_json::to_value(&empty).unwrap()["tags"].is_null());
    assert_eq!(
        serde_json::from_value::<adk_project_state::memory::Memory>(
            serde_json::to_value(empty).unwrap()
        )
        .unwrap()
        .tags,
        Vec::<String>::new()
    );
}

#[test]
fn child_process_writer() {
    let Ok(path) = std::env::var("ADK_PROJECT_STATE_CHILD_PATH") else {
        return;
    };
    let sqlite = std::env::var("ADK_PROJECT_STATE_CHILD_SQLITE").unwrap() == "true";
    let store = open(Path::new(&path), sqlite);
    for n in 0..10 {
        task(&store, &format!("{}-{n}", std::process::id()));
    }
}

#[test]
fn independent_processes_recover_one_abandoned_lock_without_lost_events() {
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let store = open(temp.path(), sqlite);
        if !sqlite {
            fs::write(
                store.state_dir().join("locks/state.lock"),
                "pid=2147483647\ntoken=abandoned\n",
            )
            .unwrap();
        }
        let mut children: Vec<_> = (0..4)
            .map(|_| {
                std::process::Command::new(std::env::args_os().next().unwrap())
                    .args(["--exact", "child_process_writer"])
                    .env("ADK_PROJECT_STATE_CHILD_PATH", temp.path())
                    .env("ADK_PROJECT_STATE_CHILD_SQLITE", sqlite.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let events = store.events().unwrap();
        assert_eq!(events.len(), 41);
        assert_eq!(store.list_tasks().unwrap().len(), 40);
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.seq, i as i64 + 1);
        }
    }
}

#[cfg(unix)]
#[test]
fn existing_permissions_are_tightened_and_symlinks_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = TempDir::new().unwrap();
    let store = open(temp.path(), false);
    let root = store.state_dir().to_path_buf();
    for path in [&root, &root.join("indexes")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o777)).unwrap();
    }
    fs::set_permissions(root.join("events.jsonl"), fs::Permissions::from_mode(0o666)).unwrap();
    drop(store);
    drop(open(temp.path(), false));
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(root.join("indexes"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(root.join("events.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let secret = temp.path().join("untouched");
    fs::write(&secret, b"do not open").unwrap();
    fs::remove_file(root.join("events.jsonl")).unwrap();
    symlink(&secret, root.join("events.jsonl")).unwrap();
    assert!(
        ProjectStore::filesystem(FilesystemOptions {
            state_dir: root.clone(),
            store: options(),
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(fs::read(&secret).unwrap(), b"do not open");
    symlink(&secret, temp.path().join("unsafe.db")).unwrap();
    assert!(
        ProjectStore::sqlite(SQLiteOptions {
            path: temp.path().join("unsafe.db"),
            store: options(),
            ..Default::default()
        })
        .is_err()
    );
    symlink(&root, temp.path().join("alias")).unwrap();
    assert!(
        ProjectStore::filesystem(FilesystemOptions {
            state_dir: temp.path().join("alias"),
            store: options(),
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn cache_cleanup_failure_does_not_commit_delete_and_reopen_reconciles_orphans() {
    let temp = TempDir::new().unwrap();
    let s = open(temp.path(), false);
    let m = memory(&s, "feline secret");
    let e = TestEmbedder {
        calls: AtomicUsize::new(0),
        fail: false,
    };
    s.search_with_embeddings(
        MemoryFilter {
            query: "cat".into(),
            ..Default::default()
        },
        &e,
        HybridConfig::default(),
    )
    .unwrap();
    let cache = s.state_dir().join("indexes/embeddings.json");
    let original = fs::read(&cache).unwrap();
    fs::write(&cache, b"invalid JSON").unwrap();
    assert!(s.delete_memory(&m.id).is_err());
    assert_eq!(s.list_memories(MemoryFilter::default()).unwrap().len(), 1);
    assert!(s.purge_memories(std::slice::from_ref(&m.id)).is_err());
    assert_eq!(s.list_memories(MemoryFilter::default()).unwrap().len(), 1);
    fs::write(&cache, &original).unwrap();
    s.delete_memory(&m.id).unwrap();
    // Simulate a pre-upgrade crash after a tombstone but before pruning its cache.
    fs::write(&cache, &original).unwrap();
    drop(s);
    let reopened = open(temp.path(), false);
    assert!(
        reopened
            .list_memories(MemoryFilter::default())
            .unwrap()
            .is_empty()
    );
    let value: Value = serde_json::from_slice(&fs::read(cache).unwrap()).unwrap();
    assert!(value["vectors"].as_object().unwrap().is_empty());
}

struct GoFixtureEmbedder {
    calls: AtomicUsize,
}
impl Embedder for GoFixtureEmbedder {
    fn model(&self) -> &str {
        "fixture-model"
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(texts
            .iter()
            .map(|s| {
                if s.contains("ownership") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }
}
#[test]
fn go_embedding_json_and_little_endian_sqlite_cache_are_reused() {
    for sqlite in [false, true] {
        let temp = TempDir::new().unwrap();
        let opts = StoreOptions {
            project_id: "fixture-project".into(),
            ..Default::default()
        };
        let store = if sqlite {
            let path = temp.path().join("go.db");
            fs::copy(fixture("baseline.sqlite"), &path).unwrap();
            ProjectStore::sqlite(SQLiteOptions {
                path,
                store: opts,
                ..Default::default()
            })
            .unwrap()
        } else {
            fs::create_dir(temp.path().join("indexes")).unwrap();
            fs::copy(
                fixture("baseline/events.jsonl"),
                temp.path().join("events.jsonl"),
            )
            .unwrap();
            fs::copy(
                fixture("baseline/indexes/embeddings.json"),
                temp.path().join("indexes/embeddings.json"),
            )
            .unwrap();
            ProjectStore::filesystem(FilesystemOptions {
                state_dir: temp.path().into(),
                store: opts,
                ..Default::default()
            })
            .unwrap()
        };
        let embedder = GoFixtureEmbedder {
            calls: AtomicUsize::new(0),
        };
        let hits = store
            .search_with_embeddings(
                MemoryFilter {
                    query: "ownership".into(),
                    ..Default::default()
                },
                &embedder,
                HybridConfig::default(),
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "pinned");
        assert_eq!(
            embedder.calls.load(Ordering::SeqCst),
            1,
            "Go vectors should be reused without backfill"
        );
    }
}
