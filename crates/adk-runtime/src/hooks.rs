use crate::{Observation, RunHooks};
use adk_core::{BoxFuture, Context, Error};
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct CompositeHooks {
    hooks: Vec<Arc<dyn RunHooks>>,
}
impl CompositeHooks {
    pub fn new(hooks: impl IntoIterator<Item = Arc<dyn RunHooks>>) -> Self {
        Self {
            hooks: hooks.into_iter().collect(),
        }
    }
    pub fn hooks(&self) -> &[Arc<dyn RunHooks>] {
        &self.hooks
    }
}

#[derive(Debug)]
pub struct HookErrors {
    pub errors: Vec<Error>,
}
impl std::fmt::Display for HookErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} run hooks failed", self.errors.len())
    }
}
impl std::error::Error for HookErrors {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.errors
            .first()
            .map(|error| error as &dyn std::error::Error)
    }
}
impl RunHooks for CompositeHooks {
    fn durable_observer(&self) -> bool {
        self.hooks.iter().all(|hook| hook.durable_observer())
    }
    fn observe<'a>(
        &'a self,
        context: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let mut errors = Vec::new();
            // An earlier failed observer must not prevent later sinks from seeing raw output.
            for hook in &self.hooks {
                if let Err(error) = hook.observe(context, observation.clone()).await {
                    errors.push(error);
                }
            }
            match errors.first() {
                None => Ok(()),
                Some(first) => Err(
                    Error::new(first.info.category, "one or more run hooks failed")
                        .with_source(HookErrors { errors }),
                ),
            }
        })
    }
}
