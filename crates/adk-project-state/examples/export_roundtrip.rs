use adk_project_state::*;
use serde_json::json;
use std::{fs, path::PathBuf};

struct FixtureEmbedder;
impl Embedder for FixtureEmbedder {
    fn model(&self) -> &str {
        "rust-fixture-model"
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.25, 0.5, 1.0]).collect())
    }
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: export_roundtrip OUTPUT_DIR"),
    );
    for sqlite in [false, true] {
        let directory = out.join(if sqlite { "sqlite" } else { "filesystem" });
        fs::create_dir_all(&directory)?;
        let options = StoreOptions {
            project_id: "rust-roundtrip".into(),
            actor: "rust-agent".into(),
            run_id: "rust-run".into(),
            ..Default::default()
        };
        let store = if sqlite {
            ProjectStore::sqlite(SQLiteOptions {
                path: directory.join("state.db"),
                store: options,
                ..Default::default()
            })?
        } else {
            ProjectStore::filesystem(FilesystemOptions {
                state_dir: directory.join("state"),
                store: options,
                ..Default::default()
            })?
        };
        let first = store.create_task(CreateTaskInput {
            title: "Foundation".into(),
            task_type: "feature".into(),
            priority: 1,
            metadata: json!({"large": 9007199254740993u64}),
            ..Default::default()
        })?;
        let second = store.create_task(CreateTaskInput {
            title: "Follow-up".into(),
            ..Default::default()
        })?;
        store.add_dependency(&second.id, &first.id)?;
        store.remove_dependency(&second.id, &first.id)?;
        store.add_dependency(&second.id, &first.id)?;
        store.update_task(
            &first.id,
            TaskPatch {
                labels: vec!["rust".into()],
                replace_labels: true,
                ..Default::default()
            },
        )?;
        store.add_comment(&first.id, "reviewer", "Reviewed")?;
        store.claim_task(&first.id, "")?;
        store.close_task(&first.id, "Complete")?;
        let memory = store.upsert_memory(UpsertMemoryInput {
            kind: "pinned".into(),
            content: "Durable Rust memory".into(),
            tags: vec!["rust".into()],
            ..Default::default()
        })?;
        store.upsert_memory(UpsertMemoryInput {
            id: memory.id,
            kind: "pinned".into(),
            content: "Durable Rust memory updated".into(),
            tags: vec!["rust".into()],
            ..Default::default()
        })?;
        let deleted = store.upsert_memory(UpsertMemoryInput {
            content: "Deleted memory".into(),
            ..Default::default()
        })?;
        store.delete_memory(&deleted.id)?;
        store.save_session_summary(SessionSummary {
            summary: "Foundation complete".into(),
            task_ids: vec![first.id],
            ..Default::default()
        })?;
        store.search_with_embeddings(
            MemoryFilter {
                query: "Rust".into(),
                ..Default::default()
            },
            &FixtureEmbedder,
            HybridConfig::default(),
        )?;
        let expected = json!({"tasks": store.list_tasks()?, "memories": store.list_memories(MemoryFilter::default())?, "sessions": store.list_session_summaries(0)?, "ready": store.ready_tasks(TaskFilter::default())?, "prime":store.prime_context(PrimeOptions::default())?});
        fs::write(
            directory.join("expected.json"),
            serde_json::to_vec_pretty(&expected)?,
        )?;
    }
    println!("Exported Rust filesystem and SQLite roundtrip data");
    Ok(())
}
