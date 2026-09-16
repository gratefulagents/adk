//! Immutable named routing. Registry construction is host-owned and credential-free.
use crate::{
    auth::{CredentialStore, Refresh},
    factory::{Kind, RouteSpec},
};
use adk_core::{
    BoxFuture, Context, Error, Model, ModelRequest, ModelResponse, ModelStream, StreamingModel,
};
use std::{collections::BTreeMap, sync::Arc};

struct Route {
    model: Arc<dyn StreamingModel>,
    kind: Option<Kind>,
    protocol: Option<crate::wire::Protocol>,
}

/// First slash selects the route; the rest is an opaque provider model ID.
/// Thus `openrouter/anthropic/claude` preserves `anthropic/claude` on the wire.
#[derive(Default)]
pub struct Routes {
    default: String,
    models: BTreeMap<String, Route>,
}
impl Routes {
    pub fn new(default: impl Into<String>) -> Self {
        Self {
            default: default.into().trim().to_ascii_lowercase(),
            models: BTreeMap::new(),
        }
    }
    /// Later registration deliberately replaces a canonical or named route.
    pub fn register(&mut self, prefix: &str, model: Arc<dyn StreamingModel>) -> Result<(), Error> {
        self.insert(prefix, model, None, None)
    }
    /// Register an injected model with explicit pricing and selection semantics.
    pub fn register_kind(
        &mut self,
        prefix: &str,
        kind: Kind,
        model: Arc<dyn StreamingModel>,
    ) -> Result<(), Error> {
        self.insert(prefix, model, Some(kind), None)
    }
    pub fn register_spec(
        &mut self,
        spec: &RouteSpec,
        store: Arc<dyn CredentialStore>,
        refresh: Arc<dyn Refresh>,
    ) -> Result<(), Error> {
        self.insert(
            &spec.prefix(),
            spec.build(store, refresh)?,
            Some(spec.kind),
            spec.protocol,
        )
    }
    fn insert(
        &mut self,
        prefix: &str,
        model: Arc<dyn StreamingModel>,
        kind: Option<Kind>,
        protocol: Option<crate::wire::Protocol>,
    ) -> Result<(), Error> {
        let prefix = prefix.trim().to_lowercase();
        if prefix.is_empty() || prefix.contains('/') {
            return Err(crate::invalid(
                "route prefix must be a nonempty single segment",
            ));
        }
        self.models.insert(
            prefix,
            Route {
                model,
                kind,
                protocol,
            },
        );
        Ok(())
    }
    pub fn resolve(&self, name: &str) -> Result<(Arc<dyn StreamingModel>, String), Error> {
        let (route, model) = self.select(name)?;
        Ok((Arc::clone(&route.model), model))
    }
    fn select(&self, name: &str) -> Result<(&Route, String), Error> {
        let name = name.trim();
        let (prefix, model) = name.split_once('/').unwrap_or((&self.default, name));
        // Never put the caller's model string into diagnostics: it is untrusted.
        let route = self
            .models
            .get(prefix)
            .ok_or_else(|| crate::invalid("unknown model provider prefix"))?;
        let model = route
            .kind
            .map_or_else(|| model.to_owned(), |kind| kind.resolve_model(model));
        if model.is_empty() {
            return Err(crate::invalid("model name is empty"));
        }
        Ok((route, model))
    }
    /// Unknown route/model prices stay unknown. Named routes use their base kind,
    /// not their arbitrary prefix, and Copilot follows the selected wire protocol.
    pub fn estimate_cost(&self, name: &str, usage: &adk_core::Usage) -> Option<f64> {
        let (route, model) = self.select(name).ok()?;
        let kind = route.kind?;
        let anthropic = kind == Kind::Anthropic
            || (kind == Kind::Copilot
                && route
                    .protocol
                    .unwrap_or_else(|| crate::copilot::protocol(&model))
                    == crate::wire::Protocol::Anthropic);
        if anthropic {
            Some(crate::cost::anthropic(&model, usage))
        } else {
            crate::cost::openai(&model, usage)
        }
    }
    pub fn normalize_model_name(&self, name: &str) -> String {
        let trimmed = name.trim();
        match trimmed.split_once('/') {
            Some((prefix, model)) if !model.is_empty() && self.models.contains_key(prefix) => {
                model.to_owned()
            }
            _ => name.to_owned(),
        }
    }
}
impl Model for Routes {
    fn provider(&self) -> &str {
        "multi"
    }
    fn retry_advice(&self, error: &Error) -> Option<adk_core::ModelRetryAdvice> {
        crate::error::retry_advice(error)
    }
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        mut request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            context.check_active()?;
            let (route, name) = self.resolve(&request.model)?;
            request.model = name;
            route.complete(context, request).await
        })
    }
}
// The resolved Arc must outlive a stream which may borrow the selected provider.
// Keep route lookup on self instead of manufacturing a self-referential owner.
impl StreamingModel for Routes {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        mut request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            context.check_active()?;
            let (route, model) = self.select(&request.model)?;
            request.model = model;
            route.model.stream(context, request).await
        })
    }
}
