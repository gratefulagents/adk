//! Lazy models.dev lookup with caller-owned transport and cache configuration.
//! This catalog is distinct from authenticated provider `/models` metadata.

pub use adk_codec::modelsdev::{Catalog, ModelLimits, parse_catalog};
use adk_core::{Context, Error, ErrorCategory};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::{io::AsyncWriteExt, sync::Mutex};

pub const CATALOG_URL: &str = "https://models.dev/api.json";
const MAX_CATALOG_BYTES: usize = 64 << 20;

#[derive(Clone)]
pub struct Options {
    pub url: String,
    pub cache_path: Option<PathBuf>,
    pub ttl: Duration,
    pub client: Option<reqwest::Client>,
}
impl Default for Options {
    fn default() -> Self {
        let cache = match std::env::consts::OS {
            "windows" => std::env::var_os("LocalAppData").map(PathBuf::from),
            "macos" => {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"))
            }
            _ => std::env::var_os("XDG_CACHE_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .filter(|home| !home.is_empty())
                        .map(|home| PathBuf::from(home).join(".cache"))
                })
                .filter(|path| path.is_absolute()),
        };
        Self {
            url: CATALOG_URL.into(),
            cache_path: cache.map(|path| path.join("gratefulagents/modelsdev.json")),
            ttl: Duration::from_secs(24 * 60 * 60),
            client: None,
        }
    }
}

#[derive(Default)]
struct Cache {
    catalog: Option<Arc<Catalog>>,
    fetched_at: Option<OffsetDateTime>,
    fresh_since: Option<Instant>,
}

pub struct Resolver {
    options: Options,
    client: reqwest::Client,
    cache: Mutex<Cache>,
}

#[derive(Serialize, Deserialize)]
struct DiskCache {
    fetched_at: String,
    catalog: Catalog,
}

impl Resolver {
    /// Construction performs no network or filesystem access.
    pub fn new(options: Options) -> Result<Self, Error> {
        let client = match &options.client {
            Some(client) => client.clone(),
            None => reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|_| {
                    Error::new(
                        ErrorCategory::Provider,
                        "cannot initialize models.dev transport",
                    )
                })?,
        };
        Ok(Self {
            options,
            client,
            cache: Mutex::new(Cache::default()),
        })
    }

    /// Failed refreshes (including cancellation) serve the previous catalog.
    /// Fresh cache hits do not consult the caller's cancellation or deadline.
    pub async fn lookup(
        &self,
        context: &Context,
        provider: &str,
        model: &str,
    ) -> Option<ModelLimits> {
        let catalog = self.catalog_snapshot(context).await?;
        adk_codec::modelsdev::lookup(&catalog, provider, model)
    }

    /// Preserve the SDK's signed, wrapping threshold arithmetic.
    pub async fn compaction_thresholds(
        &self,
        context: &Context,
        provider: &str,
        model: &str,
    ) -> Option<(i64, i64)> {
        let limits = self.lookup(context, provider, model).await?;
        Some((
            limits.context_tokens.wrapping_mul(9) / 10,
            limits.context_tokens / 2,
        ))
    }

    pub async fn resolve_thresholds(
        &self,
        context: &Context,
        default_provider: &str,
        model: &str,
    ) -> Option<(i64, i64)> {
        let provider = model
            .trim()
            .split_once('/')
            .filter(|(prefix, _)| !prefix.is_empty())
            .map_or(default_provider, |(prefix, _)| prefix);
        self.compaction_thresholds(context, provider, model).await
    }

    async fn catalog_snapshot(&self, context: &Context) -> Option<Arc<Catalog>> {
        let mut cache = self.cache.lock().await;
        if cache.catalog.is_none() {
            if let Some(loaded) = self.load_disk().await {
                *cache = loaded;
            }
        }
        let fresh = cache.fresh_since.map_or_else(
            || {
                cache.fetched_at.is_some_and(|at| {
                    (OffsetDateTime::now_utc() - at).whole_nanoseconds()
                        < self.options.ttl.as_nanos() as i128
                })
            },
            |at| at.elapsed() < self.options.ttl,
        );
        if cache.catalog.is_some() && fresh {
            return cache.catalog.clone();
        }
        if let Some(catalog) = self.fetch(context).await {
            let fetched_at = OffsetDateTime::now_utc();
            let fresh_since = Instant::now();
            self.save_disk(&catalog, fetched_at).await;
            *cache = Cache {
                catalog: Some(Arc::new(catalog)),
                fetched_at: Some(fetched_at),
                fresh_since: Some(fresh_since),
            };
        }
        cache.catalog.clone()
    }

    async fn fetch(&self, context: &Context) -> Option<Catalog> {
        let mut response = crate::active(
            context,
            self.client
                .get(&self.options.url)
                .header("Accept", "application/json")
                .send(),
        )
        .await
        .ok()?
        .ok()?;
        if response.status() != reqwest::StatusCode::OK {
            return None;
        }
        let mut bytes = Vec::new();
        while bytes.len() < MAX_CATALOG_BYTES {
            let Some(chunk) = crate::active(context, response.chunk()).await.ok()?.ok()? else {
                break;
            };
            bytes.extend_from_slice(&chunk[..chunk.len().min(MAX_CATALOG_BYTES - bytes.len())]);
        }
        parse_catalog(&bytes).ok()
    }

    async fn load_disk(&self) -> Option<Cache> {
        let path = self
            .options
            .cache_path
            .as_ref()
            .filter(|path| !path.as_os_str().is_empty())?;
        let bytes = tokio::fs::read(path).await.ok()?;
        let disk: DiskCache = serde_json::from_slice(&bytes).ok()?;
        if disk.catalog.is_empty() {
            return None;
        }
        let fetched_at = OffsetDateTime::parse(&disk.fetched_at, &Rfc3339).ok()?;
        Some(Cache {
            catalog: Some(Arc::new(disk.catalog)),
            fetched_at: Some(fetched_at),
            fresh_since: None,
        })
    }

    async fn save_disk(&self, catalog: &Catalog, fetched_at: OffsetDateTime) {
        let Some(path) = self
            .options
            .cache_path
            .as_ref()
            .filter(|path| !path.as_os_str().is_empty())
        else {
            return;
        };
        let Ok(fetched_at) = fetched_at.format(&Rfc3339) else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&DiskCache {
            fetched_at,
            catalog: catalog.clone(),
        }) else {
            return;
        };
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            let mut directories = tokio::fs::DirBuilder::new();
            directories.recursive(true);
            #[cfg(unix)]
            directories.mode(0o755);
            if directories.create(parent).await.is_err() {
                return;
            }
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o644);
        if let Ok(mut file) = options.open(path).await {
            let _ = file.write_all(&bytes).await;
        }
    }
}

#[cfg(feature = "runtime")]
pub struct CompactionResolver {
    pub resolver: Arc<Resolver>,
    pub default_provider: String,
}

#[cfg(feature = "runtime")]
impl adk_runtime::CompactionModelResolver for CompactionResolver {
    fn thresholds<'a>(
        &'a self,
        context: &'a Context,
        model: &'a str,
    ) -> adk_core::BoxFuture<'a, Result<Option<(u64, u64)>, Error>> {
        Box::pin(async move {
            let result = crate::active(
                context,
                self.resolver
                    .resolve_thresholds(context, &self.default_provider, model),
            )
            .await?;
            // The native runner cannot accept negative thresholds from signed overflow.
            Ok(result.and_then(|(trigger, target)| {
                Some((trigger.try_into().ok()?, target.try_into().ok()?))
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_ttl_uses_monotonic_time_not_wall_clock() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let resolver = Resolver::new(Options {
            url: format!("http://{}", listener.local_addr().unwrap()),
            cache_path: None,
            ..Options::default()
        })
        .unwrap();
        let catalog =
            parse_catalog(br#"{"p":{"models":{"m":{"limit":{"context":100}}}}}"#).unwrap();
        *resolver.cache.lock().await = Cache {
            catalog: Some(Arc::new(catalog)),
            fetched_at: Some(OffsetDateTime::UNIX_EPOCH),
            fresh_since: Some(Instant::now()),
        };
        let context = Context {
            run_id: "monotonic".into(),
            deadline: None,
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
        };
        let result =
            tokio::time::timeout(Duration::from_secs(1), resolver.lookup(&context, "p", "m"))
                .await
                .unwrap();
        assert_eq!(result.unwrap().context_tokens, 100);
    }
}
