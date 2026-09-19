//! Pinned SDK native-image loading; no separate model call is performed.
use crate::{Capability, network, workspace::Workspace};
use adk_core::{
    BoxFuture, Content, Context, Error, ErrorCategory, Tool, ToolCall, ToolContext, ToolDefinition,
    ToolOutput,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io::Read, path::Path, path::PathBuf, sync::Arc, time::Duration};

const MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024;

/// Compatibility injection point. The pinned SDK retains analyzers but never calls
/// them: images are attached to the active conversation instead.
pub trait Analyzer: Send + Sync {
    fn analyze<'a>(
        &'a self,
        context: &'a Context,
        data: &'a [u8],
        media_type: &'a str,
        prompt: &'a str,
        detail: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>>;
}

#[derive(Default, Clone)]
pub struct Config {
    pub analyzer: Option<Arc<dyn Analyzer>>,
    pub allow_private_network_urls: bool,
    /// Only absolute image paths may use these host-managed directories.
    pub allowed_image_dirs: Vec<PathBuf>,
}

pub fn tool(config: Config) -> Arc<dyn Tool> {
    let definition = crate::capabilities()
        .iter()
        .find(|capability| capability.name == "AnalyzeImage")
        .and_then(|capability| capability.definition.clone())
        .expect("AnalyzeImage definition");
    Arc::new(AnalyzeImage { definition, config })
}

pub(crate) fn builtin(capability: &Capability, allow_private: bool) -> Option<Arc<dyn Tool>> {
    (capability.name == "AnalyzeImage").then(|| {
        tool(Config {
            allow_private_network_urls: allow_private,
            ..Default::default()
        })
    })
}

struct AnalyzeImage {
    definition: ToolDefinition,
    config: Config,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    image_path: String,
    url: String,
    prompt: String,
    detail_level: String,
}

fn image_mime(path: &Path, data: &[u8]) -> String {
    let extension = path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.rsplit_once('.'))
        .map(|(_, extension)| extension)
        .unwrap_or("");
    let mime = match extension.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ if data.starts_with(&[0xff, 0xd8]) => "image/jpeg",
        _ if data.starts_with(b"\x89PNG") => "image/png",
        _ if data.starts_with(b"GIF8") => "image/gif",
        _ if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") => "image/webp",
        _ => "application/octet-stream",
    };
    mime.into()
}

fn file_location(root: &Path, path: &Path) -> Result<(Workspace, PathBuf), String> {
    let workspace = Workspace::new(root).map_err(|e| e.to_string())?;
    // Workspace::relative trims input; the trailing dot preserves literal spaces
    // in SDK image filenames, which are not trimmed.
    let absolute = workspace.root.join(path).join(".");
    let relative = workspace
        .relative(&absolute.to_string_lossy())
        .map_err(|e| e.to_string())?;
    Ok((workspace, relative))
}

fn load_file(root: &Path, path: &str, allowed: &[PathBuf]) -> Result<(Vec<u8>, String), String> {
    let path = Path::new(path);
    let (workspace, relative) = match file_location(root, path) {
        Ok(location) => location,
        Err(error) => {
            if !path.is_absolute() {
                return Err(error);
            }
            allowed
                .iter()
                .filter_map(|root| {
                    let root = root.to_string_lossy();
                    let root = root.trim();
                    if root.is_empty() {
                        None
                    } else {
                        file_location(Path::new(root), path).ok()
                    }
                })
                .next()
                .ok_or(error)?
        }
    };
    let mut file = workspace
        .read_file(&relative)
        .map_err(|e| format!("opening file: {e}"))?;
    let size = file
        .metadata()
        .map_err(|e| format!("stat file: {e}"))?
        .len();
    if size > MAX_IMAGE_SIZE as u64 {
        return Err(format!(
            "image too large ({size} bytes, max {MAX_IMAGE_SIZE})"
        ));
    }
    let mut data = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_IMAGE_SIZE as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| format!("reading file: {e}"))?;
    if data.len() > MAX_IMAGE_SIZE {
        return Err(format!("image too large (> {MAX_IMAGE_SIZE} bytes)"));
    }
    let mime = image_mime(&relative, &data);
    Ok((data, mime))
}

async fn load_url(raw: &str, allow_private: bool) -> Result<(Vec<u8>, String), String> {
    let (mut url, mut addresses) = network::resolve(raw, allow_private).await?;
    let request = async {
        let mut referer = None;
        loop {
            let client =
                network::client(&url, &addresses).map_err(|e| format!("fetching image: {e}"))?;
            let mut request = client
                .get(url.clone())
                .header(reqwest::header::USER_AGENT, "gratefulagents-bot/1.0");
            if let Some(previous) = &referer {
                request = request.header(reqwest::header::REFERER, previous);
            }
            let mut response = request
                .send()
                .await
                .map_err(|e| format!("fetching image: {e}"))?;
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(location) = response.headers().get(reqwest::header::LOCATION)
            {
                let next = url
                    .join(
                        location
                            .to_str()
                            .map_err(|e| format!("fetching image: {e}"))?,
                    )
                    .map_err(|e| format!("fetching image: {e}"))?;
                let resolved = network::resolve(next.as_str(), allow_private)
                    .await
                    .map_err(|e| format!("fetching image: {e}"))?;
                referer = if url.scheme() == "https" && next.scheme() == "http" {
                    None
                } else {
                    Some(url.to_string())
                };
                (url, addresses) = resolved;
                continue;
            }
            if status >= 400 {
                return Err(format!("HTTP {status} fetching image"));
            }
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let mut data = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| format!("reading response: {e}"))?
            {
                if chunk.len() > MAX_IMAGE_SIZE - data.len() {
                    return Err(format!("image too large (> {MAX_IMAGE_SIZE} bytes)"));
                }
                data.extend_from_slice(&chunk);
            }
            let mime = if mime.is_empty() {
                image_mime(Path::new(""), &data)
            } else {
                mime
            };
            return Ok((data, mime));
        }
    };
    tokio::time::timeout(Duration::from_secs(15), request)
        .await
        .map_err(|_| "fetching image: request timed out".to_owned())?
}

impl AnalyzeImage {
    async fn load(
        &self,
        context: &ToolContext,
        mut arguments: Value,
    ) -> Result<ToolOutput, String> {
        if arguments.is_null() {
            arguments = json!({});
        }
        let fields = arguments
            .as_object_mut()
            .ok_or_else(|| "Invalid input: expected an object".to_owned())?;
        fields.retain(|_, value| !value.is_null());
        for name in ["image_path", "url", "prompt", "detail_level"] {
            let aliases: Vec<_> = fields
                .keys()
                .filter(|key| key.as_str() != name && key.eq_ignore_ascii_case(name))
                .cloned()
                .collect();
            for alias in aliases {
                let value = fields.remove(&alias).expect("existing field");
                fields.insert(name.into(), value);
            }
        }
        let input: Input =
            serde_json::from_value(arguments).map_err(|e| format!("Invalid input: {e}"))?;
        if input.prompt.is_empty() {
            return Err("prompt is required".into());
        }
        if input.image_path.is_empty() && input.url.is_empty() {
            return Err("either image_path or url is required".into());
        }
        let image = if !input.image_path.is_empty() {
            let root = context.work_dir.clone();
            let path = input.image_path;
            let allowed = self.config.allowed_image_dirs.clone();
            tokio::task::spawn_blocking(move || load_file(&root, &path, &allowed))
                .await
                .map_err(|e| format!("Failed to load image: {e}"))?
        } else {
            load_url(&input.url, self.config.allow_private_network_urls).await
        };
        let (data, media_type) = image.map_err(|e| format!("Failed to load image: {e}"))?;
        Ok(ToolOutput {
            content: vec![
                Content::Text { text: input.prompt },
                Content::Attachment {
                    media_type,
                    data: STANDARD.encode(data),
                    detail: if input.detail_level.trim().eq_ignore_ascii_case("low") {
                        "low"
                    } else {
                        "high"
                    }
                    .into(),
                },
            ],
            is_error: false,
            should_pause: false,
        })
    }
}

impl Tool for AnalyzeImage {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let deadline = async {
                match context.operation.deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                    None => std::future::pending().await,
                }
            };
            let result = tokio::select! {biased;
                _=context.operation.cancellation.cancelled()=>return Err(Error::new(ErrorCategory::Cancelled,"operation cancelled")),
                _=deadline=>return Err(Error::new(ErrorCategory::DeadlineExceeded,"deadline exceeded")),
                result=self.load(context,call.arguments)=>result,
            };
            context.operation.check_active()?;
            Ok(result.unwrap_or_else(|text| ToolOutput {
                content: vec![Content::Text { text }],
                is_error: true,
                should_pause: false,
            }))
        })
    }
}
