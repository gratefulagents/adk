//! Lazy models.dev lookup with caller-owned transport and cache configuration.
//! This catalog is distinct from authenticated provider `/models` metadata.

pub use adk_codec::modelsdev::{Catalog, ModelLimits, parse_catalog};
use adk_core::{Context, Error, ErrorCategory};
use serde::Serialize;
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
pub use time::Duration as CacheTtl;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::{io::AsyncWriteExt, sync::Mutex};

pub const CATALOG_URL: &str = "https://models.dev/api.json";
const MAX_CATALOG_BYTES: usize = 64 << 20;
const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct Options {
    pub url: String,
    pub cache_path: Option<PathBuf>,
    pub ttl: CacheTtl,
    pub client: Option<reqwest::Client>,
}
impl Default for Options {
    fn default() -> Self {
        let cache = match std::env::consts::OS {
            "windows" => std::env::var_os("LocalAppData")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            "macos" => std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join("Library/Caches")),
            _ => match std::env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
                Some(value) => {
                    let path = PathBuf::from(value);
                    path.is_absolute().then_some(path)
                }
                None => std::env::var_os("HOME")
                    .filter(|home| !home.is_empty())
                    .map(|home| PathBuf::from(home).join(".cache")),
            },
        };
        Self {
            url: CATALOG_URL.into(),
            cache_path: cache.map(|path| clean_path(&path.join("gratefulagents/modelsdev.json"))),
            ttl: CacheTtl::hours(24),
            client: None,
        }
    }
}

fn clean_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if result.file_name().is_some_and(|name| name != "..") {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            component => result.push(component.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

#[derive(Default)]
struct Cache {
    catalog: Option<Arc<Catalog>>,
    fetched_at: Option<i128>,
    fresh_since: Option<Instant>,
}

pub struct Resolver {
    options: Options,
    client: reqwest::Client,
    cache: Mutex<Cache>,
}

#[derive(Serialize)]
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
                .timeout(DEFAULT_FETCH_TIMEOUT)
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
                    (OffsetDateTime::now_utc().unix_timestamp_nanos() - at)
                        .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                        < self.options.ttl.whole_nanoseconds()
                })
            },
            |at| (at.elapsed().as_nanos() as i128) < self.options.ttl.whole_nanoseconds(),
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
                fetched_at: Some(fetched_at.unix_timestamp_nanos()),
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
        let (catalog, timestamp) = adk_codec::modelsdev::parse_cached_catalog(&bytes).ok()?;
        let (utc, offset) = if timestamp.ends_with('Z') {
            (timestamp, 0_i128)
        } else {
            let zone = &timestamp[timestamp.len() - 6..];
            let minutes: i128 = zone[4..].parse().ok()?;
            let hours: i128 = zone[1..3].parse().ok()?;
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            (
                format!("{}Z", &timestamp[..timestamp.len() - 6]),
                sign * (hours * 3600 + minutes * 60),
            )
        };
        // Go accepts offsets beyond RFC3339's normal range and UTC years beyond9999.
        let fetched_at = OffsetDateTime::parse(&utc, &Rfc3339)
            .ok()?
            .unix_timestamp_nanos()
            - offset * 1_000_000_000;
        Some(Cache {
            catalog: Some(Arc::new(catalog)),
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
        let Ok(bytes) = adk_codec::snapshots::to_go_json(&DiskCache {
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
            if directories.create(clean_path(parent)).await.is_err() {
                return;
            }
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o644);
        if let Ok(mut file) = options.open(path).await {
            let _ = file.write_all(&bytes).await;
            // Tokio writes may still be pending when write_all returns.
            let _ = file.flush().await;
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
            fetched_at: Some(0),
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

    #[tokio::test]
    async fn disk_envelopes_match_pinned_go() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/modelsdev/observations.json"
        ))
        .unwrap();
        let path =
            std::env::temp_dir().join(format!("adk-modelsdev-envelope-{}", std::process::id()));
        let resolver = Resolver::new(Options {
            cache_path: Some(path.clone()),
            ..Options::default()
        })
        .unwrap();
        for case in fixture["cache"].as_array().unwrap() {
            tokio::fs::write(&path, case["input"].as_str().unwrap())
                .await
                .unwrap();
            let loaded = resolver.load_disk().await;
            if loaded.is_some() != case["ok"].as_bool().unwrap() {
                tokio::fs::remove_file(&path).await.unwrap();
                panic!("{case}");
            }
            if let Some(loaded) = loaded {
                let mut expected = case["catalog"].clone();
                for models in expected.as_object_mut().unwrap().values_mut() {
                    if models.is_null() {
                        *models = serde_json::json!({});
                    }
                }
                assert_eq!(
                    serde_json::to_value(loaded.catalog.unwrap().as_ref()).unwrap(),
                    expected,
                    "{case}"
                );
                assert_eq!(
                    loaded.fetched_at.unwrap(),
                    i128::from(case["seconds"].as_i64().unwrap()) * 1_000_000_000
                        + i128::from(case["nanos"].as_i64().unwrap()),
                    "{case}"
                );
            }
        }
        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn cache_writer_matches_go_escaping_and_file_permissions() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/modelsdev/observations.json"
        ))
        .unwrap();
        let path = std::env::temp_dir()
            .join(format!("adk-modelsdev-write-{}", std::process::id()))
            .join("catalog.json");
        let resolver = Resolver::new(Options {
            cache_path: Some(path.clone()),
            ..Options::default()
        })
        .unwrap();
        let catalog = serde_json::from_value(fixture["encoding_catalog"].clone()).unwrap();
        resolver
            .save_disk(
                &catalog,
                OffsetDateTime::parse("2026-01-02T03:04:05.123456Z", &Rfc3339).unwrap(),
            )
            .await;
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert_eq!(bytes, fixture["encoded_cache"].as_str().unwrap().as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o133,
                0
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o022,
                0
            );
        }
        let unclean = Resolver::new(Options {
            cache_path: Some(path.parent().unwrap().join("missing/../other.json")),
            ..Options::default()
        })
        .unwrap();
        unclean.save_disk(&catalog, OffsetDateTime::now_utc()).await;
        assert_eq!(
            path.parent().unwrap().join("other.json").exists(),
            fixture["unclean_cache_path"]["file_exists"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(
            path.parent().unwrap().join("missing").exists(),
            fixture["unclean_cache_path"]["intermediate_exists"]
                .as_bool()
                .unwrap()
        );
        tokio::fs::remove_dir_all(path.parent().unwrap())
            .await
            .unwrap();
    }

    #[test]
    fn constructor_defaults_match_go() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/modelsdev/observations.json"
        ))
        .unwrap();
        let options = Options::default();
        assert_eq!(options.url, fixture["defaults"]["url"]);
        assert_eq!(
            options.ttl.whole_nanoseconds(),
            i128::from(fixture["defaults"]["ttl"].as_i64().unwrap())
        );
        assert_eq!(
            DEFAULT_FETCH_TIMEOUT.as_nanos(),
            u128::from(fixture["defaults"]["timeout"].as_u64().unwrap())
        );
        assert!(options.client.is_none());
        let resolver = Resolver::new(Options {
            url: "not a URL".into(),
            cache_path: None,
            ..options
        })
        .unwrap();
        let cache = resolver.cache.try_lock().unwrap();
        assert_eq!(cache.catalog.is_none(), fixture["defaults"]["catalog_nil"]);
        assert_eq!(cache.fetched_at.is_none(), fixture["defaults"]["zero_time"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn default_cache_directories_match_go() {
        if let Ok(expected) = std::env::var("ADK_MODELSDEV_CACHE_PATH_EXPECTED") {
            let expected = (!expected.is_empty()).then(|| PathBuf::from(expected));
            assert_eq!(Options::default().cache_path, expected);
            return;
        }
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/modelsdev/observations.json"
        ))
        .unwrap();
        for case in fixture["cache_paths"].as_array().unwrap() {
            let output = std::process::Command::new(std::env::args_os().next().unwrap())
                .args([
                    "--exact",
                    "modelsdev::tests::default_cache_directories_match_go",
                    "--nocapture",
                ])
                .env("HOME", case["home"].as_str().unwrap())
                .env("XDG_CACHE_HOME", case["xdg"].as_str().unwrap())
                .env(
                    "ADK_MODELSDEV_CACHE_PATH_EXPECTED",
                    case["path"].as_str().unwrap(),
                )
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{case}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}
