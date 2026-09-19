use crate::{Capability, html, network, search::go_string};
use adk_core::{
    BoxFuture, Content, Error, ErrorCategory, Tool, ToolCall, ToolContext, ToolDefinition,
    ToolOutput,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub(crate) fn builtin(capability: &Capability, allow_private: bool) -> Option<Arc<dyn Tool>> {
    (capability.name == "WebFetch").then(|| {
        Arc::new(Fetch {
            definition: capability.definition.clone().expect("fetch definition"),
            allow_private,
        }) as Arc<dyn Tool>
    })
}
struct Fetch {
    definition: ToolDefinition,
    allow_private: bool,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    url: String,
    max_length: i64,
    start_index: i64,
}
impl Fetch {
    async fn fetch(&self, mut arguments: Value) -> Result<String, String> {
        if arguments.is_null() {
            arguments = json!({});
        }
        if let Some(fields) = arguments.as_object_mut() {
            fields.retain(|_, value| !value.is_null());
        }
        let input: Input =
            serde_json::from_value(arguments).map_err(|e| format!("Invalid input: {e}"))?;
        if input.url.is_empty() {
            return Err("url is required".into());
        }
        let (mut url, mut addresses) = network::resolve(&input.url, self.allow_private).await?;
        let request = async {
            let mut referer = None;
            loop {
                let client =
                    network::client(&url, &addresses).map_err(|e| format!("Fetch failed: {e}"))?;
                let mut request = client
                    .get(url.clone())
                    .header(reqwest::header::USER_AGENT, "gratefulagents-bot/1.0");
                if let Some(previous) = &referer {
                    request = request.header(reqwest::header::REFERER, previous);
                }
                let mut response = request
                    .send()
                    .await
                    .map_err(|e| format!("Fetch failed: {e}"))?;
                let status = response.status();
                if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
                    && let Some(location) = response.headers().get(reqwest::header::LOCATION)
                {
                    let next = url
                        .join(
                            location
                                .to_str()
                                .map_err(|e| format!("Fetch failed: {e}"))?,
                        )
                        .map_err(|e| format!("Fetch failed: {e}"))?;
                    let resolved = network::resolve(next.as_str(), self.allow_private)
                        .await
                        .map_err(|e| format!("Fetch failed: {e}"))?;
                    referer = if url.scheme() == "https" && next.scheme() == "http" {
                        None
                    } else {
                        Some(url.to_string())
                    };
                    (url, addresses) = resolved;
                    continue;
                }
                if status.as_u16() >= 400 {
                    return Err(format!(
                        "HTTP {}: {} {}",
                        status.as_u16(),
                        status.as_u16(),
                        status.canonical_reason().unwrap_or("")
                    ));
                }
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|header| header.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                let mut bytes = Vec::new();
                while bytes.len() < 2 * 1024 * 1024 {
                    let Some(chunk) = response
                        .chunk()
                        .await
                        .map_err(|e| format!("Failed to read response: {e}"))?
                    else {
                        break;
                    };
                    let count = chunk.len().min(2 * 1024 * 1024 - bytes.len());
                    bytes.extend_from_slice(&chunk[..count]);
                }
                if content_type.contains("text/html") || content_type.contains("application/xhtml")
                {
                    bytes = html::to_text(&go_string(&bytes)).into_bytes();
                }
                if input.start_index > 0 {
                    let start = input.start_index as usize;
                    if start >= bytes.len() {
                        return Ok(format!(
                            "start_index {} exceeds content length {}",
                            input.start_index,
                            bytes.len()
                        ));
                    }
                    bytes.drain(..start);
                }
                let maximum = if input.max_length <= 0 {
                    50000
                } else {
                    input.max_length as usize
                };
                let truncated = bytes.len() > maximum;
                if truncated {
                    bytes.truncate(maximum);
                }
                let mut text = go_string(&bytes);
                if truncated {
                    text.push_str(&format!(
                        "\n\n--- Content truncated. Use start_index={} to continue reading. ---",
                        input.start_index.saturating_add(maximum as i64)
                    ));
                }
                return Ok(text);
            }
        };
        tokio::time::timeout(Duration::from_secs(15), request)
            .await
            .map_err(|_| "Fetch failed: request timed out".to_owned())?
    }
}
impl Tool for Fetch {
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
                result=self.fetch(call.arguments)=>result,
            };
            let (text, is_error) = match result {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            Ok(ToolOutput {
                content: vec![Content::Text { text }],
                is_error,
                should_pause: false,
            })
        })
    }
}
