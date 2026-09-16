//! Immutable named routing. Registry construction is host-owned and credential-free.
use adk_core::{
    BoxFuture, Context, Error, Model, ModelRequest, ModelResponse, ModelStream, StreamingModel,
};
use std::{collections::BTreeMap, sync::Arc};

/// First slash selects the route; the rest is an opaque provider model ID.
/// Thus `openrouter/anthropic/claude` preserves `anthropic/claude` on the wire.
#[derive(Default)]
pub struct Routes {
    default: String,
    models: BTreeMap<String, Arc<dyn StreamingModel>>,
}
impl Routes {
    pub fn new(default: impl Into<String>) -> Self {
        Self {
            default: default.into(),
            models: BTreeMap::new(),
        }
    }
    /// Later registration deliberately replaces a canonical or named route.
    pub fn register(&mut self, prefix: &str, model: Arc<dyn StreamingModel>) -> Result<(), Error> {
        let prefix = prefix.trim().to_lowercase();
        if prefix.is_empty() || prefix.contains('/') {
            return Err(crate::invalid(
                "route prefix must be a nonempty single segment",
            ));
        }
        self.models.insert(prefix, model);
        Ok(())
    }
    pub fn resolve(&self, name: &str) -> Result<(Arc<dyn StreamingModel>, String), Error> {
        let name = name.trim();
        let (prefix, model) = name.split_once('/').unwrap_or((&self.default, name));
        if model.is_empty() {
            return Err(crate::invalid("model name is empty"));
        }
        // Never put the caller's model string into diagnostics: it is untrusted.
        let route = self
            .models
            .get(prefix)
            .ok_or_else(|| crate::invalid("unknown model provider prefix"))?;
        Ok((Arc::clone(route), model.to_owned()))
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
            let name = request.model.trim();
            let (prefix, model) = name.split_once('/').unwrap_or((&self.default, name));
            if model.is_empty() {
                return Err(crate::invalid("model name is empty"));
            }
            let route = self
                .models
                .get(prefix)
                .ok_or_else(|| crate::invalid("unknown model provider prefix"))?;
            request.model = model.to_owned();
            route.stream(context, request).await
        })
    }
}
