//! Opt-in runner integration. The host selects pricing and compaction policy.
use crate::client::Provider;
use adk_core::{BoxFuture, Context, Error, ErrorCategory, ModelRequest, StreamingModel};
use adk_runtime::{CompactedHistory, CompactionRequest, Compactor, CostEstimator};
use std::sync::Arc;

/// Baseline USD accounting for the runner's actual selected binding, including
/// named routes and fallbacks. The runner requires a number: unknown prices use
/// zero, as in baseline CalculateCost. Use Routes::estimate_cost to retain the
/// known/unknown distinction; this adapter cannot enforce budgets for unknowns.
pub struct BaselineCosts(pub Arc<crate::routing::Routes>);
impl CostEstimator for BaselineCosts {
    fn cost(&self, model: &str, usage: &adk_core::Usage) -> f64 {
        self.0.estimate_cost(model, usage).unwrap_or(0.0)
    }
}

pub struct NativeCompactor {
    /// The same registry used by the runner's model bindings.
    pub routes: Arc<crate::routing::Routes>,
    /// Must be registered in `routes` as this same Arc, not a separate instance.
    pub provider: Arc<Provider>,
    /// Instructions, tools and settings used for compaction. Model and input
    /// are supplied by the runner, not taken from this template.
    pub template: ModelRequest,
    pub costs: Arc<dyn CostEstimator>,
}
impl Compactor for NativeCompactor {
    fn compact<'a>(
        &'a self,
        context: &'a Context,
        request: CompactionRequest,
    ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
        Box::pin(async move {
            let (provider, model) = self.routes.resolve(&request.model)?;
            let expected: Arc<dyn StreamingModel> = self.provider.clone();
            if !Arc::ptr_eq(&provider, &expected) {
                return Err(Error::new(
                    ErrorCategory::Unsupported,
                    "selected route does not match native compaction provider",
                ));
            }
            let mut input = self.template.clone();
            input.model = model;
            input.input = request.history;
            let response = self.provider.compact(context, input).await?;
            let cost = self.costs.cost(&request.model, &response.usage);
            if !cost.is_finite() || cost < 0.0 {
                return Err(Error::new(
                    ErrorCategory::InvalidInput,
                    "invalid compaction cost estimate",
                ));
            }
            Ok(CompactedHistory {
                context_tokens: adk_runtime::compaction::estimate_history_tokens(&response.items),
                history: response.items,
                usage: response.usage,
                cost,
            })
        })
    }
}
