//! Copilot's pinned model-family routing and strictly allow-listed token endpoint hints.
//! No environment flags or discovery requests are performed implicitly.
use crate::{auth::Session, client::Provider, wire::Protocol};
use adk_core::{
    BoxFuture, Context, Error, Model, ModelRequest, ModelResponse, ModelStream, StreamingModel,
};
use std::sync::Arc;

/// Only the three API hosts recognized by the reference can be derived from a
/// token. Paths, ports, userinfo, query strings, fragments and lookalikes fail.
pub fn endpoint_hint(token: &str) -> Option<String> {
    if token.len() > 1024 * 1024 {
        return None;
    }
    for part in token.trim().split(';') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("proxy-ep") {
            continue;
        }
        let value = value.trim();
        let host = value
            .strip_prefix("https://")
            .unwrap_or(value)
            .to_ascii_lowercase();
        let host = host
            .strip_prefix("proxy.")
            .map(|tail| format!("api.{tail}"))
            .unwrap_or(host);
        return matches!(
            host.as_str(),
            "api.individual.githubcopilot.com"
                | "api.business.githubcopilot.com"
                | "api.enterprise.githubcopilot.com"
        )
        .then(|| format!("https://{host}"));
    }
    None
}
pub(crate) fn request_endpoint(configured: &str, token: &str) -> String {
    if matches!(
        configured,
        "https://api.githubcopilot.com" | "https://api.individual.githubcopilot.com"
    ) {
        endpoint_hint(token).unwrap_or_else(|| configured.to_owned())
    } else {
        configured.to_owned()
    }
}
/// Baseline does not fetch /models on the hot path. A host with discovery metadata
/// can select an explicit protocol in RouteSpec instead of using this heuristic.
pub fn protocol(model: &str) -> Protocol {
    let model = model.trim().to_ascii_lowercase();
    if model.starts_with("claude-") {
        Protocol::Anthropic
    } else if model.contains("gpt-5") || model.contains("codex") {
        Protocol::Responses
    } else {
        Protocol::Chat
    }
}
fn normalize(model: &str) -> &str {
    let model = model.trim();
    match model.split_once('/') {
        Some((prefix, bare)) if prefix.trim().eq_ignore_ascii_case("copilot") => bare.trim(),
        _ => model,
    }
}
pub struct Copilot {
    name: String,
    chat: Provider,
    responses: Provider,
    messages: Provider,
}
impl Copilot {
    pub fn new(name: impl Into<String>, session: Arc<Session>) -> Result<Self, Error> {
        if session.scope().mode != crate::auth::AuthMode::CopilotOAuth {
            return Err(crate::invalid(
                "Copilot requires a scoped bearer-token session",
            ));
        }
        let name = name.into();
        Ok(Self {
            chat: Provider::new(name.clone(), Protocol::Chat, session.clone())?,
            responses: Provider::new(name.clone(), Protocol::Responses, session.clone())?,
            messages: Provider::new(name.clone(), Protocol::Anthropic, session)?,
            name,
        })
    }
    fn select(&self, model: &str) -> &Provider {
        match protocol(model) {
            Protocol::Chat => &self.chat,
            Protocol::Responses => &self.responses,
            Protocol::Anthropic => &self.messages,
        }
    }
}
impl Model for Copilot {
    fn provider(&self) -> &str {
        &self.name
    }
    fn retry_advice(&self, error: &Error) -> Option<adk_core::ModelRetryAdvice> {
        crate::error::retry_advice(error)
    }
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        mut request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        request.model = normalize(&request.model).to_owned();
        self.select(&request.model).complete(context, request)
    }
}
impl StreamingModel for Copilot {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        mut request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        request.model = normalize(&request.model).to_owned();
        self.select(&request.model).stream(context, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_hints_are_exact_hosts_and_never_override_custom_endpoints() {
        for tier in ["individual", "business", "enterprise"] {
            let host = format!("api.{tier}.githubcopilot.com");
            for prefix in ["", "https://"] {
                assert_eq!(
                    endpoint_hint(&format!(
                        "tid=1; proxy-ep={prefix}proxy.{tier}.githubcopilot.com;"
                    )),
                    Some(format!("https://{host}"))
                );
            }
        }
        for value in [
            "http://proxy.individual.githubcopilot.com",
            "api.individual.githubcopilot.com:443",
            "api.individual.githubcopilot.com/",
            "api.individual.githubcopilot.com/x",
            "api.individual.githubcopilot.com?x=1",
            "api.individual.githubcopilot.com#x",
            "user@api.individual.githubcopilot.com",
            "api.individual.githubcopilot.com.evil.test",
            "evil.test",
        ] {
            assert_eq!(endpoint_hint(&format!("proxy-ep={value}")), None);
        }
        assert_eq!(
            request_endpoint(
                "https://custom.test",
                "proxy-ep=proxy.business.githubcopilot.com"
            ),
            "https://custom.test"
        );
        assert_eq!(
            request_endpoint(
                "https://api.individual.githubcopilot.com",
                "proxy-ep=proxy.business.githubcopilot.com"
            ),
            "https://api.business.githubcopilot.com"
        );
    }
}
