//! Namespace-isolated process-local memory contract from `agentsdk/memory`.
//! Its any-tag, phrase-first lexical search differs intentionally from project-state recall.
use crate::{Error, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, sync::RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub namespace: String,
    pub content: String,
    #[serde(
        deserialize_with = "crate::types::null_vec",
        serialize_with = "serialize_tags"
    )]
    pub tags: Vec<String>,
    pub source_run: String,
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub similarity: f64,
    #[serde(serialize_with = "crate::types::go_time::serialize")]
    pub created_at: DateTime<Utc>,
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
}
pub trait Store: Send + Sync {
    fn store(
        &self,
        namespace: &str,
        content: &str,
        tags: &[String],
        source_run: &str,
        metadata: Value,
    ) -> Result<Memory>;
    fn search(
        &self,
        namespace: &str,
        query: &str,
        tags: &[String],
        limit: i32,
    ) -> Result<Vec<Memory>>;
    fn list(&self, namespace: &str, tags: &[String], limit: i32) -> Result<Vec<Memory>>;
    fn delete(&self, namespace: &str, id: Uuid) -> Result<()>;
}
#[derive(Default)]
pub struct InMemoryStore {
    memories: RwLock<Vec<Memory>>,
}
fn required(value: &str, name: &str) -> Result<()> {
    if value.is_empty() {
        Err(Error::Invalid(format!("{name} is required")))
    } else {
        Ok(())
    }
}
impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
    fn filtered(&self, namespace: &str, tags: &[String]) -> Result<Vec<Memory>> {
        required(namespace, "namespace")?;
        Ok(self
            .memories
            .read()
            .map_err(|_| Error::Poisoned)?
            .iter()
            .filter(|m| {
                m.namespace == namespace
                    && (tags.is_empty() || tags.iter().any(|t| m.tags.contains(t)))
            })
            .cloned()
            .collect())
    }
}
impl Store for InMemoryStore {
    fn store(
        &self,
        namespace: &str,
        content: &str,
        tags: &[String],
        source_run: &str,
        metadata: Value,
    ) -> Result<Memory> {
        required(namespace, "namespace")?;
        required(content, "content")?;
        let memory = Memory {
            id: Uuid::new_v4(),
            namespace: namespace.into(),
            content: content.into(),
            tags: tags.into(),
            source_run: source_run.into(),
            metadata: if metadata.is_null() {
                serde_json::json!({})
            } else {
                metadata
            },
            similarity: 0.0,
            created_at: Utc::now(),
        };
        self.memories
            .write()
            .map_err(|_| Error::Poisoned)?
            .push(memory.clone());
        Ok(memory)
    }
    fn search(
        &self,
        namespace: &str,
        query: &str,
        tags: &[String],
        limit: i32,
    ) -> Result<Vec<Memory>> {
        required(query, "query")?;
        let query = query.to_lowercase();
        let terms: BTreeSet<_> = query.split_whitespace().collect();
        let mut memories = self.filtered(namespace, tags)?;
        for m in &mut memories {
            let content = m.content.to_lowercase();
            let matched = terms.iter().filter(|t| content.contains(**t)).count();
            m.similarity = if content.contains(&query) {
                1.0
            } else {
                (0.25 * matched as f64).min(0.9)
            };
        }
        memories.retain(|m| m.similarity > 0.0);
        memories.sort_by(|a, b| {
            b.similarity
                .total_cmp(&a.similarity)
                .then(b.created_at.cmp(&a.created_at))
        });
        memories.truncate(if limit <= 0 { 10 } else { limit as usize });
        Ok(memories)
    }
    fn list(&self, namespace: &str, tags: &[String], limit: i32) -> Result<Vec<Memory>> {
        let mut memories = self.filtered(namespace, tags)?;
        memories.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        memories.truncate(if limit <= 0 { 50 } else { limit as usize });
        Ok(memories)
    }
    fn delete(&self, namespace: &str, id: Uuid) -> Result<()> {
        required(namespace, "namespace")?;
        let mut memories = self.memories.write().map_err(|_| Error::Poisoned)?;
        let position = memories
            .iter()
            .position(|m| m.namespace == namespace && m.id == id)
            .ok_or_else(|| Error::NotFound("memory".into()))?;
        memories.remove(position);
        Ok(())
    }
}
pub struct NoopEmbedder {
    pub dimension: usize,
}
impl Default for NoopEmbedder {
    fn default() -> Self {
        Self { dimension: 1536 }
    }
}
impl Embedder for NoopEmbedder {
    fn embed(&self, _: &str) -> Result<Vec<f32>> {
        Ok(vec![
            0.0;
            if self.dimension == 0 {
                1536
            } else {
                self.dimension
            }
        ])
    }
}
pub fn vector_literal(vector: &[f32]) -> String {
    format!(
        "[{}]",
        vector
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn serialize_tags<S: serde::Serializer>(
    tags: &[String],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    if tags.is_empty() {
        serializer.serialize_none()
    } else {
        tags.serialize(serializer)
    }
}
