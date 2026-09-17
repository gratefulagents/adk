use crate::{
    engine::{haystack, limit, matches_any, matches_labels, required},
    *,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};

pub trait LexicalRecall: Send + Sync {
    fn search_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>>;
}
impl LexicalRecall for ProjectStore {
    fn search_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>> {
        ProjectStore::search_memories(self, filter)
    }
}
pub trait Embedder: Send + Sync {
    fn model(&self) -> &str;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}
pub trait EmbeddingRecall: Send + Sync {
    fn search_with_embeddings(
        &self,
        filter: MemoryFilter,
        embedder: &dyn Embedder,
        config: HybridConfig,
    ) -> Result<Vec<Memory>>;
}
#[derive(Debug, Clone, Copy)]
pub struct HybridConfig {
    pub lexical_weight: f64,
    pub dense_weight: f64,
    pub pinned_boost: f64,
    pub recency_weight: f64,
    pub recency_half_life: Duration,
    pub min_score: f64,
}
impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            lexical_weight: 0.4,
            dense_weight: 0.6,
            pinned_boost: 0.15,
            recency_weight: 0.1,
            recency_half_life: Duration::from_secs(30 * 24 * 3600),
            min_score: 0.0,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EmbeddingRecord {
    pub hash: String,
    pub model: String,
    pub dims: usize,
    pub vector: Vec<f32>,
}
fn hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}
fn valid(vector: &[f32]) -> bool {
    !vector.is_empty() && vector.iter().all(|v| v.is_finite())
}
impl EmbeddingRecord {
    fn fresh(&self, memory: &Memory, model: &str) -> bool {
        self.model == model
            && self.hash == hash(&memory.content)
            && self.dims == self.vector.len()
            && valid(&self.vector)
    }
}
impl EmbeddingRecall for ProjectStore {
    fn search_with_embeddings(
        &self,
        filter: MemoryFilter,
        embedder: &dyn Embedder,
        config: HybridConfig,
    ) -> Result<Vec<Memory>> {
        required(&filter.query, "query")?;
        let texts = [filter.query.clone()];
        self.hooks.before_embed(&texts)?;
        let query = match embedder.embed(&texts) {
            Ok(mut vectors) if vectors.len() == 1 && valid(&vectors[0]) => vectors.remove(0),
            _ => return self.search_memories(filter),
        };
        let candidates: Vec<_> = self
            .state()?
            .sorted_memories()
            .into_iter()
            .filter(|m| {
                matches_any(&m.kind, &filter.kinds) && matches_labels(&m.tags, &filter.tags)
            })
            .collect();
        let cache = self
            .backend
            .transaction(|tx| tx.embeddings())
            .unwrap_or_default();
        let mut vectors = BTreeMap::new();
        let mut missing = Vec::new();
        for memory in &candidates {
            if let Some(record) = cache
                .get(&memory.id)
                .filter(|r| r.fresh(memory, embedder.model()))
            {
                vectors.insert(memory.id.clone(), record.vector.clone());
            } else {
                missing.push(memory);
            }
        }
        if !missing.is_empty() {
            let texts: Vec<_> = missing.iter().map(|m| m.content.clone()).collect();
            self.hooks.before_embed(&texts)?;
            if let Ok(fresh) = embedder.embed(&texts) {
                if fresh.len() == missing.len() {
                    let mut records = BTreeMap::new();
                    for (memory, vector) in missing.iter().zip(fresh) {
                        if valid(&vector) {
                            vectors.insert(memory.id.clone(), vector.clone());
                            records.insert(
                                memory.id.clone(),
                                EmbeddingRecord {
                                    hash: hash(&memory.content),
                                    model: embedder.model().into(),
                                    dims: vector.len(),
                                    vector,
                                },
                            );
                        }
                    }
                    // A delete or rewrite while the provider runs must not resurrect an orphan/stale cache entry.
                    self.backend.transaction(|tx| {
                        let current = State::replay(&tx.events()?)?;
                        records.retain(|id, r| {
                            current
                                .memories
                                .get(id)
                                .is_some_and(|m| r.fresh(m, embedder.model()))
                        });
                        tx.put_embeddings(&records)
                    })?;
                }
            }
        }
        let mut out = rank_hybrid(
            &filter.query,
            candidates,
            &query,
            &vectors,
            config,
            Utc::now(),
        );
        // Do not return a memory deleted or changed during an embedding request.
        let current = self.state()?;
        out.retain(|m| current.memories.get(&m.id).is_some_and(|now| now == m));
        limit(&mut out, filter.limit);
        Ok(out)
    }
}
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let (mut dot, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (&a, &b) in a.iter().zip(b) {
        dot += a as f64 * b as f64;
        aa += (a as f64).powi(2);
        bb += (b as f64).powi(2);
    }
    if aa == 0.0 || bb == 0.0 || !dot.is_finite() || !aa.is_finite() || !bb.is_finite() {
        0.0
    } else {
        dot / (aa.sqrt() * bb.sqrt())
    }
}
pub fn lexical_score(memory: &Memory, terms: &[&str]) -> f64 {
    let text = haystack(memory);
    terms
        .iter()
        .filter(|t| !t.is_empty())
        .map(|term| {
            let n = text.matches(term).count();
            if n == 0 { 0.0 } else { n as f64 + 2.0 }
        })
        .sum()
}
pub fn rank_hybrid(
    query: &str,
    candidates: Vec<Memory>,
    query_vector: &[f32],
    vectors: &BTreeMap<String, Vec<f32>>,
    mut cfg: HybridConfig,
    now: DateTime<Utc>,
) -> Vec<Memory> {
    if cfg.lexical_weight == 0.0 && cfg.dense_weight == 0.0 {
        cfg.lexical_weight = 0.4;
        cfg.dense_weight = 0.6;
    }
    if cfg.recency_half_life.is_zero() {
        cfg.recency_half_life = HybridConfig::default().recency_half_life;
    }
    let query = query.trim().to_lowercase();
    let terms: Vec<_> = query.split_whitespace().collect();
    let raw: Vec<_> = candidates
        .iter()
        .map(|m| lexical_score(m, &terms))
        .collect();
    let max_lex = raw.iter().copied().fold(0.0, f64::max);
    let mut scored: Vec<_> = candidates
        .into_iter()
        .zip(raw)
        .filter_map(|(m, raw)| {
            let lexical = if max_lex > 0.0 { raw / max_lex } else { 0.0 };
            let dense = vectors
                .get(&m.id)
                .map(|v| cosine_similarity(query_vector, v).max(0.0))
                .unwrap_or(0.0);
            if lexical == 0.0 && dense == 0.0 {
                return None;
            }
            let mut score = cfg.lexical_weight * lexical + cfg.dense_weight * dense;
            if m.kind == "pinned" {
                score += cfg.pinned_boost;
            }
            if cfg.recency_weight > 0.0 {
                let age = (now - m.updated_at).num_milliseconds().max(0) as f64 / 1000.0;
                score +=
                    cfg.recency_weight * 0.5f64.powf(age / cfg.recency_half_life.as_secs_f64());
            }
            (score.is_finite() && score >= cfg.min_score).then_some((m, score))
        })
        .collect();
    scored.sort_by(|(a, sa), (b, sb)| {
        sb.total_cmp(sa)
            .then(b.updated_at.cmp(&a.updated_at))
            .then(a.id.cmp(&b.id))
    });
    scored.into_iter().map(|(m, _)| m).collect()
}
