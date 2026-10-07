use crate::{
    auth::Session,
    metadata::{ModelMetadata, fetch_model_metadata_by_id, metadata_key},
};
use adk_core::{BoxFuture, Context, Error};
use adk_runtime::{CompactionModelResolver, compaction::LocalCompactionPolicy};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Mutex as AsyncMutex, time::Instant};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetadataCompactionWarning {
    FetchFailed,
    MissingModel(String),
}

#[derive(Default)]
struct Cache {
    catalog: Option<BTreeMap<String, ModelMetadata>>,
    last_attempt: Option<Instant>,
}

/// Explicit-session, lazy metadata lookup. Successful catalogs live as long as this resolver.
pub struct MetadataCompactionResolver {
    session: Arc<Session>,
    cache: AsyncMutex<Cache>,
    warnings: Mutex<BTreeSet<MetadataCompactionWarning>>,
}
impl MetadataCompactionResolver {
    pub fn new(session: Arc<Session>) -> Self {
        Self {
            session,
            cache: AsyncMutex::new(Cache::default()),
            warnings: Mutex::new(BTreeSet::new()),
        }
    }
    pub fn warnings(&self) -> Vec<MetadataCompactionWarning> {
        self.warnings.lock().unwrap().iter().cloned().collect()
    }
    /// Return cached metadata, or `None` on a missing model or failed fetch.
    /// Cached results ignore caller cancellation/deadline. Failed fetches, including
    /// cancelled fetches, return `None` and enter the retry cooldown; see `warnings`.
    /// Unlike runtime `thresholds`, this does not apply static defaults or a 15-second budget.
    pub async fn lookup(
        &self,
        context: &Context,
        model: &str,
    ) -> Result<Option<ModelMetadata>, Error> {
        let mut cache = self.cache.lock().await;
        if cache.catalog.is_none()
            && cache
                .last_attempt
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
        {
            cache.last_attempt = Some(Instant::now());
            match fetch_model_metadata_by_id(context, &self.session).await {
                Ok(catalog) => cache.catalog = Some(catalog),
                Err(_) => {
                    self.warnings
                        .lock()
                        .unwrap()
                        .insert(MetadataCompactionWarning::FetchFailed);
                }
            }
        }
        let Some(catalog) = &cache.catalog else {
            return Ok(None);
        };
        let key = metadata_key(model);
        if let Some(metadata) = catalog.get(&key).or_else(|| {
            key.split_once('/')
                .and_then(|(_, name)| catalog.get(&metadata_key(name)))
        }) {
            return Ok(Some(metadata.clone()));
        }
        self.warnings
            .lock()
            .unwrap()
            .insert(MetadataCompactionWarning::MissingModel(key));
        Ok(None)
    }
}
impl CompactionModelResolver for MetadataCompactionResolver {
    fn thresholds<'a>(
        &'a self,
        context: &'a Context,
        model: &'a str,
    ) -> BoxFuture<'a, Result<Option<(u64, u64)>, Error>> {
        Box::pin(async move {
            context.check_active()?;
            let metadata = match tokio::time::timeout(
                Duration::from_secs(15),
                crate::active(context, self.lookup(context, model)),
            )
            .await
            {
                Ok(result) => result??,
                Err(_) => {
                    self.warnings
                        .lock()
                        .unwrap()
                        .insert(MetadataCompactionWarning::FetchFailed);
                    None
                }
            };
            context.check_active()?;
            if let Some(thresholds) = metadata.and_then(|metadata| metadata.compaction_defaults()) {
                return Ok(Some(thresholds));
            }
            let defaults = LocalCompactionPolicy::for_model(model);
            Ok(Some((defaults.trigger_tokens, defaults.target_tokens)))
        })
    }
}
