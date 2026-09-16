//! Typed baseline provider selection. Credentials never inherit across routes.
use crate::{
    auth::{AuthMode, CredentialStore, Refresh, Scope, Session},
    client::Provider,
    wire::Protocol,
};
use adk_core::Error;
use std::sync::Arc;

pub const DEFAULT_CHAT_MODEL: &str = "gpt-5.6-sol";
pub const DEFAULT_CHAT_MINI_MODEL: &str = "gpt-5.3-codex-spark";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    OpenAi,
    Anthropic,
    OpenRouter,
    Gemini,
    Groq,
    Xai,
    Local,
    Copilot,
}
impl Kind {
    /// Resolve a provider-local selection, including the baseline size aliases.
    /// Gateway IDs containing slashes remain opaque.
    pub fn resolve_model(self, model: &str) -> String {
        let model = model.trim();
        let resolved = match (self, model.to_ascii_lowercase().as_str()) {
            (Self::Anthropic, "" | "medium") => "claude-sonnet-4-6",
            (Self::Anthropic, "small") => "claude-haiku-4-5",
            (Self::Anthropic, "large") => "claude-opus-4-6",
            (_, "") => DEFAULT_CHAT_MODEL,
            (_, "small") => "gpt-5.6-luna",
            (_, "medium") => "gpt-5.6-terra",
            (_, "large") => DEFAULT_CHAT_MODEL,
            _ => model,
        };
        resolved.to_owned()
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::OpenRouter => "openrouter",
            Self::Gemini => "gemini",
            Self::Groq => "groq",
            Self::Xai => "xai",
            Self::Local => "local",
            Self::Copilot => "copilot",
        }
    }
    pub fn endpoint(self, mode: AuthMode) -> &'static str {
        match self {
            Self::OpenAi if mode == AuthMode::OpenAiOAuth => {
                "https://chatgpt.com/backend-api/codex"
            }
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/openai",
            Self::Groq => "https://api.groq.com/openai/v1",
            Self::Xai => "https://api.x.ai/v1",
            Self::Local => "http://localhost:11434/v1",
            Self::Copilot => "https://api.individual.githubcopilot.com",
        }
    }
    pub fn protocol(self) -> Protocol {
        match self {
            Self::OpenAi | Self::Xai => Protocol::Responses,
            Self::Anthropic => Protocol::Anthropic,
            _ => Protocol::Chat,
        }
    }
}
impl std::str::FromStr for Kind {
    type Err = Error;
    fn from_str(name: &str) -> Result<Self, Error> {
        match name.trim().to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "openrouter" => Ok(Self::OpenRouter),
            "gemini" => Ok(Self::Gemini),
            "groq" => Ok(Self::Groq),
            "xai" => Ok(Self::Xai),
            "local" => Ok(Self::Local),
            "copilot" => Ok(Self::Copilot),
            _ => Err(crate::invalid("unknown provider kind")),
        }
    }
}

/// Explicit default, model prefix, configured single provider, then OpenAI.
/// Inference selects a route only; it never moves credentials between scopes.
pub fn default_route(explicit: Option<&str>, model: &str, provider: Option<Kind>) -> String {
    explicit
        .filter(|value| !value.trim().is_empty())
        .or_else(|| model.trim().split_once('/').map(|(prefix, _)| prefix))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(provider.unwrap_or(Kind::OpenAi).name())
        .trim()
        .to_ascii_lowercase()
}

pub fn supports_chat_completions(model: &str) -> bool {
    !model.to_ascii_lowercase().contains("codex")
}

/// Each named instance supplies its own host-owned store and refresh boundary.
/// Omitted endpoint/protocol use canonical defaults, not another instance's values.
pub struct RouteSpec {
    pub kind: Kind,
    pub prefix: Option<String>,
    pub endpoint: Option<String>,
    pub protocol: Option<Protocol>,
    pub mode: AuthMode,
    pub account: Option<String>,
}
impl RouteSpec {
    pub fn new(kind: Kind, mode: AuthMode) -> Self {
        Self {
            kind,
            mode,
            prefix: None,
            endpoint: None,
            protocol: None,
            account: None,
        }
    }
    pub fn prefix(&self) -> String {
        self.prefix
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or(self.kind.name())
            .trim()
            .to_ascii_lowercase()
    }
    pub fn scope(&self) -> Result<Scope, Error> {
        let valid = match self.mode {
            AuthMode::ApiKey => self.kind != Kind::Copilot,
            AuthMode::Anonymous => self.kind == Kind::Local,
            AuthMode::OpenAiOAuth => self.kind == Kind::OpenAi,
            AuthMode::AnthropicOAuth => self.kind == Kind::Anthropic,
            AuthMode::CopilotOAuth => self.kind == Kind::Copilot,
        };
        if !valid {
            return Err(crate::invalid(
                "authentication mode does not match provider kind",
            ));
        }
        Scope::new(
            &self.prefix(),
            self.endpoint
                .as_deref()
                .unwrap_or(self.kind.endpoint(self.mode)),
            self.account.clone(),
            self.mode,
        )
    }
    pub fn build(
        &self,
        store: Arc<dyn CredentialStore>,
        refresh: Arc<dyn Refresh>,
    ) -> Result<Arc<dyn adk_core::StreamingModel>, Error> {
        let scope = self.scope()?;
        let protocol = self.protocol.unwrap_or(self.kind.protocol());
        if (self.kind == Kind::Anthropic && protocol != Protocol::Anthropic)
            || (protocol == Protocol::Anthropic
                && !matches!(self.kind, Kind::Anthropic | Kind::Copilot))
        {
            return Err(crate::invalid("protocol does not match provider kind"));
        }
        let name = scope.route.clone();
        let session = Arc::new(Session::new(scope, store, refresh)?);
        if self.kind == Kind::Copilot && self.protocol.is_none() {
            Ok(Arc::new(crate::copilot::Copilot::new(name, session)?))
        } else {
            Ok(Arc::new(Provider::new(name, protocol, session)?))
        }
    }
}
