//! Typed baseline provider selection. Credentials never inherit across routes.
use crate::{
    auth::{AuthMode, CredentialStore, Refresh, Scope, Session},
    client::Provider,
    wire::Protocol,
};
use adk_core::Error;
use std::sync::Arc;

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
            self.prefix.as_deref().unwrap_or(self.kind.name()),
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
