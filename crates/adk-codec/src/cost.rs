//! Lossless SDK display-cost projection, separate from native accounting policy.

use crate::dto::{ModelResponse, Usage};

/// Borrowed synchronous display-price estimator returning amount and certainty.
pub type CostEstimator<'a> = dyn Fn(&Usage) -> (f64, bool) + 'a;

/// Project an optional result's usage and responses using pinned SDK arithmetic.
///
/// The fallback returns `(amount, known)`; legacy cost-only estimators return
/// `(amount, true)`. An absent result or fallback corresponds to Go nil. Signed
/// counters wrap before residual clamping. Prices, including negative and
/// nonfinite values, are not sanitized; this is not a billing validation API.
pub fn estimate_result_cost(
    result: Option<(&Usage, &[ModelResponse])>,
    fallback: Option<&CostEstimator<'_>>,
) -> (f64, bool) {
    let Some((usage, responses)) = result else {
        return (0.0, false);
    };
    if responses.is_empty() {
        return fallback.map_or((0.0, false), |estimate| estimate(usage));
    }
    let mut cost = 0.0;
    let mut known = true;
    let mut used = Usage::default();
    for response in responses {
        cost += response.cost_usd;
        known &= response.cost_known;
        used.add(&response.usage);
    }
    let residual = Usage {
        requests: usage.requests.wrapping_sub(used.requests).max(0),
        input_tokens: usage.input_tokens.wrapping_sub(used.input_tokens).max(0),
        output_tokens: usage.output_tokens.wrapping_sub(used.output_tokens).max(0),
        cache_read_tokens: usage
            .cache_read_tokens
            .wrapping_sub(used.cache_read_tokens)
            .max(0),
        cache_create_tokens: usage
            .cache_create_tokens
            .wrapping_sub(used.cache_create_tokens)
            .max(0),
    };
    if residual.total_tokens() > 0
        || residual.cache_read_tokens > 0
        || residual.cache_create_tokens > 0
    {
        let (amount, certain) = fallback.map_or((0.0, false), |estimate| estimate(&residual));
        cost += amount;
        known &= certain;
    }
    (cost, known)
}
